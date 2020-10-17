use crate::gdt::GDT;
use core::cell::UnsafeCell;
use crate::tss::TSS;
use x86_64::registers::model_specific::{EferFlags, Efer, LStar, Star, SFMask};

use x86_64::VirtAddr;
use x86_64::registers::rflags::RFlags;
use crate::halt;
use crate::vga::VGA_WRITER;

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

            // Push arguments (reverse order because of slice)
            push rdx
            push rsi
            push rdi

            // Re-enable interrupts
            sti

            // Make a slice out of the arguments
            mov rsi, rsp // ptr
            mov rdx, 3 // len
            mov rdi, rax // syscall number
            call syscall_handler

            // Pop arguments
            pop rdi
            pop rsi
            pop rdx

            pop r11 // RCX = userland IP,
            pop rcx // R11 = userland EFLAGS

            mov rsp, [USER_RSP] // Restore user's rsp

            sysretq",
        )
    }
}

#[no_mangle]
pub extern "C" fn syscall_handler(id: u64, argv: *const u64, argc: u64) -> isize {
    let syscall = Syscall::from_u64(id).unwrap();
    // SAFETY: this is correct (see asm above)
    let args: &[u64] = unsafe { core::slice::from_raw_parts(argv, argc as usize) };
    match syscall {
        Syscall::Halt => {
            info!("Got system call halt");
            unsafe { halt() }
        },
        Syscall::Deadbeef => 0xdeadbeef,
        Syscall::Print => {
            // TODO(syscall buffers): check this
            let slice: &[u8] = unsafe {
                core::slice::from_raw_parts(
                    args[0] as *const u8,
                    args[1] as usize
                )
            };

            let string = core::str::from_utf8(slice).unwrap();
            VGA_WRITER.lock().write_str(string);
            0
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
            2 => Some(Syscall::Print),
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
