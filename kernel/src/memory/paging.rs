//! Various functions and structures to work with paging, page tables, and page table entries.
//! Thanks a __lot__ to [Phil Opp's paging blogpost](https://os.phil-opp.com/page-tables/).

mod page_map;
pub mod remap;
pub use self::page_map::*;

use super::physical_allocator::PHYSICAL_ALLOCATOR;
use bitflags::_core::cmp::Ordering;
use core::iter::Step;
use core::marker::PhantomData;
use core::ops::{Add, Index, IndexMut, Sub};
use spin::Mutex;
use x86_64::instructions::tlb;
use x86_64::registers::control::Cr3;
use x86_64::PhysAddr;

const PAGE_TABLE_ENTRIES: u64 = 512;
pub static ACTIVE_PAGE_TABLES: Mutex<ActivePageMap> = Mutex::new(unsafe { ActivePageMap::new() });

/// The size of a page. Distinct from `memory::PageSize` in that it only enumerates page sizes
/// supported by the paging module at this time.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Ord, PartialOrd)]
pub enum PageSize {
    Kib4,
    Mib2,
}

impl PageSize {
    const fn bytes(self) -> u64 {
        use self::PageSize::*;

        match self {
            Kib4 => 4 * 1024,
            Mib2 => 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Page {
    pub(super) number: usize,
    /// Size of page. None when unknown.
    pub(super) size: Option<PageSize>,
}

impl Page {
    fn p4_index(&self) -> usize {
        (self.number >> 27) & 0o777
    }

    fn p3_index(&self) -> usize {
        (self.number >> 18) & 0o777
    }

    fn p2_index(&self) -> usize {
        (self.number >> 9) & 0o777
    }

    fn p1_index(&self) -> usize {
        self.number & 0o777
    }

    pub const fn number(&self) -> usize {
        self.number
    }

    pub fn start_address(&self) -> Option<u64> {
        self.size.map(|size| self.number as u64 * size.bytes())
    }

    pub fn page_size(&self) -> Option<PageSize> {
        self.size
    }

    /// The 4kib page containing an address
    pub const fn containing_address(addr: u64) -> Page {
        Page {
            number: (addr / PageSize::Kib4.bytes()) as usize,
            size: Some(PageSize::Kib4),
        }
    }
}

impl Add<usize> for Page {
    type Output = Page;

    fn add(self, other: usize) -> Page {
        Page {
            number: self.number + other,
            size: self.size,
        }
    }
}

impl Sub<usize> for Page {
    type Output = Page;

    fn sub(self, other: usize) -> Page {
        Page {
            number: self.number - other,
            size: self.size,
        }
    }
}

impl PartialOrd<Page> for Page {
    fn partial_cmp(&self, other: &Page) -> Option<Ordering> {
        self.number.partial_cmp(&other.number)
    }
}

impl Ord for Page {
    fn cmp(&self, other: &Self) -> Ordering {
        self.number.cmp(&other.number)
    }
}

unsafe impl Step for Page {
    fn steps_between(start: &Page, end: &Page) -> Option<usize> {
        usize::steps_between(&start.number, &end.number)
    }

    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        if start.number.checked_add(count).is_some() {
            Some(start + count)
        } else {
            None
        }
    }

    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        if start.number.checked_sub(count).is_some() {
            Some(start - count)
        } else {
            None
        }
    }
}

/// An entry in a page table
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(transparent)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    pub fn set_unused(&mut self) {
        self.0 = 0;
    }

    pub fn flags(&self) -> EntryFlags {
        EntryFlags::from_bits_truncate(self.0)
    }

    pub fn physical_address(&self) -> Option<PhysAddr> {
        if self.flags().contains(self::EntryFlags::PRESENT) {
            Some(PhysAddr::new(self.0 & 0x000FFFFF_FFFFF000)) // Mask out the flag bits
        } else {
            None
        }
    }

    pub fn add_flags(&mut self, flags: EntryFlags) {
        self.0 |= flags.bits();
    }

    pub fn set(&mut self, physical_address: PhysAddr, flags: EntryFlags) {
        // Check that the physical address is page aligned
        assert_eq!(
            physical_address.as_u64() & 0xFFF,
            0,
            "Physical address 0x{:x} not page aligned!",
            physical_address.as_u64(),
        );

        // Check that physical address is correctly sign extended
        let bit_47 = (physical_address.as_u64() >> 48) & 1;
        if bit_47 == 1 {
            assert_eq!(
                physical_address.as_u64() >> 48,
                0xFFFF,
                "Physical address 0x{:x} is not correctly sign extended!",
                physical_address.as_u64(),
            )
        } else {
            assert_eq!(
                physical_address.as_u64() >> 48,
                0,
                "Physical address 0x{:x} is not correctly sign extended!",
                physical_address.as_u64(),
            )
        }

        self.0 = (physical_address.as_u64() as u64) | flags.bits();
    }
}

bitflags::bitflags! {
    pub struct EntryFlags: u64 {
        /// Whether the page is present in memory
        const PRESENT = 1;
        /// Whether the page is writable or read only
        const WRITABLE = 1 << 1;
        /// Whether ring 3 processes can access this page -- in theory. As of meltdown, this bit is
        /// essentially useless, except on possibly newer CPUs with fixes in place
        const USER_ACCESSIBLE = 1 << 2;
        /// If this bit is set, writes to this page go directly to memory
        const WRITE_DIRECT = 1 << 3;
        /// Do not use cache for this page
        const NO_CACHE = 1 << 4;
        /// Set by the CPU when this page has been accessed
        const ACCESSED = 1 << 5;
        /// Set by the CPU when this page is written to
        const DIRTY = 1 << 6;
        /// Whether this page is a huge page. 0 in P1 and P4, but sets this as a 1GiB page in P3
        /// and a 2MiB page in P2
        const HUGE_PAGE = 1 << 7;
        /// If set, this page will not be flushed in the TLB if CR3 is reset. PGE bit in CR4 must be set.
        const GLOBAL = 1 << 8; // TODO(userspace): map kernel pages as global?
        /// Do not allow executing code from this page. NXE bit in EFER must be set.
        const NO_EXECUTE = 1 << 63;
    }
}

/// A trait that indicates a type represents a page table level
pub trait TableLevel {}

pub enum Level4 {}
pub enum Level3 {}
pub enum Level2 {}
pub enum Level1 {}

impl TableLevel for Level4 {}
impl TableLevel for Level3 {}
impl TableLevel for Level2 {}
impl TableLevel for Level1 {}

/// A trait that indicates a type represents a page table level that is not P1
pub trait HierarchicalLevel: TableLevel {
    type NextLevel: TableLevel;

    const CAN_BE_HUGE: bool = false;
}

impl HierarchicalLevel for Level4 {
    type NextLevel = Level3;
}

impl HierarchicalLevel for Level3 {
    type NextLevel = Level2;
    const CAN_BE_HUGE: bool = true;
}

impl HierarchicalLevel for Level2 {
    type NextLevel = Level1;
    const CAN_BE_HUGE: bool = true;
}

/// A page table consisting of 512 entries ([PageTableEntry]).
pub struct PageTable<L: TableLevel> {
    entries: [PageTableEntry; PAGE_TABLE_ENTRIES as usize],
    _level: PhantomData<L>,
}

impl<L: TableLevel> PageTable<L> {
    pub fn zero(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.set_unused();
        }
    }

    fn next_table_addr(&self, index: usize) -> Option<u64>
    where
        L: HierarchicalLevel,
    {
        let entry_flags = self[index].flags();

        if entry_flags.contains(self::EntryFlags::PRESENT)
            && !entry_flags.contains(self::EntryFlags::HUGE_PAGE)
        {
            let table_address = self as *const _ as u64;
            Some((0xFFFF << 48) | (table_address << 9) | ((index as u64) << 12))
        // HEADS UP ^. This first mask would change if the p4 table were recursively mapped to
        // an entry in the 0 sign extended half of the address space. BEWARE!
        } else {
            None
        }
    }

    fn next_page_table(&self, index: usize) -> Option<&PageTable<L::NextLevel>>
    where
        L: HierarchicalLevel,
    {
        unsafe { self.next_table_addr(index).map(|addr| &*(addr as *const _)) }
    }

    fn next_page_table_mut(&mut self, index: usize) -> Option<&mut PageTable<L::NextLevel>>
    where
        L: HierarchicalLevel,
    {
        unsafe {
            self.next_table_addr(index)
                .map(|addr| &mut *(addr as *mut _))
        }
    }

    pub fn next_table_create(&mut self, index: usize) -> Option<&mut PageTable<L::NextLevel>>
    where
        L: HierarchicalLevel,
    {
        if self.next_page_table(index).is_none() {
            if self.entries[index]
                .flags()
                .contains(self::EntryFlags::HUGE_PAGE)
            {
                assert!(L::CAN_BE_HUGE, "Page has huge bit but cannot be huge!");
            } else {
                let frame = PHYSICAL_ALLOCATOR
                    .allocate(0)
                    .expect("No physical frames available!");

                self.entries[index].set(
                    frame.start_address(),
                    self::EntryFlags::PRESENT
                        | self::EntryFlags::WRITABLE
                        | self::EntryFlags::USER_ACCESSIBLE,
                );
                self.next_page_table_mut(index)
                    .expect("No next table!")
                    .zero();
            }
        }

        self.next_page_table_mut(index)
    }
}

impl<L: TableLevel> Index<usize> for PageTable<L> {
    type Output = PageTableEntry;

    fn index(&self, index: usize) -> &PageTableEntry {
        &self.entries[index]
    }
}

impl<L: TableLevel> IndexMut<usize> for PageTable<L> {
    fn index_mut(&mut self, index: usize) -> &mut PageTableEntry {
        &mut self.entries[index]
    }
}
