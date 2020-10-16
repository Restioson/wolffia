//! All things to do with memory
//!
//! # Kernel memory map
//!
//! `.` denotes current memory location. Addresses at `.` will always be plus the align of the
//! structure.
//!
//! | Address range                             |  Usage                    |
//! |-------------------------------------------|---------------------------|
//! | `0xffffffff40000000` ~ . + 1GiB           | Kernel heap               |
//! | `0xffffffff800b8000` ~ . + `0x1000`       | VGA frame buffer          |
//! | `0xffffffff80100000` + 1MiB ~ kernel end  | Kernel elf                |
//! | . ~ . + size of bootstrap heap            | Bootstrap heap            |
//! | . ~ . + size of heap buddy allocator tree | Heap buddy allocator tree |
//! | . ~ . + 7 * size of stack                 | IST stacks                |

#[macro_use]
pub mod paging;
pub mod bootstrap_heap;
pub mod heap;
pub mod physical_allocator;
pub mod physical_mapping;
mod stack_allocator;

use self::bootstrap_heap::{BootstrapHeap, BOOTSTRAP_HEAP};
use self::paging::*;
use self::physical_allocator::PHYSICAL_ALLOCATOR;
use self::stack_allocator::StackAllocator;
use crate::memory::physical_allocator::PhysicalTree;
use crate::tss;
use crate::tss::Tss;
use crate::util::round_up_divide;
use core::{
    iter, mem,
    ops::{Range, RangeInclusive},
};
use friendly::Block;
use multiboot2::{self, BootInformation, MemoryMapTag};
use tinyvec::ArrayVec;
use x86_64::structures::tss::TaskStateSegment;
use x86_64::{PhysAddr, VirtAddr};

pub const KERNEL_MAPPING_BEGIN: u64 = 0xffffffff80000000;
const IST_STACK_SIZE_PAGES: u64 = 4;

pub fn init_memory(mb_info_addr: u64, guard_page_addr: u64) {
    info!("mem: initialising");

    let mb_info = unsafe { multiboot2::load(mb_info_addr as usize) };
    let kernel_area = kernel_area(&mb_info);

    let mb_info_phys = mb_info.start_address() as u64..=mb_info.end_address() as u64;
    let memory_map = mb_info
        .memory_map_tag()
        .expect("Expected a multiboot2 memory map tag, but it is not present!");

    print_memory_info(memory_map);

    debug!("mem: initialising bootstrap heap");
    let (bootstrap_heap_phys, bootstrap_heap_virtual) = unsafe {
        let physical_start = PhysAddr::new(*mb_info_phys.end() as u64 + 1); // TODO what if really high and no more space ?
        let virtual_start = VirtAddr::new(*kernel_area.end() as u64 + 1);

        setup_bootstrap_heap(virtual_start, physical_start)
    };

    debug!("mem: initialising pmm (1/2)");
    let (gibbibytes, usable) = unsafe {
        setup_physical_allocator_prelim(&mb_info, mb_info_phys, bootstrap_heap_phys, kernel_area)
    };

    // ** IMPORTANT! **
    // The heap must NOT BE USED except in one specific place -- all heap objects will be corrupted
    // after the remap.
    debug!("mem: setting up kernel heap");
    let heap_tree_start = bootstrap_heap_virtual.end() + 1;
    let heap_tree_start = unsafe { crate::HEAP.init(heap_tree_start) };
    let heap_tree_end = heap_tree_start + heap::Heap::tree_size() as u64;

    debug!("mem: initialising pmm (2/2)");
    unsafe { setup_physical_allocator_rest(gibbibytes, usable.iter()) };

    debug!("mem: remapping kernel");
    remap::remap_kernel(&mb_info, heap_tree_start);

    trace!("mem: setting up guard page");
    unsafe { setup_guard_page(guard_page_addr) };

    debug!("mem: setting up ist");
    let page = Page::containing_address(
        (round_up_divide(heap_tree_end as u64, 4096) * 4096) as u64,
        PageSize::Kib4,
    );

    unsafe { setup_ist(page) }

    info!("mem: initialised");
}

fn print_memory_info(memory_map: &MemoryMapTag) {
    trace!("mem: Usable memory areas: ");

    // For when log_level != debug | trace
    #[allow(unused_variables)]
    for area in memory_map.memory_areas() {
        trace!(
            " - 0x{:x} to 0x{:x}",
            area.start_address(),
            area.end_address()
        );
    }

    // Calculate how many GiBs are available
    let bytes_available: u64 = memory_map
        .memory_areas()
        .map(|area| (area.end_address() - area.start_address()) as u64)
        .sum();

    let gibbibytes_available = bytes_available as f64 / (1 << 30) as f64;
    if gibbibytes_available > 1.0 {
        info!("{:.3} GiB of RAM available", gibbibytes_available);
    } else {
        let mebbibytes_available = bytes_available as f64 / (1 << 20) as f64;
        info!("{:.3} MiB of RAM available", mebbibytes_available);
    }
}

unsafe fn setup_ist(begin: Page) {
    let mut allocator = StackAllocator::new(begin, 8, IST_STACK_SIZE_PAGES);

    // 7 for IST, 1 for syscalls
    let pages = IST_STACK_SIZE_PAGES * 8;

    for page in 0..pages {
        if page % IST_STACK_SIZE_PAGES == 0 {
            // Page is guard page: do not map
        } else {
            ACTIVE_PAGE_TABLES.lock().map(
                Page::containing_address(
                    begin.start_address().unwrap() + (page * 4096),
                    PageSize::Kib4,
                ),
                EntryFlags::WRITABLE | EntryFlags::NO_EXECUTE,
                InvalidateTlb::Invalidate,
                ZeroPage::Zero,
            );
        }
    }

    tss::TSS.call_once(|| {
        let mut tss = TaskStateSegment::new();

        let mut alloc = || {
            let stack_start = allocator.alloc().unwrap();
            stack_start as u64 + (IST_STACK_SIZE_PAGES * 4096) as u64
        };

        for i in 0..7 {
            // Packed struct; cannot safely borrow fields
            tss.interrupt_stack_table[i] = x86_64::VirtAddr::new(alloc());
        }

        tss.privilege_stack_table[0] = x86_64::VirtAddr::new(alloc());

        Tss::new(tss)
    });
}

/// Sets up the bootstrap heap and returns its physical address range and its virtual address range
/// (physical in the tuple first).
///
/// # Arguments
///
/// The addresses given are the smallest possible starting addresses.
unsafe fn setup_bootstrap_heap(
    virtual_start: VirtAddr,
    physical_start: PhysAddr,
) -> (RangeInclusive<u64>, RangeInclusive<u64>) {
    let start_ptr: *const u8 = virtual_start.as_ptr();
    let heap_start = start_ptr
        .add(start_ptr.align_offset(mem::align_of::<[Block; PhysicalTree::total_blocks()]>()))
        as u64;

    let start_page = Page::containing_address(heap_start, PageSize::Kib4) + 1;
    let start_frame = (physical_start.as_u64() / 4096) as u64 + 1;

    let mapping =
        PageRangeMapping::new(start_page, start_frame, BootstrapHeap::space_taken() / 4096);

    ACTIVE_PAGE_TABLES.lock().map_page_range(
        mapping,
        InvalidateTlb::NoInvalidate,
        EntryFlags::WRITABLE | EntryFlags::NO_EXECUTE,
    );

    let virtual_start = start_page.number() as u64 * 4096;

    BOOTSTRAP_HEAP.init_unchecked(virtual_start);

    let physical_start = start_frame as u64 * 4096;
    let virtual_start = start_page.number() as u64 * 4096;
    let physical = physical_start..=physical_start + BootstrapHeap::space_taken();
    let virtual_range = virtual_start..=virtual_start + BootstrapHeap::space_taken();

    (physical, virtual_range)
}

unsafe fn setup_physical_allocator_prelim(
    mb_info: &BootInformation,
    mb_info_phys: RangeInclusive<u64>,
    bootstrap_heap_phys: RangeInclusive<u64>,
    kernel_area: RangeInclusive<u64>,
) -> (u8, ArrayVec<[Range<u64>; 256]>) {
    let memory_map = mb_info
        .memory_map_tag()
        .expect("Expected a multiboot2 memory map tag, but it is not present!");

    let highest_address = memory_map
        .memory_areas()
        .map(|area| area.end_address() - 1)
        .max()
        .expect("No usable physical memory available!");

    // Do round-up division by 2^30 = 1GiB in bytes
    let trees = round_up_divide(highest_address as u64, 1 << 30) as u8;
    trace!("Allocating {} trees", trees);

    // Calculate the usable memory areas by using the MB2 memory map but excluding kernel areas
    let usable_areas = memory_map
        .memory_areas()
        .map(|area| (area.start_address(), area.end_address()))
        .map(|(start, end)| start..end);

    // Remove already used physical mem areas
    let kernel_area_phys = 0..=kernel_area.end() - KERNEL_MAPPING_BEGIN;

    let usable_areas = constant_unroll! { // Use this macro to make types work
        for used_area in [kernel_area_phys, mb_info_phys, bootstrap_heap_phys] {
            usable_areas = usable_areas.flat_map(move |free_area| {
                // Convert to Range from  RangeInclusive
                let range = *used_area.start()..*used_area.end() + 1;

                // HACK: arrays iterate with moving weirdly
                // Also, filter map to remove `None`s
                let [first, second] = range_sub(&free_area, &range);
                iter::once(first).chain(iter::once(second)).filter_map(|i| i)
            });
        }
    };

    // Collect into a large ArrayVec for performance
    let usable_areas = usable_areas.collect::<ArrayVec<[_; 256]>>();

    PHYSICAL_ALLOCATOR.init_prelim(usable_areas.iter());

    (trees, usable_areas)
}

unsafe fn setup_physical_allocator_rest<'a, I>(gibbibytes: u8, usable_areas: I)
where
    I: Iterator<Item = &'a Range<u64>> + Clone + 'a,
{
    PHYSICAL_ALLOCATOR.init_rest(gibbibytes, usable_areas);
}

unsafe fn setup_guard_page(addr: u64) {
    use self::paging::*;

    let page = Page::containing_address(addr, PageSize::Kib4);

    // Check it is a 4kib page
    let size = ACTIVE_PAGE_TABLES
        .lock()
        .walk_page_table(page)
        .expect("Guard page must be mapped!")
        .1;
    assert_eq!(size, PageSize::Kib4, "Guard page must be on a 4kib page!");

    ACTIVE_PAGE_TABLES
        .lock()
        .unmap(page, FreeMemory::NoFree, InvalidateTlb::Invalidate);
}

fn kernel_area(mb_info: &BootInformation) -> RangeInclusive<u64> {
    use multiboot2::ElfSectionFlags;

    let elf_sections = mb_info
        .elf_sections_tag()
        .expect("Expected a multiboot2 elf sections tag, but it is not present!");

    let used_areas = elf_sections
        .sections()
        .filter(|section| section.flags().contains(ElfSectionFlags::ALLOCATED))
        .map(|section| section.start_address()..section.end_address())
        .chain(
            mb_info
                .module_tags()
                .map(|section| section.start_address() as u64..section.end_address() as u64),
        );

    let begin = used_areas.clone().map(|range| range.start).min().unwrap() as u64;
    let end = used_areas.map(|range| range.end).max().unwrap() as u64;

    begin..=end
}

/// Subtracts one range from another, provided that start <= end in all cases
fn range_sub<T>(main: &Range<T>, sub: &Range<T>) -> [Option<Range<T>>; 2]
where
    T: Ord + Copy,
{
    if sub.start <= main.start {
        // Hole starts before range
        if sub.end < main.end {
            // Hole covers entire bottom section of range  -- only top section
            [None, Some(sub.end..main.end)]
        } else {
            // Hole covers entire range -- no range
            [None, None]
        }
    } else if sub.start < main.end {
        // Hole starts inside range
        if sub.end >= main.end {
            // Hole covers entire end section of range -- only bottom section
            [Some(main.start..sub.start), None]
        } else {
            // Hole divides range into two sections
            [Some(main.start..sub.start), Some(sub.end..main.end)]
        }
    } else {
        // Hole starts outside of range -- full range
        [Some(main.start..main.end), None]
    }
}
