build_containing_dir := build
debug ?= 0
target := x86_64-unknown-wolffiakernel-none

ifneq ($(debug), 1)
else ifndef log_level
    log_level := debug
endif

ifndef log_level
    log_level := ""
endif

ifeq ($(debug), 1)
    build_type := debug
    target_dir := target/$(target)/$(build_type)
    nasm_flags := -f elf64 -F dwarf -g
    qemu_flags := -s -m 256M -d int -no-reboot -no-shutdown -monitor stdio -serial file:$(target_dir)/serial.log
    cargo_flags := --features $(log_level)
else
    build_type := release
	target_dir := target/$(target)/$(build_type)
    nasm_flags := -f elf64
    cargo_flags := --release --features $(log_level)
 	rustflags := "-C code-model=kernel"
    qemu_flags := -m 256M -serial file:$(target_dir)/serial.log
endif

ifeq ($(wait_for_gdb), 1)
    qemu_flags := -s -S
endif

linker_script := linker.ld
grub_cfg := grub.cfg
out_dir = $(build_containing_dir)/$(build_type)
asm_dir := src/asm
rust_kernel := $(out_dir)/libwolffia_kernel.a
asm_source_files := $(wildcard $(asm_dir)/*.asm)
asm_obj_files = $(patsubst $(asm_dir)/%.asm, $(out_dir)/%.o, $(asm_source_files))

kernel = $(out_dir)/kernel.elf
grub_iso = $(out_dir)/wolffia.iso

default: build

.PHONY: clean run build $(rust_kernel) iso test
$(grub_iso): $(kernel) $(grub_cfg)
	@cp $(grub_cfg) $(out_dir)/isofiles/boot/grub/
	@cp $(kernel) $(out_dir)/isofiles/boot/
	@grub-mkrescue -o $(out_dir)/wolffia.iso $(out_dir)/isofiles

build: $(kernel)
iso: $(grub_iso)

# Run with qemu
run: $(grub_iso)
	@qemu-system-x86_64 -cdrom $(grub_iso) $(qemu_flags) -m 128M

# Clean build dir
clean:
	@rm -rf build
	@RUST_TARGET_PATH=$(shell pwd) cargo +nightly clean

# Make build directories
makedirs:
	@mkdir -p $(out_dir)
	@mkdir -p $(out_dir)/isofiles
	@mkdir -p $(out_dir)/isofiles/boot/grub

# Compile rust
$(rust_kernel):
	@ RUST_TARGET_PATH=$(shell pwd) RUSTFLAGS=$(rustflags) cargo +nightly build $(cargo_flags)
	@rm -f $(rust_kernel)
	@mv $(target_dir)/libwolffia_kernel.a $(rust_kernel)

# Compile kernel.elf
$(kernel): $(asm_obj_files) $(linker_script) $(rust_kernel)
	@ld -n -T $(linker_script) -o $(kernel) $(asm_obj_files) $(rust_kernel) --gc-sections

# Compile asm files
$(out_dir)/%.o: $(asm_dir)/%.asm makedirs
	@nasm $(nasm_flags) $< -o $@
