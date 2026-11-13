use std::alloc::{GlobalAlloc, Layout};
use std::cell::UnsafeCell;

const BLOCK_SIZE: usize = 32;

#[repr(C, align(32))] // Want to ensure alignment up until block size.
struct OneBlockAllocator {
    block: UnsafeCell<[u8; BLOCK_SIZE]>,
}

// set the global_allocator to an instance of allocator struct.
#[global_allocator]
static ALLOCATOR: OneBlockAllocator =
    OneBlockAllocator { block: UnsafeCell::new([0x55; BLOCK_SIZE]) };

unsafe impl Sync for OneBlockAllocator {}

// We implement the GlobalAlloc trait for our custom allocator.
unsafe impl GlobalAlloc for OneBlockAllocator {
    // gives out pointer to single block, no matter the size.
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        self.block.get().cast::<u8>()
    }
    // No actual dealloc implementation needed, static allocator
    // will be deallocated automatically.
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

fn main() {
    // Allocate memory using the global allocator.
    let layout = Layout::from_size_align(16, 16).unwrap();
    let ptr = unsafe { std::alloc::alloc(layout) };
    // Deallocate memory directly, should raise warning
    unsafe { ALLOCATOR.dealloc(ptr, layout) }
}
