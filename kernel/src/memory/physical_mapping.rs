use core::{mem, ptr::NonNull, ops::Deref};
use crate::util;
use crate::acpi_handler::WolffiaAcpiHandler;

pub unsafe fn map_physical_region<T>(
    physical_address: u64,
    size: u64,
    mutable: bool
) -> PhysicalMapping<T> {
    let frames = util::round_up_divide(size as u64, 4096) as u64;
    let physical_begin_frame = physical_address / 4096;

    let alloc_ptr = crate::HEAP.alloc_specific(physical_begin_frame, frames) as u64;

    if alloc_ptr == 0 {
        panic!("Ran out of heap memory!");
    }

    let obj_ptr = alloc_ptr + physical_address - (physical_begin_frame * 4096);

    PhysicalMapping {
        physical_start: physical_begin_frame * 4096,
        // alloc_ptr is zero if there is no more heap memory available
        virtual_start: NonNull::new(obj_ptr as *mut T)
            .expect("Ran out of heap memory!"),
        mapped_length: frames * 4096,
        mutable,
    }
}

pub unsafe fn map_physical_type<T>(physical_address: u64, mutable: bool) -> PhysicalMapping<T> {
    map_physical_region(physical_address, mem::size_of::<T>() as u64, mutable)
}

pub struct PhysicalMapping<T> {
    physical_start: u64,
    virtual_start: NonNull<T>,
    mapped_length: u64,
    mutable: bool,
}

impl<T> PhysicalMapping<T> {
    pub fn into_acpi(self, handler: WolffiaAcpiHandler) -> acpi::PhysicalMapping<WolffiaAcpiHandler, T> {
        let mapping = acpi::PhysicalMapping {
            physical_start: self.physical_start as usize,
            virtual_start: self.virtual_start,
            region_length: self.mapped_length as usize,
            mapped_length: self.mapped_length as usize,
            handler,
        };
        mem::forget(self); // prevent drop
        mapping
    }

    /// Returns a mutable reference to the data if this mapping is mutable and returns None if not
    /// mutable.
    pub fn deref_mut(&mut self) -> Option<&mut T> {
        if self.mutable {
            Some(unsafe { self.virtual_start.as_mut() })
        } else {
            None
        }
    }
}

impl<T> Drop for PhysicalMapping<T> {
    fn drop(&mut self) {
        let obj_addr = self.virtual_start.as_ptr() as *mut T as u64;

        // Clear lower page offset bits
        let page_begin = obj_addr & !0xFFF;

        unsafe {
            crate::HEAP.dealloc_specific(
                page_begin as *mut u8,
                self.mapped_length / 4096,
            );
        }
    }
}

impl<T> Deref for PhysicalMapping<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { self.virtual_start.as_ref() }
    }
}
