use crate::memory::paging::Page;

/// A bump allocator for kernel stacks. There is no guard page.
pub struct StackAllocator {
    base: Page,
    capacity: u64,
    /// Stack size in 4kib pages
    stack_size_pages: u64,
    current: u64,
}

impl StackAllocator {
    pub fn new(base: Page, capacity: u64, stack_size: u64) -> StackAllocator {
        base.start_address().expect("Page requires size");

        StackAllocator {
            base,
            capacity,
            stack_size_pages: stack_size,
            current: 0,
        }
    }

    pub fn alloc(&mut self) -> Option<*const u8> {
        if self.current >= self.capacity {
            return None;
        }

        let addr = self.base.start_address().unwrap() + (self.current * (self.stack_size_pages << 12));
        self.current += 1;

        Some(addr as *const u8)
    }
}
