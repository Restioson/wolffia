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
    out_dir = $(build_containing_dir)/$(build_type)
    target_dir := target/$(target)/$(build_type)
    nasm_flags := -f elf64 -F dwarf -g
    qemu_flags := -s -m 256M -d int -no-reboot -no-shutdown -monitor stdio -serial file:$(out_dir)/serial.log
    cargo_flags := --features $(log_level)
else
    build_type := release
    out_dir = $(build_containing_dir)/$(build_type)
	target_dir := target/$(target)/$(build_type)
    nasm_flags := -f elf64
    release_flags := --release
    cargo_flags := --release --features $(log_level)
 	rustflags := "-C code-model=kernel"
    qemu_flags := -m 256M -serial file:$(out_dir)/serial.log
endif

ifeq ($(wait_for_gdb), 1)
    qemu_flags := -s -S
endif

asm_dir := kernel/src/asm
rust_kernel := $(out_dir)/libwolffia_kernel.a
init_elf := $(out_dir)/init.elf
asm_source_files := $(wildcard $(asm_dir)/*.asm)
asm_obj_files = $(patsubst $(asm_dir)/%.asm, $(out_dir)/%.o, $(asm_source_files))

kernel = $(out_dir)/kernel.elf
grub_iso = $(out_dir)/wolffia.iso

default: build

.PHONY: clean run build $(rust_kernel) iso test
$(grub_iso): $(kernel) kernel/grub.cfg
	@cp kernel/grub.cfg $(out_dir)/isofiles/boot/grub/
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

$(init_elf): makedirs
	@cd userspace/init && cargo +nightly build $(release_flags)
	@rm -f $(init_elf)
	@mv userspace/target/x86_64-unknown-wolffia/$(build_type)/init $(init_elf)

# Compile rust
$(rust_kernel): $(init_elf)
	@cd kernel && \
		RUSTFLAGS=$(rustflags) cargo +nightly build $(cargo_flags)
	@rm -f $(rust_kernel)
	@mv kernel/$(target_dir)/libwolffia_kernel.a $(rust_kernel)

# Compile kernel.elf
$(kernel): $(asm_obj_files) kernel/linker.ld $(rust_kernel)
	ld -n -T kernel/linker.ld -o $(kernel) $(asm_obj_files) $(rust_kernel) --gc-sections

# Compile asm files
$(out_dir)/%.o: $(asm_dir)/%.asm makedirs
	@nasm $(nasm_flags) $< -o $@
