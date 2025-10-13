//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

#[allow(unused_variables)]
pub fn main() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let ref1 = unsafe { &mut *ptr_base };
    let ref2 = unsafe { &*ptr_base };

    // both references get exposed
    let int1 = ref1 as *mut u32 as usize;
    let int2 = ref2 as *const u32 as usize;

    let wild = int1 as *mut u32;

    //    ┌────────────┐
    //    │            │
    //    │  ptr_base  ├──────────────┐
    //    │            │              │
    //    └──────┬─────┘              │
    //           │                    │
    //           │                    │
    //           ▼                    ▼
    //    ┌────────────┐       ┌────────────┐
    //    │            │       │            │
    //    │ ref1(Res)* │       │ ref2(Frz)* │
    //    │            │       │            │
    //    └────────────┘       └────────────┘

    // disables ref2 as the only write could happen through ref1
    unsafe { wild.write(13) };

    // fails because ref2 is disabled
    let fail = *ref2; //~ ERROR: /read access through .* is forbidden/
}
