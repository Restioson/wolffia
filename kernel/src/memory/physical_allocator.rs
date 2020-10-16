use super::bootstrap_heap::{BootstrapHeapBox, BOOTSTRAP_HEAP};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::convert::TryInto;
use core::{
    iter,
    ops::{Deref, DerefMut, Range},
};
use friendly::{Block, Tree};
use spin::{Mutex, Once};
use x86_64::structures::paging::PhysFrame;
use x86_64::PhysAddr;

/// Number of orders.
const LEVEL_COUNT: u8 = 19;
/// The base order size. All orders are in context of this -- i.e the size of a block of order `k`
/// is `2^(k + MIN_ORDER)`, not `2^k`.
const BASE_ORDER: u8 = 12;

/// The physical frame allocator. Requires the bootstrap heap to be initialized, or else the
/// initializer will panic.
pub static PHYSICAL_ALLOCATOR: PhysicalAllocator<'static> =
    PhysicalAllocator { trees: Once::new() };

pub type PhysicalTree<'a> = Tree<TreeBox<'a>, LEVEL_COUNT, BASE_ORDER>;

pub struct PhysicalAllocator<'a> {
    // Max 256GiB
    trees: Once<[Mutex<Option<PhysicalTree<'a>>>; 256]>,
}

impl<'a> PhysicalAllocator<'a> {
    /// Initialize the allocator's first 8 gibbibytes. The PMM has a two stage init -- in the first
    /// stage, the first 8 GiBs are set up, using the bootstrap heap. This is enough to set up the
    /// main kernel heap. In the second stage, the rest of the GiBs are set up, using the kernel
    /// heap.
    pub fn init_prelim<'r, I>(&self, usable: I)
    where
        I: Iterator<Item = &'r Range<u64>> + Clone + 'r,
    {
        self.trees.call_once(|| {
            let mut trees: [Mutex<Option<PhysicalTree<'a>>>; 256] =
                array_init::array_init(|_| Mutex::new(None));

            // Init the first 8 trees on the bootstrap heap
            for (i, slot) in trees.iter_mut().take(8).enumerate() {
                let usable = Self::localize(i as u8, usable.clone());

                let tree = PhysicalTree::new(
                    usable,
                    TreeBox::Bootstrap(unsafe {
                        BOOTSTRAP_HEAP
                            .allocate()
                            .expect("Ran out of bootstrap heap memory!")
                    }),
                );

                *slot = Mutex::new(Some(tree));
            }

            trees
        });
    }

    /// Initialise the rest of the allocator's gibbibytes. See [PhysicalAllocator.init_prelim].
    pub fn init_rest<'r, I>(&self, gibbibytes: u8, usable: I)
    where
        I: Iterator<Item = &'r Range<u64>> + Clone + 'r,
    {
        let trees = self.trees.wait().unwrap();

        for i in 8..=gibbibytes {
            let usable = Self::localize(i as u8, usable.clone());

            let blocks = iter::repeat(Block::new_used())
                .take(PhysicalTree::total_blocks())
                .collect::<Vec<Block>>()
                .into_boxed_slice()
                .try_into()
                .map_err(|_| unreachable!())
                .unwrap();

            let tree = PhysicalTree::new(usable, TreeBox::Heap(blocks));
            *trees[i as usize].lock() = Some(tree);
        }
    }

    /// Filter out addresses that apply to a GiB and make them local to it
    fn localize<'r, I>(gib: u8, usable: I) -> impl Iterator<Item = Range<usize>> + Clone + 'r
    where
        I: Iterator<Item = &'r Range<u64>> + Clone + 'r,
    {
        (&usable).clone().filter_map(move |range| {
            let gib = ((gib as usize) << 30)..(((gib as usize + 1) << 30) + 1);

            // If the range covers any portion of the GiB
            if range.start as usize <= gib.end && (range.end as usize) >= gib.start {
                let end = range.end as usize - gib.start;
                let begin = if range.start as usize >= gib.start {
                    range.start as usize - gib.start // Begin is within this GiB
                } else {
                    0 // Begin is earlier than this GiB
                };

                Some(begin..end)
            } else {
                None
            }
        })
    }

    /// Allocate a frame of order `order`. Panics if not initialized. Does __not__ zero the memory.
    pub fn allocate(&self, order: u8) -> Option<PhysFrame> {
        #[derive(Eq, PartialEq, Copy, Clone, Debug)]
        enum TryState {
            Tried,
            WasInUse,
            Untried,
        }

        let mut tried = [TryState::Untried; 256];

        // Try every tree. If it's locked, come back to it later.
        loop {
            let index = tried
                .iter()
                .position(|i| *i == TryState::Untried)
                .or_else(|| tried.iter().position(|i| *i == TryState::WasInUse))?;

            let trees = self.trees.wait().unwrap();

            // Try to lock the tree
            if let Some(ref mut tree) = trees[index].try_lock() {
                // Get Option<&mut Tree>
                if let Some(ref mut tree) = tree.as_mut() {
                    // Try to allocate something on the tree
                    match tree.allocate(order) {
                        Some(address) => {
                            let addr =
                                address + (index * (1 << (PhysicalTree::max_order() + BASE_ORDER)));
                            return Some(PhysFrame::containing_address(PhysAddr::new(addr as u64)));
                        }
                        None => tried[index] = TryState::Tried, // Tree empty for alloc of this size
                    }
                } else {
                    // Tree was None and nonexistent. We've tried it so set it to tried
                    tried[index] = TryState::Tried;
                }
            } else {
                // Tree was already locked -- it is busy and in use by something else (in futuure,
                // another core)
                tried[index] = TryState::WasInUse;
            }
        }
    }

    /// Deallocate the block of `order` at `frame_addr`. Panics if not initialized, if block is free,
    /// or if block is out of bounds of the # of GiB available.
    pub fn deallocate(&self, frame_addr: u64, order: u8) {
        let tree = (frame_addr as usize) >> (LEVEL_COUNT - 1 + BASE_ORDER);
        let local_ptr = (frame_addr % (1 << (LEVEL_COUNT - 1 + BASE_ORDER))) as *const u8;

        let trees = self.trees.wait().unwrap();
        let mut lock = trees[tree].lock();
        let tree = lock.as_mut().unwrap();

        tree.deallocate(local_ptr as usize, order);
    }
}

type RawArray = [Block; friendly::blocks_in_tree(LEVEL_COUNT)];

pub enum TreeBox<'a> {
    Bootstrap(BootstrapHeapBox<'a, RawArray>),
    Heap(Box<RawArray>),
}

impl<'a> Deref for TreeBox<'a> {
    type Target = RawArray;

    fn deref(&self) -> &RawArray {
        use self::TreeBox::*;
        match self {
            Bootstrap(tree_box) => tree_box,
            Heap(tree_box) => tree_box,
        }
    }
}

impl<'a> DerefMut for TreeBox<'a> {
    fn deref_mut(&mut self) -> &mut RawArray {
        use self::TreeBox::*;
        match self {
            Bootstrap(tree_box) => tree_box,
            Heap(tree_box) => tree_box,
        }
    }
}
