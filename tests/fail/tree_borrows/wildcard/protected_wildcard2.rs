//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance
use std::cell::UnsafeCell;

pub fn main() {
    let mut x: UnsafeCell<[u32; 2]> = UnsafeCell::new([32, 33]);
    let ref1 = &mut x;
    let cell_ptr = ref1.get() as *mut u32;

    let int = ref1 as *mut UnsafeCell<[u32; 2]> as usize;
    let wild = int as *mut UnsafeCell<u32>;

    let ref2 = unsafe { &mut *cell_ptr };

    let ref3 = unsafe { &mut *wild.wrapping_add(1) };

    let protect = |arg4: &mut u32| {
        *arg4 = 41;

        let ref5 = &mut *arg4;
        let _int = ref5 as *mut u32 as usize;

        let ref6 = unsafe { &mut *ref3.get() };
        //    ┌───────────┐
        //    │    ref1*  │
        //    │ Cel │ Cel │           *
        //    └─────┬─────┘           │
        //          │                 │
        //          │                 │
        //          ▼                 ▼
        //    ┌───────────┐     ┌───────────┐
        //    │ ref2      │     │    ref3   │
        //    │ Act │ Res │     │ Cel │ Cel │
        //    └─────┬─────┘     └─────┬─────┘
        //          │                 │
        //          │                 │
        //          ▼                 ▼
        //    ┌───────────┐     ┌───────────┐
        //    │ arg4      │     │       ref6│
        //    │ Act │ Res │     │ Res │ Res │
        //    └─────┬─────┘     └───────────┘
        //          │
        //          │
        //          ▼
        //    ┌───────────┐
        //    │ ref5*     │
        //    │ Res │ Res │
        //    └───────────┘
        return (ref6 as *mut u32).wrapping_sub(1);
    };

    let ptr = protect(ref2);
    let _fail = unsafe { *ptr }; //~ ERROR: /read access through .* is forbidden/
}
