//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

pub fn main() {
    let mut x: u32 = 42;

    let ref_base = &mut x;

    let int0 = ref_base as *mut u32 as usize;
    let wild = int0 as *mut u32;

    let reb1 = unsafe { &mut *wild };
    let ref1 = &mut *reb1;
    let int1 = ref1 as *mut u32 as usize;
    let wild = int1 as *mut u32;

    let reb2 = unsafe { &mut *wild };
    //    ┌──────────────┐
    //    │              │
    //    │ptr_base(Res)*│         *                 *
    //    │              │         │                 │
    //    └──────────────┘         │                 │
    //                             │                 │
    //                             │                 │
    //                             ▼                 ▼
    //                       ┌────────────┐    ┌────────────┐
    //                       │            │    │            │
    //                       │ reb1(Res)  ├    │ reb2(Res)  ├
    //                       │            │    │            │
    //                       └──────┬─────┘    └────────────┘
    //                              │
    //                              │
    //                              ▼
    //                       ┌────────────┐
    //                       │            │
    //                       │ ref1(Res)* │
    //                       │            │
    //                       └────────────┘

    *ref1 = 13;

    let _fail = *reb2; //~ ERROR: /read access through .* is forbidden/
}
