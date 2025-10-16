//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

#[allow(unused_variables)]
pub fn main() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let ref1 = unsafe { &mut *ptr_base };
    let ref2 = unsafe { &mut *ptr_base };
    let ref3 = unsafe { &mut *ptr_base };

    // both references get exposed
    let int1 = ref1 as *mut u32 as usize;
    let int2 = ref2 as *mut u32 as usize;

    let wild = int1 as *mut u32;

    //    ┌────────────┐
    //    │            │
    //    │  ptr_base  ├──────────────┬───────────────────┐
    //    │            │              │                   │
    //    └──────┬─────┘              │                   │
    //           │                    │                   │
    //           │                    │                   │
    //           ▼                    ▼                   ▼
    //    ┌────────────┐       ┌────────────┐       ┌───────────┐
    //    │            │       │            │       │           │
    //    │ ref1(Res)* │       │ ref2(Res)* │       │ ref3(Res) │
    //    │            │       │            │       │           │
    //    └────────────┘       └────────────┘       └───────────┘

    // disables ref1,ref2
    *ref3 = 13;

    // both exposed pointers are disabled so this fails
    let fail = unsafe { *wild }; //~ ERROR: /read access through .* is forbidden/
}
