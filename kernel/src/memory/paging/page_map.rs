// Taken from https://os.phil-opp.com/remap-the-kernel/
// Many thanks!

use super::*;
use crate::memory::LAST_USABLE_PAGE;
use crate::process::STACK_BOTTOM;
use crate::util::round_up_divide;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::ops::Range;
use core::ops::RangeInclusive;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::PhysFrame;
use x86_64::{PhysAddr, VirtAddr};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FreeMemory {
    Free,
    NoFree,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum InvalidateTlb {
    Invalidate,
    NoInvalidate,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ZeroPage {
    Zero,
    NoZero,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TryMapError {
    InvalidAddress(Page),
    AlreadyMapped(Page),
}

pub struct Mapper {
    p4: NonNull<PageTable<Level4>>,
}

// Safety: must be unique.
unsafe impl Send for Mapper {}
unsafe impl Sync for Mapper {}

impl Mapper {
    /// # Safety
    ///
    /// Must be unique.
    const unsafe fn new() -> Self {
        #[allow(clippy::inconsistent_digit_grouping)] // It's specifically laid out
                                                      // The address points to the recursively mapped entry (511) in the P4 table, which we can
                                                      // use to access the P4 table itself.
                                                      //                sign ext  p4  p3  p2  p1  offset
        const P4: u64 = 0o177_777_776_776_776_776_0000;

        Mapper {
            p4: NonNull::new_unchecked(P4 as *mut _),
        }
    }

    pub fn p4(&self) -> &PageTable<Level4> {
        unsafe { self.p4.as_ref() }
    }

    fn p4_mut(&mut self) -> &mut PageTable<Level4> {
        unsafe { self.p4.as_mut() }
    }

    /// Walks the page tables and translates this page into a physical address
    pub fn walk_page_table(&self, page: Page) -> Option<(PageTableEntry, PageSize)> {
        let p3 = self.p4().next_page_table(page.p4_index());

        let huge_page = || {
            p3.and_then(|p3| {
                // 1GiB page
                let p3_entry = &p3[page.p3_index()];
                if p3_entry.physical_address().is_some()
                    && p3_entry.flags().contains(EntryFlags::HUGE_PAGE)
                {
                    panic!("1 GiB pages are not supported!");
                }

                if let Some(p2) = p3.next_page_table(page.p3_index()) {
                    let p2_entry = &p2[page.p2_index()];

                    // 2MiB page
                    if let Some(start_frame) = p2_entry.physical_address() {
                        if p2_entry
                            .flags()
                            .contains(EntryFlags::PRESENT | EntryFlags::HUGE_PAGE)
                        {
                            // Check that the address is 2MiB aligned
                            assert_eq!(
                                (start_frame.as_u64() >> 12) % PAGE_TABLE_ENTRIES,
                                0,
                                "Adress is not 2MiB aligned!"
                            );
                            return Some((*p2_entry, PageSize::Mib2));
                        }
                    }
                }
                None
            })
        };

        p3.and_then(|p3| p3.next_page_table(page.p3_index()))
            .and_then(|p2| p2.next_page_table(page.p2_index()))
            .and_then(|p1| {
                let p1_entry = p1[page.p1_index()];
                if p1_entry.flags().contains(EntryFlags::PRESENT) {
                    Some((p1_entry, PageSize::Kib4))
                } else {
                    None
                }
            })
            .or_else(huge_page)
    }

    pub unsafe fn map_to(
        &mut self,
        page: Page,
        physical_address: PhysAddr,
        flags: EntryFlags,
        invplg: InvalidateTlb,
    ) {
        let p2 = self
            .p4_mut()
            .next_table_create(page.p4_index())
            .expect("No next p3 table!")
            .next_table_create(page.p3_index())
            .expect("No next p2 table!");

        assert!(page.size.is_some(), "Page to map requires size!");

        if page.size.unwrap() == PageSize::Kib4 {
            let p1 = match p2.next_table_create(page.p2_index()) {
                Some(p1) => p1,
                None => {
                    if p2[page.p2_index()].flags().contains(EntryFlags::HUGE_PAGE) {
                        panic!("No next p1 table - the area is mapped in 2mib pages")
                    } else {
                        panic!("No next p1 table (unknown reason)")
                    }
                }
            };

            // 4kib page
            p1[page.p1_index()].set(physical_address, flags | EntryFlags::PRESENT);

            if invplg == InvalidateTlb::Invalidate {
                tlb::flush(::x86_64::VirtAddr::new(page.start_address().unwrap() as u64));
            }
        } else {
            panic!("2mib pages are only partially supported!");
        }
    }

    /// # Safety notes
    ///
    /// `ZeroPage::Zero` must *not* be passed if this is being used to map into an inactive page table.
    pub unsafe fn map(
        &mut self,
        page: Page,
        flags: EntryFlags,
        invplg: InvalidateTlb,
        zero: ZeroPage,
    ) {
        assert!(page.size.is_some(), "Page needs size!");
        let order = if page.size.unwrap() == PageSize::Kib4 {
            0
        } else {
            9
        };

        let frame = PHYSICAL_ALLOCATOR
            .allocate(order)
            .expect("Out of physical memory!");
        self.map_to(page, frame.start_address(), flags, invplg);

        // Zero the page
        if zero == ZeroPage::Zero {
            crate::util::memset_volatile_64bit(
                page.start_address().unwrap() as *mut u64,
                0,
                page.size.unwrap().bytes() as usize,
            );
        }
    }

    /// Maps a range of pages, allocating physical memory for them
    // TODO use this more widely
    pub unsafe fn map_range(
        &mut self,
        pages: RangeInclusive<Page>,
        flags: EntryFlags,
        invplg: InvalidateTlb,
        zero: ZeroPage,
    ) {
        assert!(
            pages.start().page_size() == Some(PageSize::Kib4)
                && pages.end().page_size() == Some(PageSize::Kib4),
            "Only mapping of 4kib pages is supported"
        );

        for no in pages.start().number()..=pages.end().number() {
            let page = Page::containing_address(no as u64 * 0x1000);
            self.map(page, flags, invplg, zero);
        }
    }

    /// Tries to map a range of pages for a user.
    pub unsafe fn try_map_user_range(
        &mut self,
        pages: RangeInclusive<Page>,
        flags: EntryFlags,
        invplg: InvalidateTlb,
        ignore_already_mapped: bool,
        zero: ZeroPage,
    ) -> Result<(), TryMapError> {
        assert!(
            pages.start().page_size() == Some(PageSize::Kib4)
                && pages.end().page_size() == Some(PageSize::Kib4),
            "Only mapping of 4kib pages is supported"
        );

        let v_start = pages.start().start_address().unwrap();
        let v_end = pages.end().start_address().unwrap();

        // Page above last usable page's last addr + 1 = noncanonical, which creates syscall bug
        if *pages.end() > LAST_USABLE_PAGE {
            trace!("v_end + 1 noncanonical");
            return Err(TryMapError::InvalidAddress(pages.end().clone()));
        }

        // Noncanonical address
        if VirtAddr::try_new(v_end).is_err() {
            return Err(TryMapError::InvalidAddress(pages.end().clone()));
        } else if VirtAddr::try_new(v_start).is_err() {
            return Err(TryMapError::InvalidAddress(pages.start().clone()));
        }

        // Kernel memory (higher half)
        if v_end >> 63 == 1 {
            return Err(TryMapError::InvalidAddress(pages.end().clone()));
        } else if v_start >> 63 == 1 {
            return Err(TryMapError::InvalidAddress(pages.start().clone()));
        }

        // Program stack
        let stack_bottom = Page::containing_address(STACK_BOTTOM.as_u64());
        if *pages.end() > stack_bottom {
            return Err(TryMapError::InvalidAddress(pages.end().clone()));
        }

        for no in pages.start().number()..=pages.end().number() {
            let page = Page::containing_address(no as u64 * 0x1000);

            if !ignore_already_mapped && self.walk_page_table(page).is_some() {
                return Err(TryMapError::AlreadyMapped(page));
            }

            self.map(page, flags, invplg, zero);
        }

        Ok(())
    }

    pub unsafe fn set_flags(
        &mut self,
        pages: RangeInclusive<Page>,
        flags: EntryFlags,
        invplg: InvalidateTlb,
    ) {
        for no in pages.start().number()..=pages.end().number() {
            let page = Page::containing_address(no as u64 * 0x1000);

            let paddr = self
                .walk_page_table(page)
                .expect("Virtual address is not mapped!");
            self.map_to(page, paddr.0.physical_address().unwrap(), flags, invplg)
        }
    }

    pub unsafe fn unmap(&mut self, page: Page, free_physmem: FreeMemory, invplg: InvalidateTlb) {
        assert!(page.start_address().is_some(), "Page to map requires size!");
        assert!(
            self.walk_page_table(page).is_some(),
            "Virtual address 0x{:x} is not mapped!",
            page.start_address().unwrap()
        );

        let p2 = self
            .p4_mut()
            .next_page_table_mut(page.p4_index())
            .expect("Unmap called on unmapped page!")
            .next_page_table_mut(page.p3_index())
            .expect("Unmap called on unmapped page!");

        let p1 = p2.next_page_table_mut(page.p2_index());

        if let Some(p1) = p1 {
            // 4kib page

            let frame = p1[page.p1_index()]
                .physical_address()
                .expect("Page already unmapped!");
            p1[page.p1_index()].set_unused();

            // TODO free p1/p2/p3 tables if they are empty
            if free_physmem == FreeMemory::Free {
                PHYSICAL_ALLOCATOR.deallocate(frame.as_u64(), 0);
            }
        } else {
            // Huge 2mib page

            let frame = p2[page.p2_index()]
                .physical_address()
                .expect("Page already unmapped!");
            p2[page.p2_index()].set_unused();

            // TODO free p2/p3 tables if they are empty
            if free_physmem == FreeMemory::Free {
                PHYSICAL_ALLOCATOR.deallocate(frame.as_u64(), 9);
            }
        }

        if invplg == InvalidateTlb::Invalidate {
            // Flush tlb
            tlb::flush(::x86_64::VirtAddr::new(page.start_address().unwrap() as u64));
        }
    }

    /// Identity maps a range of addresses as 4 kib pages
    pub unsafe fn id_map_range(
        &mut self,
        addresses: RangeInclusive<u64>,
        flags: EntryFlags,
        invplg: InvalidateTlb,
    ) {
        for frame_no in (addresses.start() / 4096)..=(addresses.end() / 4096) {
            let addr = (frame_no * 4096) as u64;
            self.map_to(
                Page::containing_address(addr),
                PhysAddr::new(addr as u64),
                flags,
                invplg,
            );
        }
    }

    /// Maps a range of higher half addresses as 4kib pages in the -2GiB higher "half", mapping
    /// them to their address minus `KERNEL_MAPPING_BEGIN`.
    pub unsafe fn higher_half_map_range(
        &mut self,
        addresses: Range<u64>,
        flags: EntryFlags,
        invplg: InvalidateTlb,
    ) {
        let frame_end = round_up_divide(addresses.end as u64, 4096) as u64;
        for frame_no in (addresses.start / 4096)..=frame_end {
            let address = frame_no * 4096;

            self.map_to(
                Page::containing_address(address),
                PhysAddr::new(address - crate::memory::KERNEL_MAPPING_BEGIN),
                flags,
                invplg,
            );
        }
    }

    pub unsafe fn map_page_range(
        &mut self,
        mapping: PageRangeMapping,
        invplg: InvalidateTlb,
        flags: EntryFlags,
    ) {
        let frames =
            mapping.start_frame..=mapping.start_frame + mapping.pages.size_hint().1.unwrap() as u64;

        for (frame_no, page_no) in frames.zip(mapping.pages) {
            let phys_address = frame_no * 4096;
            let virtual_address = page_no * 4096;

            self.map_to(
                Page::containing_address(virtual_address),
                PhysAddr::new(phys_address as u64),
                flags,
                invplg,
            );
        }
    }
}

/// A 4kib page range mapping -- represents a contigous area of 4kib pages mapped to a contigous
/// area of 4kib frames. However, this does not need to be an identity mapping, i.e there may be
/// an offset
pub struct PageRangeMapping {
    /// Range of page numbers
    pub pages: RangeInclusive<u64>,

    /// The start frame number
    pub start_frame: u64,
}

impl PageRangeMapping {
    pub fn new(start_page: Page, start_frame: u64, pages: u64) -> PageRangeMapping {
        assert_eq!(
            start_page.page_size(),
            Some(PageSize::Kib4),
            "Start page needs to be 4kib!"
        );
        let page_number = start_page.start_address().unwrap() / 4096;

        PageRangeMapping {
            pages: page_number..=(page_number + pages),
            start_frame,
        }
    }
}

pub struct TemporaryPage {
    page: Page,
    frame_addr: PhysAddr,
}

impl TemporaryPage {
    pub fn new() -> TemporaryPage {
        // Allocate some heap memory for us to put the temporary page on (virtual addr)
        let layout = Layout::from_size_align(0x1000, 0x1000).unwrap();
        let page_addr = unsafe { crate::HEAP.alloc(layout) };
        let page = Page::containing_address(page_addr as u64);
        let frame_addr = ACTIVE_PAGE_TABLES
            .lock()
            .walk_page_table(page)
            .unwrap()
            .0
            .physical_address()
            .unwrap();

        // Unmap the heap page temporarily to avoid confusing the temporary page code
        unsafe {
            ACTIVE_PAGE_TABLES
                .lock()
                .unmap(page, FreeMemory::NoFree, InvalidateTlb::Invalidate);
        }

        TemporaryPage { page, frame_addr }
    }

    /// Maps the temporary page to the given frame in the active table.
    /// Returns the start address of the temporary page.
    pub unsafe fn map(&mut self, frame: PhysAddr, active_table: &mut ActivePageMap) -> VirtAddr {
        let page_addr = self
            .page
            .start_address()
            .expect("Temporary page requires size");
        assert!(
            active_table.walk_page_table(self.page).is_none(),
            "Temporary page {:?} at 0x{:x} is already mapped",
            self.page,
            page_addr,
        );

        active_table.map_to(
            self.page,
            frame,
            EntryFlags::WRITABLE,
            InvalidateTlb::Invalidate,
        );
        VirtAddr::new(
            self.page
                .start_address()
                .expect("Page in TemporaryPage requires size"),
        )
    }

    /// Unmaps the temporary page in the active table.
    pub unsafe fn unmap(&mut self, active_table: &mut ActivePageMap) {
        active_table.unmap(self.page, FreeMemory::NoFree, InvalidateTlb::Invalidate);
    }

    pub unsafe fn map_table_frame(
        &mut self,
        frame: PhysAddr,
        active_table: &mut ActivePageMap,
    ) -> &mut PageTable<Level1> {
        &mut *(self.map(frame, active_table).as_mut_ptr())
    }
}

impl Drop for TemporaryPage {
    fn drop(&mut self) {
        // Remap heap page so it can be deallocated correctly
        unsafe {
            ACTIVE_PAGE_TABLES.lock().map_to(
                self.page,
                self.frame_addr,
                EntryFlags::from_bits_truncate(0),
                InvalidateTlb::Invalidate,
            );
        }

        let layout = Layout::from_size_align(0x1000, 0x1000).unwrap();

        unsafe { crate::HEAP.dealloc(self.page.start_address().unwrap() as *mut _, layout) };
    }
}

pub struct ActivePageMap {
    mapper: Mapper,
}

impl ActivePageMap {
    pub const unsafe fn new() -> ActivePageMap {
        ActivePageMap {
            mapper: Mapper::new(),
        }
    }

    pub fn with_inactive_p4<F: FnOnce(&mut ActivePageMap) -> R, R>(
        &mut self,
        table: &mut InactivePageMap,
        temporary_page: &mut TemporaryPage,
        f: F,
    ) -> R {
        let ret = {
            let backup = PhysAddr::new(
                x86_64::registers::control::Cr3::read()
                    .0
                    .start_address()
                    .as_u64(),
            );

            // map temporary_page to current p4 table
            let p4_table = unsafe { temporary_page.map_table_frame(backup, self) };

            // overwrite recursive mapping
            self.p4_mut()[510].set(
                table.p4_frame.clone().start_address(),
                EntryFlags::PRESENT
                    | EntryFlags::WRITABLE
                    | EntryFlags::NO_EXECUTE
                    | EntryFlags::USER_ACCESSIBLE,
            );

            tlb::flush_all();

            // execute f in the new context
            let ret = f(self);

            // restore recursive mapping to original p4 table
            p4_table[510].set(
                backup,
                EntryFlags::PRESENT
                    | EntryFlags::WRITABLE
                    | EntryFlags::NO_EXECUTE
                    | EntryFlags::USER_ACCESSIBLE,
            );

            tlb::flush_all();

            ret
        };

        unsafe {
            temporary_page.unmap(self);
        }

        ret
    }

    pub fn remap_range(
        &mut self,
        new_table: &mut InactivePageMap,
        temporary_page: &mut TemporaryPage,
        pages: RangeInclusive<u64>,
        flags: EntryFlags,
    ) {
        let num_pages = pages.end() - pages.start();
        let mut frames = Vec::with_capacity(num_pages as usize);
        for i in 0..=num_pages {
            let page = Page::containing_address((i + pages.start()) * 4096);

            let entry = self.walk_page_table(page).unwrap().0;
            frames.push(entry.physical_address().unwrap());
        }

        self.with_inactive_p4(new_table, temporary_page, |mapper| {
            for page_no in pages.clone() {
                let page = Page::containing_address(page_no * 4096);
                let phys_addr = frames[page_no as usize - *pages.start() as usize];

                unsafe {
                    mapper.map_to(page, phys_addr, flags, InvalidateTlb::NoInvalidate);
                }
            }
        });
    }

    pub fn with_inactive<F, E>(
        &mut self,
        new_table: InactivePageMap,
        f: F,
    ) -> Result<InactivePageMap, E>
    where
        F: FnOnce(&mut ActivePageMap) -> Result<(), E>,
    {
        let old = self.switch(new_table);
        let res = f(self);
        let new = self.switch(old);
        res.map(|_| new)
    }

    pub fn switch(&mut self, new_table: InactivePageMap) -> InactivePageMap {
        let (p4_frame, flags) = Cr3::read();
        let old_table = InactivePageMap { p4_frame, flags };

        unsafe {
            Cr3::write(new_table.p4_frame, new_table.flags);
        }

        old_table
    }
}

impl Deref for ActivePageMap {
    type Target = Mapper;

    fn deref(&self) -> &Mapper {
        &self.mapper
    }
}

impl DerefMut for ActivePageMap {
    fn deref_mut(&mut self) -> &mut Mapper {
        &mut self.mapper
    }
}

#[derive(Debug, Clone)]
pub struct InactivePageMap {
    p4_frame: PhysFrame,
    flags: Cr3Flags,
}

impl Default for InactivePageMap {
    fn default() -> Self {
        InactivePageMap {
            p4_frame: PhysFrame::from_start_address(PhysAddr::new(0)).unwrap(),
            flags: Cr3Flags::from_bits_truncate(0),
        }
    }
}

impl InactivePageMap {
    /// # Safety:
    ///
    /// Frame must be valid.
    pub unsafe fn new(
        frame: PhysFrame,
        flags: Cr3Flags,
        active_table: &mut ActivePageMap,
        temporary_page: &mut TemporaryPage,
    ) -> InactivePageMap {
        {
            // SAFETY: frame must be valid (declared above in doc comment)
            let table = temporary_page.map_table_frame(frame.start_address(), active_table);

            table.zero();

            // Set up recursive mapping for table
            table[510].set(
                frame.start_address(),
                EntryFlags::PRESENT
                    | EntryFlags::WRITABLE
                    | EntryFlags::NO_EXECUTE
                    | EntryFlags::USER_ACCESSIBLE, // TODO(userspace0
            );
        }

        temporary_page.unmap(active_table);

        InactivePageMap {
            p4_frame: frame,
            flags,
        }
    }
}
