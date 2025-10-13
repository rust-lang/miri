//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

#[allow(unused_variables)]
pub fn main() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let ref1 = unsafe { &mut *ptr_base };
    let ref2 = unsafe { &mut *ptr_base };

    let int1 = ref1 as *mut u32 as usize;
    let wild = int1 as *mut u32;

    //    ┌────────────┐
    //    │            │
    //    │  ptr_base  ├───────────┐
    //    │            │           │
    //    └──────┬─────┘           │
    //           │                 │
    //           │                 │
    //           ▼                 ▼
    //    ┌────────────┐     ┌───────────┐
    //    │            │     │           │
    //    │ ref1(Res)* │     │ ref2(Res) │
    //    │            │     │           │
    //    └────────────┘     └───────────┘

    *ref2 = 13; //disables ref1

    // tries to do a wildcard access through the only exposed ref1, which is disabled
    let fail = unsafe { *wild }; //~ ERROR: /read access through .* is forbidden/
}
