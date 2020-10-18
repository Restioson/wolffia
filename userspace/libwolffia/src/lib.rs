#![feature(asm, lang_items, panic_info_message)]
#![no_std]

pub mod syscall;

use core::panic::PanicInfo;
use core::fmt::{self, Write};

pub use libwolffia_macros::*;

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({
        use ::core::fmt::Write;
        write!(&mut $crate::Stdout, $($arg)*).unwrap();
    });
}

#[macro_export]
macro_rules! println {
    ($fmt:expr) => (print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => (print!(concat!($fmt, "\n"), $($arg)*));
}

pub mod prelude {
    pub use crate::{print, println};
}

#[lang = "eh_personality"]
fn eh_personality() {}

#[panic_handler]
// TODO(userspace heap): backtrace
fn panic_fmt(info: &PanicInfo) -> ! {
    let arguments = match info.message() {
        Some(args) => *args,
        None => format_args!("undefined"),
    };

    if let Some(loc) = info.location() {
        let _ = write!(
            &mut Stdout,
            "Panicked at \"{}\", {file}:{line}",
            arguments,
            file = loc.file(),
            line = loc.line()
        );
    } else {
        let _ = write!(
            &mut Stdout,
            "Panicked at \"{}\" at an undefined location",
            arguments
        );
    }

    syscall::halt()
}

pub struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        syscall::print(s).map_err(|_| fmt::Error)
    }
}
