use std::alloc::{GlobalAlloc, Layout};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

const BLOCK_SIZE: usize = 32;

#[repr(C, align(32))] // Want to ensure alignment up until block size.
struct SimpleAllocator {
    block: UnsafeCell<[u8; BLOCK_SIZE]>,
    cnt: AtomicUsize,
}

// set the global_allocator to an instance of allocator struct.
#[global_allocator]
static ALLOCATOR: SimpleAllocator =
    SimpleAllocator { block: UnsafeCell::new([0x55; BLOCK_SIZE]), cnt: AtomicUsize::new(0) };

unsafe impl Sync for SimpleAllocator {}

// We implement the GlobalAlloc trait for our custom allocator.
unsafe impl GlobalAlloc for SimpleAllocator {
    // gives out pointer to first 16 bytes the first time, then the lsat the second time.
    // Also tries to read blocked memory on the second allocation, which is UB.
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        let cnt = self.cnt.fetch_add(1, Ordering::Relaxed);
        unsafe {
            eprintln!("Potentially reading from blocked memory: {:?}", *self.block.get());
            self.block.get().cast::<u8>().add(cnt * 16)
        }
    }
    // No actual dealloc implementation needed, static allocator
    // will be deallocated automatically.
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

fn main() {
    // Create layout for allocations.
    let layout = Layout::from_size_align(16, 16).unwrap();
    let ptr = unsafe { std::alloc::alloc(layout) };
    // Allocate second allocation, leads to UB
    let ptr_ub = unsafe { std::alloc::alloc(layout) };
    unsafe { std::alloc::dealloc(ptr, layout) };
    unsafe { std::alloc::dealloc(ptr_ub, layout) };
}
