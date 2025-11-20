//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

pub fn main() {
    multiple_exposed_siblings1();
    multiple_exposed_siblings2();
    reborrow3();
    returned_mut_is_usable();
}

/// Checks that accessing through a reborrowed wildcard doesn't
/// disable any exposed reference.
fn multiple_exposed_siblings1() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;

    let ref1 = unsafe { &mut *ptr_base };
    let int1 = ref1 as *mut u32 as usize;

    let ref2 = unsafe { &mut *ptr_base };
    let _int2 = ref2 as *mut u32 as usize;

    let wild = int1 as *mut u32;

    let reb = unsafe { &mut *wild };

    //   ┌────────────┐
    //   │            │
    //   │  ptr_base  ├────────────┐                 *
    //   │            │            │                 │
    //   └──────┬─────┘            │                 │
    //          │                  │                 │
    //          │                  │                 │
    //          ▼                  ▼                 ▼
    //   ┌────────────┐     ┌────────────┐    ┌────────────┐
    //   │            │     │            │    │            │
    //   │ ref1(Res)* │     │ ref2(Res)* │    │  reb(Res)  │
    //   │            │     │            │    │            │
    //   └────────────┘     └────────────┘    └────────────┘

    // Could either have as a parent ref1 or ref2.
    // So we can't disable either of them.
    *reb = 13;

    // We can still access both ref1, ref2.
    assert_eq!(*ref2, 13);
}

/// Checks that wildcard accesses do not invalidate any exposed
/// nodes through which the access could have happened.
/// It checks this for the case where some reborrowed wildcard
/// pointers are exposed as well.
fn multiple_exposed_siblings2() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let int = ptr_base as usize;

    let wild = int as *mut u32;

    let reb_ptr = unsafe { &mut *wild } as *mut u32;

    let ref1 = unsafe { &mut *reb_ptr };
    let _int1 = ref1 as *mut u32 as usize;

    let ref2 = unsafe { &mut *reb_ptr };
    let _int2 = ref2 as *mut u32 as usize;

    //   ┌────────────┐
    //   │            │
    //   │ ptr_base*  │            *
    //   │            │            │
    //   └────────────┘            │
    //                             │
    //                             │
    //                             ▼
    //                      ┌────────────┐
    //                      │            │
    //                      │    reb     ├────────────┐
    //                      │            │            │
    //                      └──────┬─────┘            │
    //                             │                  │
    //                             │                  │
    //                             ▼                  ▼
    //                      ┌────────────┐     ┌────────────┐
    //                      │            │     │            │
    //                      │ ref1(Res)* │     │ ref2(Res)* │
    //                      │            │     │            │
    //                      └────────────┘     └────────────┘

    // Writes either through ref1, ref2 or ptr_base, which are all exposed.
    // Since we don't know which we do not apply any transitions to any of
    // the references.
    unsafe { wild.write(13) };

    // We should be able to access any of the references.
    assert_eq!(*ref2, 13);
}

/// Checks that accessing a reborrowed wildcard reference doesn't
/// invalidate other reborrowed wildcard references, if they
/// are also exposed.
fn reborrow3() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let int = ptr_base as usize;

    let wild = int as *mut u32;

    let reb1 = unsafe { &mut *wild };
    let ref1 = &mut *reb1;
    let _int = ref1 as *mut u32 as usize;

    let reb2 = unsafe { &mut *wild };

    //   ┌────────────┐
    //   │            │
    //   │ ptr_base*  │            *                  *
    //   │            │            │                  │
    //   └────────────┘            │                  │
    //                             │                  │
    //                             │                  │
    //                             ▼                  ▼
    //                      ┌────────────┐     ┌────────────┐
    //                      │            │     │            │
    //                      │ reb1(Res)  |     │ reb2(Res)  |
    //                      │            │     │            │
    //                      └──────┬─────┘     └────────────┘
    //                             │
    //                             │
    //                             ▼
    //                      ┌────────────┐
    //                      │            │
    //                      │ ref1(Res)* │
    //                      │            │
    //                      └────────────┘

    // This is the only valid ordering these accesses can happen in.
    *reb2 = 1;
    *ref1 = 2;
    *reb1 = 3;
}

/// Analogous to same test in `../tree-borrows.rs` but with returning a
/// reborrowed wildcard reference.
fn returned_mut_is_usable() {
    let mut x: u32 = 32;
    let ref1 = &mut x;

    let y = protect(ref1);

    fn protect(arg: &mut u32) -> &mut u32 {
        // Reborrow `arg` through a wildcard.
        let int = arg as *mut u32 as usize;
        let wild = int as *mut u32;
        let ref2 = unsafe { &mut *wild };

        // Activate the reference so that it is vulnerable to foreign reads.
        *ref2 = 42;

        ref2
        // An implicit read through `arg` is inserted here.
    }

    *y = 4;
}
