//@compile-flags: -Zmiri-tree-borrows

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

    // disables ref3 as both exposed pointers are foreign to it
    unsafe { wild.write(13) };

    // fails because ref3 is disabled
    let fail = *ref3; //~ ERROR: /read access through .* is forbidden/
}
