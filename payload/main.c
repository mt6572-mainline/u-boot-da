#define u8 unsigned char
#define u16 unsigned short
#define u32 unsigned int

#define CACHE_LINE 32

#define PATCH_MEM(addr, ...)                                                   \
  do {                                                                         \
    const u16 patch_data[] = {__VA_ARGS__};                                    \
    volatile u16 *p = (volatile u16 *)(addr);                                  \
    for (u32 i = 0; i < sizeof(patch_data) / sizeof(patch_data[0]); i++) {  \
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

__attribute__((section(".text.start"))) int main() {
  void (*usbdl_handler)(u32, u32) = (void *)(0x02008710 | 1);

  uart_println("");

  uart_print("Patching hardcoded address in send_da...");
  NOP(0x020088C0, 1);
  PATCH_MEM(0x020088B4, 0x9807); // ldr r0, sp+0x1c
  uart_println("ok");

  uart_print("Patching hardcoded address in jump_da...");
  NOP(0x020089EE, 14); // kill memcpy too
  uart_println("ok");

  uart_print("Patching sec_region_check...");
  FORCE_RETURN(0x020150BC, 0); // keep for debugging purposes (read32)
  uart_println("ok");

  uart_println("Jumping back to usbdl_handler...");
  asm volatile("dsb; isb");
  usbdl_handler(*(u32 *)0x2000828, 300);
}
