//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

pub fn main() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let ref1 = unsafe { &mut *ptr_base };
    let ref2 = unsafe { &mut *ptr_base };

    let int1 = ref1 as *mut u32 as usize;
    let wild = int1 as *mut u32;

    let reb = unsafe { &mut *wild };

    //    ┌────────────┐
    //    │            │
    //    │  ptr_base  ├───────────┐                 *
    //    │            │           │                 │
    //    └──────┬─────┘           │                 │
    //           │                 │                 │
    //           │                 │                 │
    //           ▼                 ▼                 ▼
    //    ┌────────────┐     ┌───────────┐     ┌───────────┐
    //    │            │     │           │     │           │
    //    │ ref1(Res)* │     │ ref2(Res) │     │ reb(Res)  │
    //    │            │     │           │     │           │
    //    └────────────┘     └───────────┘     └───────────┘

    *ref2 = 13;

    let _fail = *reb; //~ ERROR: /read access through .* is forbidden/
}
