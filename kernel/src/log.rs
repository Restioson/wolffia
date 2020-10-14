use crate::vga::VGA_WRITER;
use log::{self, Level, Log, Metadata, Record};

static LOGGER: Logger = Logger;

#[allow(unused_macros)]
macro_rules! error {
    ($thing:expr, $($extra:tt)*) => {
        {
            use alloc::fmt::Write;
            crate::vga::VGA_WRITER.lock().write_str_coloured("[error] ", colour!(Red on Black));
            write!(crate::vga::VGA_WRITER.lock(), "{}\n", format_args!($thing, $($extra)*)).unwrap();
            crate::serial1_print(format_args!("[error] "));
            crate::serial1_print(format_args!($thing, $($extra)*));
            crate::serial1_print(format_args!("\n"));
        }
    };

    ($thing:expr) => {
        error!($thing,)
    }
}

#[allow(unused_macros)]
macro_rules! warn {
    ($thing:expr, $($extra:tt)*) => {
        {
            use alloc::fmt::Write;
            crate::vga::VGA_WRITER.lock().write_str_coloured("[warn]  ", colour!(LightRed on Black));
            write!(crate::vga::VGA_WRITER.lock(), "{}\n", format_args!($thing, $($extra)*)).unwrap();
            crate::serial1_print(format_args!("[warn]  "));
            crate::serial1_print(format_args!($thing, $($extra)*));
            crate::serial1_print(format_args!("\n"));
        }
    };

    ($thing:expr) => {
        warn!($thing,)
    }
}

macro_rules! info {
    ($thing:expr, $($extra:tt)*) => {
        {
            use alloc::fmt::Write;
            crate::vga::VGA_WRITER.lock().write_str_coloured("[info]  ", colour!(LightBlue on Black));
            write!(crate::vga::VGA_WRITER.lock(), "{}\n", format_args!($thing, $($extra)*)).unwrap();
            crate::serial1_print(format_args!("[info]  "));
            crate::serial1_print(format_args!($thing, $($extra)*));
            crate::serial1_print(format_args!("\n"));
        }
    };

    ($thing:expr) => {
        info!($thing,)
    }
}

macro_rules! debug {
    ($thing:expr, $($extra:tt)*) => {
        #[cfg(feature = "debug")]
        {
            use alloc::fmt::Write;
            crate::vga::VGA_WRITER.lock().write_str_coloured("[debug] ", colour!(Cyan on Black));
            write!(crate::vga::VGA_WRITER.lock(), "{}\n", format_args!($thing, $($extra)*)).unwrap();
            crate::serial1_print(format_args!("[debug] "));
            crate::serial1_print(format_args!($thing, $($extra)*));
            crate::serial1_print(format_args!("\n"));
        }
    };

    ($thing:expr) => {
        debug!($thing,)
    }
}

macro_rules! trace {
    ($thing:expr, $($extra:tt)*) => {
        #[cfg(feature = "trace")]
        {
            use alloc::fmt::Write;
            crate::vga::VGA_WRITER.lock().write_str_coloured("[trace] ", colour!(White on Black));
            write!(crate::vga::VGA_WRITER.lock(), "{}\n", format_args!($thing, $($extra)*)).unwrap();
            crate::serial1_print(format_args!("[trace] "));
            crate::serial1_print(format_args!($thing, $($extra)*));
            crate::serial1_print(format_args!("\n"));
        }
    };

    ($thing:expr) => {
        trace!($thing,)
    }
}

struct Logger;

// `return` statements and `#[allow]` required here because of the `cfg`s and how log levels work
#[allow(unreachable_code)]
const fn log_level() -> Level {
    #[cfg(feature = "trace")]
    return Level::Trace;

    #[cfg(feature = "debug")]
    return Level::Debug;

    Level::Info
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log_level()
    }

    fn log(&self, record: &Record) {
        use core::fmt::Write;

        if self.enabled(record.metadata()) {
            let (label, colour) = match record.level() {
                Level::Trace => ("[trace] ", colour!(White on Black)),
                Level::Debug => ("[debug] ", colour!(Cyan on Black)),
                Level::Info => ("[info]  ", colour!(LightBlue on Black)),
                Level::Warn => ("[warn]  ", colour!(LightRed on Black)),
                Level::Error => ("[error] ", colour!(Red on Black)),
            };

            let message = format!("{}: {}\n", record.target(), record.args());

            crate::vga::VGA_WRITER
                .lock()
                .write_str_coloured(label, colour);
            VGA_WRITER.lock().write_str(&message);

            write!(crate::SERIAL_WRITER.lock(), "{}", label).unwrap();
            write!(crate::SERIAL_WRITER.lock(), "{}", message).unwrap();
        }
    }

    fn flush(&self) {}
}

pub fn init() {
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(log_level().to_level_filter()))
        .expect("Error setting logger!");
}
