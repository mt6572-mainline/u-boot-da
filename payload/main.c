#define u8 unsigned char
#define u16 unsigned short
#define u32 unsigned int

#define CACHE_LINE 32
#define PRELOADER_BASE 0x2007500
#define PRELOADER_END 0x2107500
#define USBDL_HANDLER_END (usbdl_handler_addr + 0x200)

#define SEARCH_PATTERN(start_addr, end_addr, ...)                              \
  ({                                                                           \
    static u16 pattern[] = {__VA_ARGS__};                                      \
    const u32 pattern_count = sizeof(pattern) / sizeof(pattern[0]);            \
    u32 result = 0;                                                            \
                                                                               \
    u32 max_addr = end_addr - (pattern_count * 2);                             \
    for (u32 offset = start_addr; offset < max_addr; offset += 2) {            \
      u16 first_val = *(volatile u16 *)offset;                                 \
      if (first_val != pattern[0])                                             \
        continue;                                                              \
                                                                               \
      u32 i;                                                                   \
      for (i = 1; i < pattern_count; i++) {                                    \
        u32 check_addr = offset + (i * 2);                                     \
        u16 value = *(volatile u16 *)check_addr;                               \
                                                                               \
        if (value != pattern[i])                                               \
          break;                                                               \
      }                                                                        \
                                                                               \
      if (i == pattern_count) {                                                \
        result = offset;                                                       \
        break;                                                                 \
      }                                                                        \
    }                                                                          \
                                                                               \
    result;                                                                    \
  })

#define PATCH_MEM(addr, ...)                                                   \
  do {                                                                         \
    const u16 patch_data[] = {__VA_ARGS__};                                    \
    volatile u16 *p = (volatile u16 *)(addr);                                  \
    for (u32 i = 0; i < sizeof(patch_data) / sizeof(patch_data[0]); i++) {     \
      p[i] = patch_data[i];                                                    \
    }                                                                          \
    arch_clean_invalidate_cache_range((u32)(addr), sizeof(patch_data));        \
  } while (0)

#define FORCE_RETURN(addr, value)                                              \
  do {                                                                         \
    PATCH_MEM(addr, 0x2000 | ((value) & 0xFF), 0x4770);                        \
  } while (0)

#define NOP(addr, count)                                                       \
  do {                                                                         \
    for (int i = 0; i < (count); i++) {                                        \
      PATCH_MEM((addr) + (i * 2), 0xBF00);                                     \
    }                                                                          \
  } while (0)

#define STATUS(desc, code)                                                     \
  do {                                                                         \
    uart_print(desc " is ");                                                   \
    code;                                                                      \
    if (!addr)                                                                 \
      uart_print("NOT ");                                                      \
    uart_println("patched");                                                   \
    addr = 0;                                                                  \
  } while (0)

void uart_putc(char c) {
  volatile u32 *uart0_thr = (volatile u32 *)0x11005000;
  volatile u32 *uart0_lsr = (volatile u32 *)0x11005014;

  while (!((*uart0_lsr) & 0x20))
    ;

  *uart0_thr = c;
}

void _putchar(char c) {
  if (c == '\n')
    uart_putc('\r');

  uart_putc(c);
}

void uart_print(const char *s) {
  while (*s) {
    uart_putc(*s);
    s++;
  }
}

void uart_println(const char *s) {
  uart_print(s);
  uart_putc('\r');
  uart_putc('\n');
}

void arch_clean_invalidate_cache_range(u32 start, u32 size) {
  u32 end = start + size;
  start &= ~(CACHE_LINE - 1);

  while (start < end) {
    __asm__ volatile("mcr p15, 0, %0, c7, c14, 1\n"
                     "add %0, %0, %[clsize]\n"
                     : "+r"(start)
                     : [clsize] "I"(CACHE_LINE)
                     : "memory");
  }

  __asm__ volatile("mcr p15, 0, %0, c7, c10, 4\n" ::"r"(0) : "memory");
}

u8 is_movs_thumb2(u16 instr) { return (instr & 0xf800) == 0x2000; }
u8 is_str_sp_rel_thumb2(u16 instr) {
  return (instr & 0xF800) == 0x9000;
}
void flip_str_to_ldr(u16 *instr) {
      *instr |= (1 << 11);
}
u32 extract_ldr_offset(u16 instr) { return (instr & 0xff) * 4; }

__attribute__((section(".text.start"))) int main() {
  u32 addr, usbdl_handler_addr;
  void (*usbdl_handler)(u32, u32);

  uart_println("");

  usbdl_handler_addr =
      SEARCH_PATTERN(PRELOADER_BASE, PRELOADER_END, 0xe92d, 0x4ef0, 0x460e);
  if (!usbdl_handler_addr) {
    uart_println("usbdl_handler not found :(");
    while (1)
      ;
  }

  usbdl_handler = (void *)(usbdl_handler_addr | 1);

  STATUS("send_da", {
    addr = SEARCH_PATTERN(usbdl_handler_addr, USBDL_HANDLER_END,
                          0x4603); // mov r3, r0
    if (addr) {
      addr -= 8; // skip 32 bit instructions to be safe
      do {
        addr -= 2;
      } while (!is_str_sp_rel_thumb2(*(u16 *)addr));

      flip_str_to_ldr((u16 *)addr);
    }
  });

  STATUS("jump_da", {
    addr = SEARCH_PATTERN(PRELOADER_BASE, PRELOADER_END, 0x2600, 0x4630);
    if (addr) {
      addr += 40; // ldr

      // some preloaders may overwrite the payload with DA boot argument
      if (is_movs_thumb2(*(u16 *)(addr + 6))) {
        NOP(addr + 2, 13);
      } else {
        NOP(addr + 2, 7);
      }

      addr += extract_ldr_offset(*(u16 *)addr) + 2;
      *(u32 *)(addr) = 0x800d0000;
    }
  });

  STATUS("sec_region_check", {
    addr = SEARCH_PATTERN(usbdl_handler_addr, PRELOADER_END, 0xb537, 0x4604,
                          0x460d);
    if (addr)
      FORCE_RETURN(addr, 0); // keep for debugging purposes (read32)
  });

  uart_println("Jumping back to usbdl_handler...");
  asm volatile("dsb; isb");
  usbdl_handler(*(u32 *)0x2000828, 300);
}
