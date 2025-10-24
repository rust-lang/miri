//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

#[allow(unused_variables)]
pub fn main() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let ref1 = unsafe { &mut *ptr_base };
    let ref2 = unsafe { &mut *ptr_base };

    let int1 = ref1 as *mut u32 as usize;
    let wild = int1 as *mut u32;

    // graph TD
    // ptr_base --> ref1(Res)* & ref2(Res)
    //
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

    // write thourgh the wildcard to the only exposed reference ref1
    // disabling ref2
    unsafe { wild.write(13) };

    let fail = *ref2; //~ ERROR: /read access through .* is forbidden/
}
