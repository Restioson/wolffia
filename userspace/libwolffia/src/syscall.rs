#[allow(dead_code)]
#[repr(u64)]
pub enum Syscall {
    Halt = 0,
    Map = 1,
    Unmap = 2,
    Print = 3,
}

pub enum SyscallError {
    InvalidBuffer,
    InvalidUtf8,
    InvalidPage,
    InvalidPagesLength,
    OutOfMemory,
    UnknownError(i64),
}

bitflags::bitflags! {
     pub struct UserPageFlags: u64 {
        const WRITABLE = 1;
        const EXECUTABLE = 1 << 1;
     }
}

pub fn res_from_code(code: i64) -> Result<i64, SyscallError> {
    match code {
        x if x >= 0 => Ok(x),
        -1 => Err(SyscallError::InvalidBuffer),
        -2 => Err(SyscallError::InvalidUtf8),
        -3 => Err(SyscallError::InvalidPagesLength),
        -4 => Err(SyscallError::OutOfMemory),
        unknown => Err(SyscallError::UnknownError(unknown)),
    }
}

macro_rules! syscall_raw {
    ($($name:ident($($reg:tt = $val:ident),*)),*) => {
        $(paste::paste! {
            #[allow(dead_code)]
            extern "C" fn [<$name _raw>](
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
            }

            #[allow(dead_code)]
            pub fn $name(
                call: $crate::syscall::Syscall,
                $($val: u64),*
            ) -> ::core::result::Result<i64, $crate::syscall::SyscallError> {
                $crate::syscall::res_from_code([<$name _raw>](call, $($val),*))
            }
        })*
    }
}

pub mod raw {
    syscall_raw!(
        syscall_0(),
        syscall_1("rdi" = arg1),
        syscall_2("rdi" = arg1, "rsi" = arg2)
    );
}

pub fn print(string: &str) -> Result<(), SyscallError> {
    let (ptr, len) = (string.as_ptr(), string.len());
    raw::syscall_2(Syscall::Print, ptr as u64, len as u64)
        .map(|_| ())
}

pub fn halt() -> ! {
    let _ = raw::syscall_0(Syscall::Halt);
    unreachable!()
}
