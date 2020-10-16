//! Lang items

use crate::halt;
use crate::vga::{Colour, ColourPair, VgaWriter};
use core::alloc::Layout;
use core::fmt::Write;
use core::panic::PanicInfo;
use uart_16550::SerialPort;

// A note on the `#[no_mangle]`s:
// Apparently, removing them makes it link-error with undefined symbols, so we include them

#[lang = "eh_personality"]
#[no_mangle]
unsafe extern "C" fn eh_personality() {}

#[panic_handler]
#[no_mangle]
// TODO backtrace
extern "C" fn panic_fmt(info: &PanicInfo) -> ! {
    let mut vga_writer = unsafe { VgaWriter::new() };
    let mut serial = unsafe { SerialPort::new(0x3f8) };

    // Ignore the errors because we can't afford to panic in the panic handler
    let _ = vga_writer.colour = ColourPair::new(Colour::Red, Colour::Black);

    let arguments = match info.message() {
        Some(args) => *args,
        None => format_args!("undefined"),
    };

    if let Some(loc) = info.location() {
        let _ = write!(
            &mut vga_writer,
            "Panicked at \"{}\", {file}:{line}",
            arguments,
            file = loc.file(),
            line = loc.line()
        );

        let _ = write!(
            &mut serial,
            "Panicked at \"{}\", {file}:{line}",
            arguments,
            file = loc.file(),
            line = loc.line()
        );
    } else {
        let _ = write!(
            &mut vga_writer,
            "Panicked at \"{}\" at an undefined location",
            arguments
        );
        let _ = write!(
            &mut serial,
            "Panicked at \"{}\" at an undefined location",
            arguments
        );
    }

    // TODO(userspace) this overwrites panic messages with GPF
    unsafe { halt() }
}

#[alloc_error_handler]
fn oom(_: Layout) -> ! {
    panic!("Ran out of kernel heap memory")
}
