//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

pub fn main() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let ref1 = unsafe { &mut *ptr_base };

    *ref1 = 4;

    let int1 = ref1 as *mut u32 as usize;
    let wild = int1 as *mut u32;

    let ref2 = unsafe { &mut *wild };

    let ref3 = unsafe { &mut *ptr_base };
    let _int3 = ref3 as *mut u32 as usize;

    //    ┌──────────────┐
    //    │              │
    //    │ptr_base(Act) ├───────────┐                  *
    //    │              │           │                  │
    //    └──────┬───────┘           │                  │
    //           │                   │                  │
    //           │                   │                  │
    //           ▼                   ▼                  ▼
    //     ┌─────────────┐     ┌────────────┐     ┌───────────┐
    //     │             │     │            │     │           │
    //     │ ref1(Frz)*  │     │ ref3(Res)* │     │ ref2(Res) │
    //     │             │     │            │     │           │
    //     └─────────────┘     └────────────┘     └───────────┘

    *ref2 = 13; //~ ERROR: /write access through .* is forbidden/
}
