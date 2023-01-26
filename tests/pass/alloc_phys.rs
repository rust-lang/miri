extern "Rust" {
    fn miri_alloc_phys(phys_addr: usize, size: usize) -> *mut u8;
}

pub const PHYS_ADDR: usize = 0xB8000;

fn main() {
    unsafe {
        let ptr = miri_alloc_phys(PHYS_ADDR, 2);
        ptr.write_volatile(0u8);
        ptr.add(1).write_volatile(0u8);
    }
}
