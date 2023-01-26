extern "Rust" {
    fn miri_alloc_phys(phys_addr: usize, size: usize) -> *mut u8;
}

pub const PHYS_ADDR: usize = 0xB8000;

fn main() {
    unsafe {
        miri_alloc_phys(PHYS_ADDR, 2);
        miri_alloc_phys(PHYS_ADDR + 1, 2); //~ERROR: unsupported operation: trying to allocate physical memory 0xb8001..0xb8003, but 0xb8000..0xb8002 was already allocated
    }
}
