#[allow(dead_code)]
#[repr(u64)]
pub enum Syscall {
    Halt = 0,
    Deadbeef = 1,
    Print = 2,
}

macro_rules! syscall_raw {
    ($($name:ident($($reg:tt = $val:ident),*)),*) => {
        $(#[allow(dead_code)] pub extern "C" fn $name(
            call: $crate::syscall::Syscall,
            $($val: u64),*
        ) -> i64 {
            let out: i64;
            unsafe {
                asm!("syscall",
                in("rax") call as u64,
                $(in($reg) $val,)*
                lateout("rax") out,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
                );
            };
            out
        })*
    };
}

pub mod raw {
    syscall_raw!(
        syscall_0(),
        syscall_1("rdi" = arg1),
        syscall_2("rdi" = arg1, "rsi" = arg2)
    );
}

pub fn print(string: &str) -> Result<(), ()> {
    let (ptr, len) = (string.as_ptr(), string.len());
    match raw::syscall_2(Syscall::Print, ptr as u64, len as u64) {
        -1 => Err(()),
        _ => Ok(()),
    }
}

pub fn halt() -> ! {
    raw::syscall_0(Syscall::Halt);
    unreachable!()
}

pub fn dead_beef() -> Result<i64, ()> {
    match raw::syscall_0(Syscall::Deadbeef) {
        -1 => Err(()),
        val => Ok(val),
    }
}
