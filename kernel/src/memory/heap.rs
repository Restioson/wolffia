/// The base heap address.
pub const HEAP_START: u64 = 0xffffffff40000000;

use crate::memory::paging::*;
use crate::util;
use core::alloc::{GlobalAlloc, Layout};
use core::{iter, mem, ptr};
use friendly::{Block, Tree};
use spin::{Mutex, Once};
use x86_64::PhysAddr;

pub const BASE_ORDER: u8 = 6;
const BLOCKS_IN_TREE: usize = friendly::blocks_in_tree(25);
type RawArray = [Block; BLOCKS_IN_TREE];
pub type HeapTree = Tree<&'static mut RawArray, 25, BASE_ORDER>;

pub struct Heap {
    tree: Once<Mutex<HeapTree>>,
}

impl Heap {
    pub const fn new() -> Self {
        Heap { tree: Once::new() }
    }

    /// Initializes the heap. Required for it to be usable, otherwise all of its methods will panic.
    ///
    /// # Safety
    ///
    /// Safe if `heap_tree_start` is correct (unused) and well-aligned (currently always true as
    /// Block is a u8 and `repr(transparent)`.
    pub unsafe fn init(&self, heap_tree_start: u64) -> u64 {
        self.tree.call_once(|| {
            // Get the next page up from the given heap start
            let heap_tree_start = ((heap_tree_start / 4096) + 1) * 4096;

            // Map pages for the tree to use for accounting info
            let tree_size_pages = mem::size_of::<[Block; BLOCKS_IN_TREE]>() / 4096;

            let page_start = Page::containing_address(heap_tree_start);
            let page_end = page_start + tree_size_pages;

            ACTIVE_PAGE_TABLES.lock().map_range(
                page_start..=page_end,
                EntryFlags::WRITABLE | EntryFlags::NO_EXECUTE | EntryFlags::GLOBAL,
                InvalidateTlb::Invalidate,
                ZeroPage::Zero,
            );

            let tree = HeapTree::new(
                iter::once(0..(1 << (30 + 1))),
                // Safety: zero initialised, unique, and lasts the entire program.
                &mut *(heap_tree_start as *mut _),
            );

            Mutex::new(tree)
        });

        ((heap_tree_start / 4096) + 1) * 4096
    }

    /// Allocate a block of minimum size of 4096 bytes (rounded to this if smaller) with specific
    /// requirements about where it is to be placed in physical memory.
    ///
    /// Note: `physical_begin_frame` is the frame number of the beginning physical frame to allocate
    /// memory from (i.e address / 4096).
    ///
    /// # Panicking
    ///
    /// Panics if the heap is not initialized.
    ///
    /// # Unsafety
    ///
    /// Unsafe as it remaps pages, which could cause memory unsafety if the heap is not set up
    /// correctly.
    pub unsafe fn alloc_specific(&self, physical_begin_frame: u64, frames: u64) -> *mut u8 {
        let mut tree = self.tree.wait().expect("Heap not initialized!").lock();

        let order = order(frames * 4096);
        if order > HeapTree::max_order() {
            return ptr::null_mut();
        }

        let ptr = tree.allocate(order);

        if ptr.is_none() {
            return ptr::null_mut();
        }

        let ptr = (ptr.unwrap() as u64 + HEAP_START) as *mut u8;

        // Map pages that must be mapped
        for page in 0..util::round_up_divide(1u64 << (order + BASE_ORDER), 4096) as u64 {
            let page_addr = ptr as u64 + (page * 4096);
            ACTIVE_PAGE_TABLES.lock().map_to(
                Page::containing_address(page_addr),
                PhysAddr::new((physical_begin_frame + page) * 4096),
                EntryFlags::WRITABLE | EntryFlags::NO_EXECUTE | EntryFlags::GLOBAL,
                InvalidateTlb::Invalidate,
            );
        }

        ptr
    }

    /// The `dealloc` counterpart to `alloc_specific`. This function does not free the backing
    /// physical memory.
    ///
    /// # Panicking
    ///
    /// Panics if the heap is not initialized.
    ///
    /// # Unsafety
    ///
    /// Unsafe as it unmaps pages, which could cause memory unsafety if the heap is not set up
    /// correctly.
    pub unsafe fn dealloc_specific(&self, ptr: *mut u8, frames: u64) {
        if ptr.is_null() || frames == 0 {
            return;
        }

        let order = order(frames * 4096);

        assert!(
            ptr as u64 >= HEAP_START && (ptr as u64) < (HEAP_START + (1 << 30)),
            "Heap object {:?} pointer not in heap!",
            ptr,
        );

        let global_ptr = ptr;
        let ptr = ptr as usize - HEAP_START as usize;

        self.tree
            .wait()
            .expect("Heap not initialized!")
            .lock()
            .deallocate(ptr, order);

        // Unmap pages that have were used for this alloc
        for page in 0..util::round_up_divide(1u64 << (order + BASE_ORDER), 4096) as u64 {
            let page_addr = global_ptr as u64 + (page * 4096);

            ACTIVE_PAGE_TABLES.lock().unmap(
                Page::containing_address(page_addr),
                FreeMemory::NoFree,
                InvalidateTlb::NoInvalidate,
            );
        }
    }

    pub const fn tree_size() -> usize {
        mem::size_of::<RawArray>()
    }
}

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut tree = self.tree.wait().expect("Heap not initialized!").lock();

        let order = order(layout.size() as u64);
        if order > HeapTree::max_order() {
            return ptr::null_mut();
        }

        let ptr = tree.allocate(order);
        if ptr.is_none() {
            return ptr::null_mut();
        }
        let ptr = (ptr.unwrap() as u64 + HEAP_START) as *mut u8;

        // Map pages that have yet to be mapped
        for page in 0..util::round_up_divide(1u64 << (order + BASE_ORDER - 1), 4096) as u64 {
            let mut page_tables = ACTIVE_PAGE_TABLES.lock();

            let page_addr = ptr as u64 + (page * 4096);

            let mapped = page_tables
                .walk_page_table(Page::containing_address(page_addr))
                .is_some();

            if !mapped {
                page_tables.map(
                    Page::containing_address(page_addr),
                    EntryFlags::WRITABLE | EntryFlags::NO_EXECUTE | EntryFlags::GLOBAL,
                    InvalidateTlb::NoInvalidate,
                    ZeroPage::Zero,
                );
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }

        let order = order(layout.size() as u64);

        assert!(
            ptr as u64 >= HEAP_START && (ptr as u64) < (HEAP_START + (1 << 30)),
            "Heap object {:?} pointer not in heap!",
            ptr,
        );

        let global_ptr = ptr;
        let ptr = ptr as u64 - HEAP_START;

        self.tree
            .wait()
            .expect("Heap not initialized!")
            .lock()
            .deallocate(ptr as usize, order);

        let page_order = 12 - BASE_ORDER; // log2(4096) - base order

        // There will only be pages to unmap which totally contained this allocation if this
        // allocation was larger or equal to the size of a page
        if order < page_order {
            // Else, we must check if it happened to be on its own page

            let page_base_ptr = ptr & !0xFFF;

            let level = HeapTree::max_order() - page_order;
            let level_offset = friendly::blocks_in_tree(level);
            let index = level_offset + (page_base_ptr >> (page_order + BASE_ORDER)) as usize + 1;
            let order_free = self.tree.wait().unwrap().lock().block(index - 1).order_free;

            if order_free == page_order + 1 {
                let global_ptr = page_base_ptr + HEAP_START;

                ACTIVE_PAGE_TABLES.lock().unmap(
                    Page::containing_address(global_ptr),
                    FreeMemory::Free,
                    InvalidateTlb::Invalidate,
                );
            }
        } else {
            // Unmap pages that have were only used for this alloc
            for page in 0..util::round_up_divide(1u64 << (order + BASE_ORDER - 1), 4096) as u64 {
                let page_addr = global_ptr as u64 + (page * 4096);

                ACTIVE_PAGE_TABLES.lock().unmap(
                    Page::containing_address(page_addr),
                    FreeMemory::Free,
                    InvalidateTlb::Invalidate,
                );
            }
        }
    }
}

/// Converts log2 to order (NOT minus 1)
fn order(val: u64) -> u8 {
    if val == 0 {
        return 0;
    }

    let log2 = log2_ceil(val as u64) + 1;

    if log2 > BASE_ORDER {
        log2 - BASE_ORDER
    } else {
        0
    }
}

fn log2_ceil(val: u64) -> u8 {
    let log2 = log2_floor(val);
    if val != (1u64 << log2) {
        log2 + 1
    } else {
        log2
    }
}

fn log2_floor(mut val: u64) -> u8 {
    let mut log2 = 0;
    while val > 1 {
        val >>= 1;
        log2 += 1;
    }
    log2 as u8
}
