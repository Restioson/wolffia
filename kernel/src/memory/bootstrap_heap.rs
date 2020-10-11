//! A simple bitmap allocator used to allocate memory for the buddy allocator


use core::{mem, ptr::{self, NonNull}};
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use spin::{Once, Mutex};
use bit_field::BitField;
use friendly::Block;
use crate::memory::physical_allocator::PhysicalTree;

pub static BOOTSTRAP_HEAP: BootstrapHeap = BootstrapHeap(Once::new());

/// A holding struct for the bootstrap heap.
pub struct BootstrapHeap(Once<BootstrapAllocator<[Block; PhysicalTree::total_blocks()]>>);

impl BootstrapHeap {
    /// Allocates a zeroed object. Panics if bootstrap heap is not initialized
    pub unsafe fn allocate(&self) -> Option<BootstrapHeapBox<[Block; PhysicalTree::total_blocks()]>> {
        self.0.wait().unwrap().allocate()
    }

    /// Initialises the bootstrap heap with a begin address.
    ///
    /// # Unsafety
    ///
    /// Unsafe if address is incorrect (not free memory)
    pub unsafe fn init_unchecked(&self, address: u64) {
        self.0.call_once(|| BootstrapAllocator::new_unchecked(address));
    }

    /// Get the start address of the bootstrap heap. Panics if uninitialized
    pub fn start(&self) -> u64 {
        self.0.wait().unwrap().start() as u64
    }

    /// Get the end address of the bootstrap heap. Inclusive. Panics if uninitialized
    pub fn end(&self) -> u64 {
        self.0.wait().unwrap().start() as u64 +
            BootstrapAllocator::<[Block; PhysicalTree::total_blocks()]>::space_taken()
    }

    pub const fn space_taken() -> u64 {
        BootstrapAllocator::<[Block; PhysicalTree::total_blocks()]>::space_taken() as u64
    }
}

/// A bitmap heap/physmem allocator to bootstrap the buddy allocator since it requires a
/// (relative to how much the stack should be used for) large amount of memory.
#[derive(Debug)]
pub struct BootstrapAllocator<T> {
    start_addr: u64,
    bitmap: Mutex<u8>,
    _phantom: PhantomData<T>,
}

impl<T> BootstrapAllocator<T> {
    pub const fn space_taken() -> u64 {
        mem::size_of::<T>() as u64 * 8
    }

    pub fn start(&self) -> *mut T {
        self.start_addr as *mut T
    }

    /// Create an allocator with a start address of `start`. UB if not enough space given to the
    /// allocator (could overwrite other memory) or if the start ptr is not well aligned.
    pub const fn new_unchecked(start: u64) -> Self {
        BootstrapAllocator {
            start_addr: start,
            bitmap: Mutex::new(0),
            _phantom: PhantomData,
        }
    }

    /// Set a block to used or not at an index
    #[inline]
    fn set_used(&self, index: usize, used: bool) {
        let bit = index % 8;
        self.bitmap.lock().set_bit(bit, used);
    }

    /// Allocate an object and return the address if there is space
    fn allocate(&self) -> Option<BootstrapHeapBox<T>> {
        for bit in 0..8 {
            let mut byte = self.bitmap.lock();

            if !byte.get_bit(bit) {
                byte.set_bit(bit, true);

                let ptr = unsafe {
                    NonNull::new_unchecked(self.start().offset((bit) as isize))
                };
                return Some(BootstrapHeapBox { ptr, allocator: self });
            }
        }

        None
    }

    /// Deallocate a heap box. Must be only called in the `Drop` impl of `BootstrapHeapBox`.
    fn deallocate(&self, obj: &BootstrapHeapBox<T>) {
        let addr_in_heap = obj.ptr.as_ptr() as u64 - self.start_addr;
        let index = addr_in_heap as usize / mem::size_of::<T>();

        self.set_used(index, false);
    }
}

pub struct BootstrapHeapBox<'a, T: 'a> {
    ptr: NonNull<T>,
    allocator: &'a BootstrapAllocator<T>,
}

impl<'a, T> PartialEq for BootstrapHeapBox<'a, T> {
    fn eq(&self, other: &Self) -> bool {
        ptr::eq(self.ptr.as_ptr() as *const _, other.ptr.as_ptr() as *const _)
    }
}

impl<'a, T> Eq for BootstrapHeapBox<'a, T> {}

impl<'a, T> Deref for BootstrapHeapBox<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a, T> DerefMut for BootstrapHeapBox<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.ptr.as_mut() }
    }
}

impl<'a, T> Drop for BootstrapHeapBox<'a, T> {
    fn drop(&mut self) {
        self.allocator.deallocate(self);
    }
}

unsafe impl<'a, T: Send> Send for BootstrapHeapBox<'a, T> {}
unsafe impl<'a, T: Sync> Sync for BootstrapHeapBox<'a, T> {}
