#![feature(
    asm,
    naked_functions,
    allocator_api,
    alloc_error_handler,
    lang_items,
    panic_info_message,
    abi_x86_interrupt,
    const_mut_refs
)]
#![no_std]

#[macro_use]
extern crate alloc;

use crate::memory::heap::Heap;
use crate::vga::VGA_WRITER;
use core::fmt;
use core::fmt::Write;
use spin::Mutex;
use uart_16550::SerialPort;

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
mod tss;
mod userspace;
mod syscall;

#[global_allocator]
pub static HEAP: Heap = Heap::new();

pub static SERIAL_WRITER: Mutex<SerialPort> = Mutex::new(unsafe { SerialPort::new(0x3f8) });

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

    pit::CONTROLLER.lock().initialize();
    info!("pit: ready");

    let _acpi = acpi_handler::acpi_init();
    unsafe { syscall::setup_syscall() };

    crate::userspace::usermode_begin()
}

/// # Safety
///
/// Must be inside of the kernel, else this will throw a GPF.
// TODO(userspace): unsafe can be removed when no userspace is in the kernel binary
unsafe fn halt() -> ! {
    // Disable interrupts
    asm!("cli");

    // Halt forever...
    loop {
        asm!("hlt");
    }
}
