CC := arm-none-eabi-gcc
AS := arm-none-eabi-as
LD := arm-none-eabi-gcc
OBJCOPY := arm-none-eabi-objcopy

CFLAGS := -std=gnu99 -Os -mcpu=cortex-a7  -fno-builtin-printf -fno-strict-aliasing -fno-builtin-memcpy -mno-unaligned-access -Wall -Wextra
LDFLAGS := -nodefaultlibs -nostdlib -lgcc

SRC = main.c
ASM_SRC = start.S

OBJ = $(SRC:%.c=%.o) $(ASM_SRC:%.S=%.o)
DEP = $(OBJ:%.o=%.d)

all: preloader_patcher.bin

preloader_patcher.bin: preloader_patcher.elf
	$(OBJCOPY) -O binary $^ $@

preloader_patcher.elf: $(OBJ)
	$(LD) -o $@ $^ $(LDFLAGS) -T payload.ld

-include $(DEP)

%.o: %.c
	$(CC) -MMD -c -o $@ $< $(CFLAGS)

%.o: %.S
	$(AS) -o $@ $<

clean:
	-rm -rf *.d *.o *.elf *.bin

