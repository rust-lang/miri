//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance
#[path = "../../../utils/mod.rs"]
#[macro_use]
mod utils;

#[allow(unused_variables)]
pub fn main() {
    let mut x: u32 = 0;

    let ref1 = &mut x;
    let alloc_id = unsafe { alloc_id!(ref1 as *mut u32) };

    let int = ref1 as *mut u32 as usize;
    let wild = int as *mut u32;

    // activates ref1
    unsafe { wild.write(41) };

    // needs to be created after the write, because otherwise it gets disabled
    // causes implicit read, disabling sibling ref1
    let ref2 = &x;

    unsafe { wild.write(0) }; //~ ERROR: /write access through wildcard at .* is forbidden/
}
