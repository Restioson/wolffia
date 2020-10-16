use crate::memory::paging::*;
use core::sync::atomic::{AtomicU64, Ordering};
use dashmap::DashMap;

use crate::memory::physical_allocator::PHYSICAL_ALLOCATOR;
use crate::tss::TSS;
use alloc::vec::Vec;
use core::ops::RangeInclusive;
use x86_64::registers::control::Cr3;
use x86_64::VirtAddr;

// Top of lower half but page aligned
const STACK_TOP: VirtAddr = VirtAddr::new_truncate(0x7ffffffff000);
const INITIAL_STACK_SIZE_PAGES: usize = 16; // 64kib stack

lazy_static::lazy_static! {
    pub static ref PROCESSES: DashMap<ProcessId, Process> = DashMap::default();
}

pub static NEXT_PID: AtomicU64 = AtomicU64::new(0);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct ProcessId(u64);

impl ProcessId {
    pub fn next() -> Self {
        let next_pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);

        assert!(
            next_pid < u64::max_value(),
            "Ran out of process ids. This should never happen"
        );

        ProcessId(next_pid)
    }
}

#[derive(Debug)]
pub struct Process {
    pub page_tables: InactivePageMap,
    stack_ptr: VirtAddr,
    instruction_ptr: VirtAddr,
    io_port_ranges: Vec<RangeInclusive<u16>>,
    new: bool,
}

impl Process {
    /// # Safety
    ///
    /// Instruction ptr must be valid.
    pub unsafe fn spawn(instruction_ptr: VirtAddr) -> ProcessId {
        let page_tables = Self::new_process_page_tables();

        let process = Process {
            page_tables,
            stack_ptr: STACK_TOP,
            instruction_ptr,
            io_port_ranges: Vec::new(),
            new: true,
        };

        let pid = ProcessId::next();
        PROCESSES.insert(pid, process);

        pid
    }

    fn new_process_page_tables() -> InactivePageMap {
        let mut temporary_page = TemporaryPage::new();

        // This must be duplicated to avoid double locks. This is safe though -- in this context!
        let mut active_table = unsafe { ActivePageMap::new() };

        let frame = PHYSICAL_ALLOCATOR.allocate(0).expect("no more frames");
        let new_table = unsafe {
            // SAFETY: frames returned from the physical allocator are always valid.
            InactivePageMap::new(frame, Cr3::read().1, &mut active_table, &mut temporary_page)
        };

        // Copy kernel pml4 entry
        let kernel_pml4_entry = active_table.p4()[511];
        let table =
            unsafe { temporary_page.map_table_frame(frame.start_address(), &mut active_table) };

        table[511] = kernel_pml4_entry;

        unsafe {
            temporary_page.unmap(&mut active_table);
        }

        // Drop this lock so that the RAII guarded temporary page can be destroyed
        drop(active_table);

        new_table
    }

    pub fn run(&mut self) -> ! {
        ACTIVE_PAGE_TABLES.lock().switch(self.page_tables.clone());

        if self.new {
            unsafe {
                self.setup();
            }
            self.new = false;
        }

        // TODO(userspace) track process io ports
        TSS.wait()
            .unwrap()
            .iomap
            .lock_or_panic()
            .set_port_range_usable(0x3f8..=0x3F8 + 7, true);

        unsafe { jump_usermode(self.stack_ptr, self.instruction_ptr) }
    }

    /// Sets up the process for it to be run for the first time.
    ///
    /// # Safety
    ///
    /// The page tables must have been switched to the process's AND the processor must be in ring0.
    unsafe fn setup(&mut self) {
        // Set up user stack
        let stack_top = Page::containing_address(STACK_TOP.as_u64(), PageSize::Kib4);
        let stack_bottom = stack_top - INITIAL_STACK_SIZE_PAGES;

        ACTIVE_PAGE_TABLES.lock().map_range(
            stack_bottom..=stack_top,
            EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE | EntryFlags::NO_EXECUTE,
            InvalidateTlb::NoInvalidate,
            ZeroPage::Zero,
        );
    }
}

/// # Safety
///
/// Expects to be in the page tables where instruction and stack pointer are loaded and valid.
unsafe fn jump_usermode(stack_ptr: VirtAddr, instruction_ptr: VirtAddr) -> ! {
    asm!(
    "
        mov ax, 0x2b
        mov ds, ax
        mov es, ax
        mov fs, ax
        mov gs, ax

        push 0x2b // stack segment
        push {0} // stack pointer
        pushfq // push RFLAGS
        push 0x33 // code segment
        push {1} // instruction pointer
        iretq
        ",
    in(reg) stack_ptr.as_u64(),
    in(reg) instruction_ptr.as_u64(),
    );

    unreachable!()
}
