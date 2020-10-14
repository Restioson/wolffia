use self::process::{Process, PROCESSES};
use core::fmt::Write;

pub const STACK_TOP: u64 = 0x7ffffffff000; // Top of lower half but page aligned
pub const INITIAL_STACK_SIZE_PAGES: usize = 16; // 64kib stack

mod jump;
pub mod process;

pub fn usermode_begin() -> ! {
    let pid = unsafe { Process::spawn(usermode as usize) };
    let mut process = PROCESSES.get_mut(&pid).unwrap();
    process.run()
}

pub extern "C" fn usermode() -> ! {
    info!("Jumped into userspace successfully!");
    info!("Intentional General Protection Fault incoming...");
    panic!("Nothing more to do.");
}
