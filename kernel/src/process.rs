use crate::memory::paging::*;
use core::sync::atomic::{AtomicU64, Ordering};
use dashmap::DashMap;

use crate::memory::physical_allocator::PHYSICAL_ALLOCATOR;
use crate::tss::TSS;
use alloc::vec::Vec;
use core::ops::{Range, RangeInclusive};
use core::slice;
use goblin::elf::program_header::PT_LOAD;
use goblin::elf::Elf;
use x86_64::registers::control::Cr3;
use x86_64::VirtAddr;

// Top of lower half minus 1 but page aligned
pub const STACK_TOP: VirtAddr = VirtAddr::new_truncate(0x7fffffffe000);
pub const INITIAL_STACK_SIZE_PAGES: usize = 16; // 64kib stack
pub const STACK_BOTTOM: VirtAddr =
    VirtAddr::new_truncate(STACK_TOP.as_u64() - INITIAL_STACK_SIZE_PAGES as u64);

lazy_static::lazy_static! {
    pub static ref PROCESSES: DashMap<ProcessId, Process> = DashMap::default();
}

static NEXT_PID: AtomicU64 = AtomicU64::new(0);

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

#[derive(Debug)]
pub enum ElfLaunchError {
    NotExecutable,
    Not64Bit,
    NotStaticallyLinked,
    /// The process attempted to have a page mapped at an invalid address
    InvalidPage(TryMapError),
    InvalidEntryPoint(u64),
    ParseError(goblin::error::Error),
    InvalidHeaderRange(Range<usize>),
}

impl Process {
    pub fn spawn_from_elf(data: &[u8]) -> Result<ProcessId, ElfLaunchError> {
        let elf = Elf::parse(data).map_err(ElfLaunchError::ParseError)?;

        if elf.is_lib || elf.entry == 0 {
            return Err(ElfLaunchError::NotExecutable);
        }

        if !elf.is_64 {
            return Err(ElfLaunchError::Not64Bit);
        }

        if !elf.libraries.is_empty() {
            return Err(ElfLaunchError::NotStaticallyLinked);
        }

        let page_tables = Self::new_process_page_tables();
        let page_tables = ACTIVE_PAGE_TABLES
            .lock()
            .with_inactive(page_tables, |tables| {
                for p_header in &elf.program_headers {
                    if p_header.p_type != PT_LOAD {
                        continue;
                    }

                    let mut flags = EntryFlags::USER_ACCESSIBLE;
                    let vm_range = p_header.vm_range();

                    if vm_range.contains(&0) {
                        let zpg = Page::containing_address(0);
                        return Err(ElfLaunchError::InvalidPage(TryMapError::InvalidAddress(
                            zpg,
                        )));
                    }

                    let page_start = Page::containing_address(vm_range.start as u64);
                    let page_end = Page::containing_address(vm_range.end as u64 - 1);

                    if !p_header.is_executable() {
                        flags |= EntryFlags::NO_EXECUTE;
                    }

                    if p_header.is_write() {
                        flags |= EntryFlags::WRITABLE;
                    }

                    unsafe {
                        tables
                            .try_map_user_range(
                                page_start..=page_end,
                                EntryFlags::WRITABLE,
                                InvalidateTlb::NoInvalidate,
                                true, // ignore_already_mapped
                                ZeroPage::NoZero,
                            )
                            .map_err(ElfLaunchError::InvalidPage)?;

                        let src_slice = data
                            .get(p_header.file_range())
                            .ok_or(ElfLaunchError::InvalidHeaderRange(p_header.file_range()))?;

                        // SAFETY: range is TrustedLen
                        let dst_slice =
                            slice::from_raw_parts_mut(vm_range.start as *mut u8, vm_range.len());

                        if dst_slice.len() != src_slice.len() {
                            return Err(ElfLaunchError::InvalidHeaderRange(p_header.file_range()));
                        }

                        dst_slice.copy_from_slice(src_slice);

                        tables.set_flags(page_start..=page_end, flags, InvalidateTlb::NoInvalidate);
                    }
                }

                Ok(())
            })?;

        // Kernel space or non canonical address... no.
        if elf.entry >> 63 == 1 || VirtAddr::try_new(elf.entry).is_err() {
            return Err(ElfLaunchError::InvalidEntryPoint(elf.entry));
        }

        let process = Process {
            page_tables,
            stack_ptr: STACK_TOP,
            instruction_ptr: VirtAddr::new(elf.entry),
            io_port_ranges: Vec::new(),
            new: true,
        };

        let pid = ProcessId::next();
        PROCESSES.insert(pid, process);

        Ok(pid)
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

    pub fn run_by_pid(pid: &ProcessId) -> Result<!, OutOfMemory> {
        let mut this = PROCESSES.get_mut(pid).unwrap();
        ACTIVE_PAGE_TABLES.lock().switch(this.page_tables.clone());

        if this.new {
            unsafe {
                this.setup()?;
            }
            this.new = false;
        }

        // TODO(permissions) track process io ports
        TSS.wait()
            .unwrap()
            .iomap
            .lock_or_panic()
            .set_port_range_usable(0x3f8..=0x3F8 + 7, true);

        let (rsp, rip) = (this.stack_ptr, this.instruction_ptr);
        drop(this);
        unsafe { jump_usermode(rsp, rip) }
    }

    /// Sets up the process for it to be run for the first time.
    ///
    /// # Safety
    ///
    /// The page tables must have been switched to the process's AND the processor must be in ring0.
    unsafe fn setup(&mut self) -> Result<(), OutOfMemory> {
        // Set up user stack
        let stack_top = Page::containing_address(STACK_TOP.as_u64());
        let stack_bottom = Page::containing_address(STACK_BOTTOM.as_u64());

        ACTIVE_PAGE_TABLES.lock().map_range(
            stack_bottom..=stack_top,
            EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE | EntryFlags::NO_EXECUTE,
            InvalidateTlb::NoInvalidate,
            ZeroPage::Zero,
        )
    }
}

/// # Safety
///
/// Expects to be in the page tables where instruction and stack pointer are loaded and valid.
unsafe fn jump_usermode(stack_ptr: VirtAddr, instruction_ptr: VirtAddr) -> ! {
    asm!("
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
