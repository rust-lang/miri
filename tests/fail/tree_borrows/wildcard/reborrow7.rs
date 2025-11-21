//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

pub fn main() {
    let mut x: u32 = 42;

    let ref_base = &mut x;

    let int = ref_base as *mut u32 as usize;
    let wild = int as *mut u32;

    let reb = unsafe { &mut *wild };
    let ptr_reb = reb as *mut u32;
    let ref1 = unsafe { &mut *ptr_reb };
    let _int1 = ref1 as *mut u32 as usize;
    let ref2 = unsafe { &mut *ptr_reb };

    //    ┌──────────────┐
    //    │              │
    //    │ptr_base(Res)*│         *
    //    │              │         │
    //    └──────────────┘         │
    //                             │
    //                             │
    //                             ▼
    //                       ┌────────────┐
    //                       │            │
    //                       │  reb(Res)  ├───────────┐
    //                       │            │           │
    //                       └──────┬─────┘           │
    //                              │                 │
    //                              │                 │
    //                              ▼                 ▼
    //                       ┌────────────┐     ┌───────────┐
    //                       │            │     │           │
    //                       │ ref1(Res)* │     │ ref2(Res) │
    //                       │            │     │           │
    //                       └────────────┘     └───────────┘

    unsafe { *wild = 13 };

    let _fail = *ref2; //~ ERROR: /read access through .* is forbidden/
}
