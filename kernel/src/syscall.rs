use crate::gdt::GDT;
use crate::tss::TSS;
use core::cell::UnsafeCell;
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, SFMask, Star};

use crate::halt;
use crate::memory::buffer::BorrowedKernelBuffer;
use crate::memory::paging::{EntryFlags, InvalidateTlb, Page, ZeroPage, ACTIVE_PAGE_TABLES};
use crate::vga::VGA_WRITER;
use core::convert::TryInto;
use core::ptr::NonNull;
use x86_64::registers::rflags::RFlags;
use x86_64::VirtAddr;

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
        selectors.kernel_ds,
    )
    .unwrap();

    // Ignore interrupts on syscall
    SFMask::write(RFlags::INTERRUPT_FLAG);
}

/// # Syscall ABI
///
/// Modified cdecl. Arguments are passed in `rdi, rsi, rdx, rcx, r8, r9`. `rcx` and `r11` are
/// clobbered. The system call number is passed in `rax`, and the return is from `rax` too.
#[naked]
#[no_mangle]
pub extern "C" fn syscall_callback() {
    unsafe {
        // TODO Restore user's FS
        asm!(
            "
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

#[repr(i64)]
enum Error {
    InvalidBuffer = -1,
    InvalidUtf8 = -2,
    InvalidPage = -3,
    InvalidPagesLength = -4,
    OutOfMemory = -5,
}

bitflags::bitflags! {
     pub struct UserPageFlags: u64 {
        const WRITABLE = 1;
        const EXECUTABLE = 1 << 1;
     }
}

impl From<UserPageFlags> for EntryFlags {
    fn from(user: UserPageFlags) -> Self {
        let mut flags = EntryFlags::PRESENT | EntryFlags::USER_ACCESSIBLE;

        if user.contains(UserPageFlags::WRITABLE) {
            flags |= EntryFlags::WRITABLE;
        }

        if !user.contains(UserPageFlags::EXECUTABLE) {
            flags |= EntryFlags::NO_EXECUTE;
        }

        flags
    }
}

#[no_mangle]
pub extern "C" fn syscall_handler(id: u64, argv: *const u64, argc: u64) -> i64 {
    let syscall = Syscall::from_u64(id).unwrap();
    // SAFETY: this is correct (see asm above)
    let args: &[u64] = unsafe { core::slice::from_raw_parts(argv, argc as usize) };
    match syscall {
        Syscall::Halt => {
            info!("Got system call halt");
            halt()
        }
        Syscall::Map => {
            let [addr_begin, len, flags]: [u64; 3] = args[0..3].try_into().unwrap();

            if addr_begin & 0xfff != 0 {
                return Error::InvalidPage as i64;
            }

            if len == 0 {
                return Error::InvalidPagesLength as i64;
            }

            let page_begin = Page::containing_address(addr_begin);
            let page_end = page_begin + (len - 1) as usize;
            let flags = UserPageFlags::from_bits_truncate(flags).into();
            let mut tables = ACTIVE_PAGE_TABLES.lock();

            // SAFETY: we are in the user's page tables
            let res = unsafe {
                tables.try_map_user_range(
                    page_begin..=page_end,
                    flags,
                    InvalidateTlb::Invalidate,
                    false,
                    ZeroPage::Zero,
                )
            };

            res.map(|_| 0).unwrap_or(Error::InvalidPage as i64)
        }
        Syscall::Unmap => {
            let [addr_begin, len]: [u64; 2] = args[0..2].try_into().unwrap();

            if addr_begin & 0xfff != 0 {
                return Error::InvalidPage as i64;
            }

            if len == 0 {
                return Error::InvalidPagesLength as i64;
            }

            todo!()
        }
        Syscall::Print => {
            // SAFETY: we are in the user's page tables
            let res = unsafe {
                BorrowedKernelBuffer::try_from_user(NonNull::new(args[0] as *mut u8), args[1])
            };

            let buf: BorrowedKernelBuffer<u8> = match res {
                Ok(buf) => buf,
                Err(_) => return Error::InvalidBuffer as i64,
            };

            let string = match core::str::from_utf8(buf.0) {
                Ok(str) => str,
                Err(_) => return Error::InvalidUtf8 as i64,
            };

            VGA_WRITER.lock().write_str(string);

            0
        }
    }
}

#[repr(u64)]
pub enum Syscall {
    Halt = 0,
    Map = 1,
    Unmap = 2,
    Print = 3,
}

impl Syscall {
    fn from_u64(v: u64) -> Option<Syscall> {
        match v {
            0 => Some(Syscall::Halt),
            1 => Some(Syscall::Map),
            2 => Some(Syscall::Unmap),
            3 => Some(Syscall::Print),
            _ => None,
        }
    }
}
