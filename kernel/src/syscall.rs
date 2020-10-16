use crate::gdt::GDT;
use core::cell::UnsafeCell;
use crate::tss::TSS;
use x86_64::registers::model_specific::{EferFlags, Efer, LStar, Star, SFMask};

use x86_64::VirtAddr;
use x86_64::registers::rflags::RFlags;

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

pub unsafe fn setup_syscall() {
    *SYSCALL_STACK.0.get() = TSS.wait().unwrap().tss.privilege_stack_table[0].as_u64();

    // Enable system calls
    Efer::update(|flags| *flags |= EferFlags::SYSTEM_CALL_EXTENSIONS);

    // Set the system call handler
    LStar::write(VirtAddr::new(syscall_callback as u64));

    let selectors = &GDT.selectors;

   //panic!("{} {} {} {}", selectors.user_cs.0, selectors.user_ds.0, selectors.kernel_cs.0, selectors.kernel_ds.0);

    Star::write(
        selectors.user_cs,
        selectors.user_ds,
        selectors.kernel_cs,
        selectors.kernel_ds
    ).unwrap();

    // Ignore interrupts on syscall
    SFMask::write(RFlags::from_bits_truncate(0));
}

#[naked]
#[no_mangle]
pub extern fn syscall_callback() {
    info!("aaa");
    unsafe {
        // TODO Restore user's FS
        asm!("
            mov [USER_RSP], rsp // Save RSP
            mov rsp, SYSCALL_STACK
            push rax // Push system call number

            // Save user state

            push rcx // RCX = userland IP,
            push r11 // R11 = userland EFLAGS
            push rdi // Save user regs
            push rsi
            push rdx
            push r10
            push r8
            push r9
            sti

            mov rdi, rax
            mov rsi, rsp
            mov rdx, 6

            call syscall_handler

            pop rcx
            pop r11
            pop rdi
            pop rsi
            pop rdx
            pop r10
            pop r8
            pop r9
            mov rsp, [USER_RSP] // Restore user's rsp

            sysretq",
        )
    }
}

#[no_mangle]
pub extern "C" fn syscall_handler(id: usize) {
    match id {
        0 => panic!("WAT"),
        _ => {},
    };
}
