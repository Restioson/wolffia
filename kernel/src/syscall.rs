use crate::gdt::GDT;
use core::cell::UnsafeCell;
use crate::tss::TSS;
use x86_64::registers::model_specific::{EferFlags, Efer, LStar, Star, SFMask};

use x86_64::VirtAddr;
use x86_64::registers::rflags::RFlags;
use crate::halt;

// TODO(SMP): use gs/swapgs
/// SAFETY: always used from asm, one at a time.
#[no_mangle]
static mut USER_RSP: AsmCell<u64> = AsmCell(UnsafeCell::new(0));
#[no_mangle]
static SYSCALL_STACK: AsmCell<u64> = AsmCell(UnsafeCell::new(0));

#[repr(transparent)]
struct AsmCell<T>(UnsafeCell<T>);
unsafe impl<T> Send for AsmCell<T> {}
unsafe impl<T> Sync for AsmCell<T> {}

/// # Safety
///
/// TSS's `privilege_stack_table[0]` must be initialised to a valid value.
pub unsafe fn setup_syscall() {
    *SYSCALL_STACK.0.get() = TSS.wait().unwrap().tss.privilege_stack_table[0].as_u64();

    // Enable system calls
    Efer::update(|flags| *flags |= EferFlags::SYSTEM_CALL_EXTENSIONS);

    // Set the system call handler
    LStar::write(VirtAddr::new(syscall_callback as u64));

    let selectors = &GDT.selectors;

    Star::write(
        selectors.user_cs,
        selectors.user_ds,
        selectors.kernel_cs,
        selectors.kernel_ds
    ).unwrap();

    // Ignore interrupts on syscall
    SFMask::write(RFlags::INTERRUPT_FLAG);
}

// TODO noncanonical rip/rcx https://fuchsia.dev/fuchsia-src/concepts/kernel/sysret_problem
/// # Syscall ABI
///
/// Modified cdecl. Arguments are passed in `rdi, rsi, rdx, rcx, r8, r9`. `rcx` and `r11` are
/// clobbered. The system call number is passed in `rax`, and the return is from `rax` too.
#[naked]
#[no_mangle]
pub extern fn syscall_callback() {
    unsafe {
        // TODO Restore user's FS
        asm!("
            mov [USER_RSP], rsp // Save RSP
            mov rsp, SYSCALL_STACK

            push rcx // RCX = userland IP,
            push r11 // R11 = userland EFLAGS
            sti // Re-enable interrupts

            mov rdi, rax // First arg in rdi
            call syscall_handler

            pop r11 // RCX = userland IP,
            pop rcx // R11 = userland EFLAGS

            mov rsp, [USER_RSP] // Restore user's rsp

            sysretq",
        )
    }
}

#[no_mangle]
pub extern "C" fn syscall_handler(id: u64, arg1: u64, arg2: u64) -> isize {
    let syscall = Syscall::from_u64(id).unwrap();
    match syscall {
        Syscall::Halt => {
            info!("Got system call halt");
            unsafe { halt() }
        },
        Syscall::Deadbeef => 0xdeadbeef,
        Syscall::Print => {
            // TODO(syscall buffers)
            panic!()
        }
    }
}

#[repr(u64)]
pub enum Syscall {
    Halt = 0,
    Deadbeef = 1,
    Print = 2,
}

impl Syscall {
    fn from_u64(v: u64) -> Option<Syscall> {
        match v {
            0 => Some(Syscall::Halt),
            1 => Some(Syscall::Deadbeef),
            _ => None,
        }
    }
}

pub extern "C" fn syscall_raw(call: Syscall) -> i64 {
    let out: i64;
    unsafe {
        asm!("syscall",
            in("rax") call as u64,
            lateout("rax") out,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    };
    out
}
