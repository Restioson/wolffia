#![feature(
    asm,
    naked_functions,
    allocator_api,
    alloc_error_handler,
    lang_items,
    panic_info_message,
    abi_x86_interrupt,
    const_mut_refs,
    step_trait,
    step_trait_ext,
    never_type
)]
#![no_std]

#[macro_use]
extern crate alloc;

use crate::memory::heap::Heap;
use crate::process::Process;
use crate::vga::VGA_WRITER;
use core::fmt;
use core::fmt::Write;
use spin::Mutex;
use uart_16550::SerialPort;
use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};

mod lang;
#[macro_use]
mod vga;
#[macro_use]
mod log;
#[macro_use]
mod util;
mod acpi_handler;
mod gdt;
mod interrupts;
mod memory;
mod pit;
pub mod process;
mod syscall;
mod tss;

#[global_allocator]
pub static HEAP: Heap = Heap::new();
pub static SERIAL_WRITER: Mutex<SerialPort> = Mutex::new(unsafe { SerialPort::new(0x3f8) });
static INIT_ELF: &[u8] = include_bytes!(env!("WOLFFIA_INIT_PATH"));

/// Writes formatted string to serial 1, for print macro use
pub fn serial1_print(args: fmt::Arguments) {
    SERIAL_WRITER.lock().write_fmt(args).unwrap()
}

#[no_mangle]
pub extern "C" fn kmain(mb_info_addr: u64, guard_page_addr: u64) -> ! {
    VGA_WRITER.lock().clear();
    log::init();
    memory::init_memory(mb_info_addr, guard_page_addr);
    gdt::init();

    interrupts::init();
    interrupts::enable();
    info!("interrupts: ready");

    enable_features();
    info!("cpu features: enabled");

    pit::CONTROLLER.lock().initialize();
    info!("pit: ready");

    let _acpi = acpi_handler::acpi_init();
    unsafe { syscall::setup_syscall() };

    info!("init: loading");
    let pid = Process::spawn_from_elf(INIT_ELF)
        .map_err(|e| panic!("{:#x?}", e))
        .unwrap();
    info!("init: launching");

    Process::run_by_pid(&pid).expect("Out of physical memory")
}

fn enable_features() {
    unsafe {
        Cr0::update(|flags| {
            flags.remove(Cr0Flags::EMULATE_COPROCESSOR);
            *flags |= Cr0Flags::MONITOR_COPROCESSOR;
        });

        Cr4::update(|flags| {
            *flags |= Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE;
        });
    }
}

fn halt() -> ! {
    // Disable interrupts
    unsafe {
        asm!("cli");
    }

    // Halt forever...
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}
