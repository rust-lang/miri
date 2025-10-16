//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

#[allow(unused_variables)]
pub fn main() {
    use std::alloc::Layout;
    let x = unsafe { std::alloc::alloc_zeroed(Layout::new::<u32>()) as *mut u32 };

    let ref1 = unsafe { &mut *x };
    let ref2 = unsafe { &mut *x };

    let int = ref1 as *mut u32 as usize;
    let wild = int as *mut u32;

    *ref2 = 14;
    unsafe { std::alloc::dealloc(wild as *mut u8, Layout::new::<u32>()) }; //~ ERROR: /deallocation through wildcard .* is forbidden/
}
