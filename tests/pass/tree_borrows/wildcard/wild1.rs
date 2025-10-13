//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

pub fn main() {
    wildcard_parallel();
    wildcard_sequence();
    destructor();
    protector();
    returned_mut_is_usable();
}
#[allow(unused_variables)]
pub fn wildcard_parallel() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let ref1 = unsafe { &mut *ptr_base };
    let ref2 = unsafe { &mut *ptr_base };

    // both references get exposed
    let int1 = ref1 as *mut u32 as usize;
    let int2 = ref2 as *mut u32 as usize;

    let wild = int1 as *mut u32;

    // graph TD
    // ptr_base --> ref1(Res)* & ref2(Res)*
    //
    //   ┌────────────┐
    //   │            │
    //   │  ptr_base  ├────────────┐
    //   │            │            │
    //   └──────┬─────┘            │
    //          │                  │
    //          │                  │
    //          ▼                  ▼
    //   ┌────────────┐     ┌────────────┐
    //   │            │     │            │
    //   │ ref1(Res)* │     │ ref2(Res)* │
    //   │            │     │            │
    //   └────────────┘     └────────────┘

    // writes through either of the exposed references
    // we do not know which so we cannot disable the other
    unsafe { wild.write(13) };

    // reading through either of these references should be valid
    assert_eq!(*ref2, 13);
}

#[allow(unused_variables)]
pub fn wildcard_sequence() {
    let mut x: u32 = 42;

    let ref1 = &mut x;
    let int1 = ref1 as *mut u32 as usize;

    let ref2 = &mut *ref1;

    let ref3 = &mut *ref2;
    let int3 = ref3 as *mut u32 as usize;

    let wild = int1 as *mut u32;

    // graph TD
    // ref1(Res)* --> ref2(Res) --> ref3(Res)*
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
    //     │ ref3(Res)* │
    //     │            │
    //     └────────────┘

    // writes through either ref1 or ref3, which is either a child or foreign access to ref2.
    unsafe { wild.write(42) };

    //reading from ref2 still works since the previous access could have been through its child
    //this also freezes ref3
    let x = *ref2;

    // we can still write through wild, as there is still the exposed ref1 with write permissions
    unsafe { wild.write(43) };
}

fn destructor() {
    use std::alloc::Layout;
    let x = unsafe { std::alloc::alloc_zeroed(Layout::new::<u32>()) as *mut u32 };
    let ref1 = unsafe { &mut *x };
    let int = ref1 as *mut u32 as usize;
    let wild = int as *mut u32;
    unsafe { std::alloc::dealloc(wild as *mut u8, Layout::new::<u32>()) };
}

fn protector() {
    fn protect(arg: &mut u32) {
        *arg = 4;
    }
    let mut x: u32 = 32;
    let ref1 = &mut x;
    let int = ref1 as *mut u32 as usize;
    let wild = int as *mut u32;
    let wild_ref = unsafe { &mut *wild };

    protect(wild_ref);

    assert_eq!(*ref1, 4);
}

// analogous to same test in `../tree-borrows.rs` but with a protected wildcard pointer
fn returned_mut_is_usable() {
    // NOTE: currently we ignore protectors on wildcard references
    fn reborrow(x: &mut u8) -> &mut u8 {
        let y = &mut *x;
        // Activate the reference so that it is vulnerable to foreign reads.
        *y = *y;
        y
        // An implicit read through `x` is inserted here.
    }
    let mut x: u8 = 0;
    let ref1 = &mut x;
    let int = ref1 as *mut u8 as usize;
    let wild = int as *mut u8;
    let wild_ref = unsafe { &mut *wild };
    let y = reborrow(wild_ref);
    *y = 1;
}
