#![feature(asm, lang_items, allocator_api, alloc_error_handler, panic_info_message, abi_x86_interrupt, naked_functions)]
#![no_std]

#[macro_use]
extern crate alloc;

use spin::Mutex;
use uart_16550::SerialPort;
use core::fmt::Write;
use core::fmt;
use crate::memory::heap::Heap;
use crate::vga::VGA_WRITER;

mod lang;
#[macro_use]
mod vga;
#[macro_use]
mod log;
#[macro_use]
mod util;
mod memory;
mod gdt;
mod acpi_handler;
mod interrupts;
mod pit;
mod userspace;

#[global_allocator]
pub static HEAP: Heap = Heap::new();

#[allow(unused_macros)]
macro_rules! print {
    ($($arg:tt)*) => ({
        $crate::vga::stdout_print(format_args!($($arg)*));
        $crate::serial1_print(format_args!($($arg)*));
    });
}

#[allow(unused_macros)]
macro_rules! println {
    ($fmt:expr) => (print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => (print!(concat!($fmt, "\n"), $($arg)*));
}

pub static SERIAL_WRITER: Mutex<SerialPort> = Mutex::new(unsafe { SerialPort::new(0x3f8) });

/// Writes formatted string to serial 1, for print macro use
pub fn serial1_print(args: fmt::Arguments) {
    SERIAL_WRITER.lock().write_fmt(args).unwrap()
}

#[no_mangle]
pub extern fn kmain(mb_info_addr: u64, guard_page_addr: u64) -> ! {
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

    crate::userspace::usermode_begin()
}

fn halt() -> ! {
    unsafe {
        // Disable interrupts
        asm!("cli");

        // Halt forever...
        loop {
            asm!("hlt");
        }
    }
}
