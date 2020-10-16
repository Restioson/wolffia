use self::process::{Process, PROCESSES};
use crate::syscall::{syscall_raw, Syscall};

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

    info!("Asking for deadbeef...");
    let v = syscall_raw(Syscall::Deadbeef);
    info!("System call returned 0x{:x}", v);
    info!("Halting...");
    syscall_raw(Syscall::Halt);
    unreachable!()
}
