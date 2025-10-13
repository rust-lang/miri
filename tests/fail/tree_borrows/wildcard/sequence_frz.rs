//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

#[allow(unused_variables)]
pub fn main() {
    let mut x: u32 = 42;

    let ref1 = &mut x;
    let int1 = ref1 as *mut u32 as usize;

    let ref2 = &mut *ref1;

    let ref3 = &*ref2;
    let int3 = ref3 as *const u32 as usize;

    let wild = int1 as *mut u32;

    // graph TD
    // ref1(Res)* --> ref2(Res) --> ref3(Frz)*
    //
    //     ┌────────────┐
    //     │            │
    //     │ ref1(Res)* │
    //     │            │
    //     └──────┬─────┘
    //            │
    //            │
    //            ▼
    //     ┌────────────┐
    //     │            │
    //     │ ref2(Res)  │
    //     │            │
    //     └──────┬─────┘
    //            │
    //            │
    //            ▼
    //     ┌────────────┐
    //     │            │
    //     │ ref3(Frz)* │
    //     │            │
    //     └────────────┘

    // writes through ref1. we cannot write through ref3 as its frozen
    // disables ref2, ref3
    unsafe { wild.write(42) };

    // ref2 is disabled
    let fail = *ref2; //~ ERROR: /read access through .* is forbidden/
}
