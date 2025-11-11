//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

pub fn main() {
    multiple_exposed_siblings1();
    multiple_exposed_siblings2();
    reborrow3();
}

fn multiple_exposed_siblings1() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let ref1 = unsafe { &mut *ptr_base };
    let ref2 = unsafe { &mut *ptr_base };

    let int1 = ref1 as *mut u32 as usize;
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

    *reb = 13;

    assert_eq!(*ref2, 13);
}

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

    unsafe { wild.write(13) };

    assert_eq!(*ref2, 13);
}

fn reborrow3() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;

    let int = ptr_base as usize;

    let wild = int as *mut u32;
    let reb1 = unsafe { &mut *wild };
    let ref1 = &mut *reb1;
    let wild = ref1 as *mut u32 as usize as *mut u32;
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

    *reb2 = 1;
    *ref1 = 2;
    *reb1 = 3;
}
