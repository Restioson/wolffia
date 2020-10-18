use crate::memory::paging::{EntryFlags, Page, ACTIVE_PAGE_TABLES};
use crate::memory::LAST_USABLE_PAGE;
use core::ptr::NonNull;
use core::{mem, slice};

/// Plain old data. Size must be a multiple of the alignment.
pub unsafe trait PlainOldData: Sized {
    /// Safely transmute from a byte slice to a byte slice of the type
    fn from_bytes(buf: &[u8]) -> &[Self];
}

unsafe impl PlainOldData for u8 {
    fn from_bytes(buf: &[u8]) -> &[u8] {
        buf
    }
}

pub enum InvalidBufferError {
    OverlapsKernelSpace,
    InvalidLen,
    Unaligned,
    Unmapped,
    Null,
}

pub struct BorrowedKernelBuffer<'a, T>(pub &'a [T]);

impl<'a, T: PlainOldData> BorrowedKernelBuffer<'a, T> {
    /// # Safety
    ///
    /// The current page tables must be of the same address space where the buffer comes from.
    pub unsafe fn try_from_user(
        ptr: Option<NonNull<u8>>,
        len: u64,
    ) -> Result<Self, InvalidBufferError> {
        let ptr = ptr.ok_or(InvalidBufferError::Null)?.as_ptr() as *const u8;

        if (ptr as usize) % mem::align_of::<T>() != 0 {
            return Err(InvalidBufferError::Unaligned);
        }

        if len == 0 || len > isize::MAX as u64 {
            return Err(InvalidBufferError::InvalidLen);
        }

        let added = (ptr as u64).checked_add(len * mem::size_of::<T>() as u64 - 1);
        let buffer_end_byte = match added {
            Some(end) if end < (LAST_USABLE_PAGE + 1).start_address().unwrap() => end,
            Some(_invalid_end) => return Err(InvalidBufferError::OverlapsKernelSpace),
            None => return Err(InvalidBufferError::InvalidLen),
        };

        // Split the buffer into its memory pages
        let page_begin = Page::containing_address(ptr as u64);
        let page_end = Page::containing_address(buffer_end_byte as u64);

        let all_mapped = (page_begin..=page_end)
            .map(|p| ACTIVE_PAGE_TABLES.lock().walk_page_table(p))
            .all(|opt| {
                opt.map(|(entry, _)| entry.flags().contains(EntryFlags::USER_ACCESSIBLE))
                    .unwrap_or(false)
            });

        if !all_mapped {
            return Err(InvalidBufferError::Unmapped);
        }

        let byte_slice = if all_mapped {
            // SAFETY: all memory is mapped and aligned.
            slice::from_raw_parts(ptr, len as usize)
        } else {
            return Err(InvalidBufferError::Unmapped);
        };

        Ok(BorrowedKernelBuffer(T::from_bytes(byte_slice)))
    }
}
