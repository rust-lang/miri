//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

#[allow(unused_variables)]
pub fn main() {
    let mut x: u32 = 42;

    let ref1 = &mut x;
    let int1 = ref1 as *mut u32 as usize;

    let ref2 = &mut *ref1;

    let ref3 = &mut *ref2;
    let int3 = ref3 as *mut u32 as usize;

    //write through ref3 so that they all are active
    *ref3 = 43;

    let wild = int1 as *mut u32;

    // graph TD
    // ref1(Res)* --> ref2(Res) --> ref3(Res)*
    //
    //     ┌────────────┐
    //     │            │
    //     │ ref1(Act)* │
    //     │            │
    //     └──────┬─────┘
    //            │
    //            │
    //            ▼
    //     ┌────────────┐
    //     │            │
    //     │ ref2(Act)  │
    //     │            │
    //     └──────┬─────┘
    //            │
    //            │
    //            ▼
    //     ┌────────────┐
    //     │            │
    //     │ ref3(Act)* │
    //     │            │
    //     └────────────┘

    // writes through either ref1 or ref3, which is either a child or foreign access to ref2.
    unsafe { wild.write(42) };

    //reading from ref2 still works since the previous access could have been through its child
    //this also freezes ref3
    let x = *ref2;

    // we can still write through wild, as there is still the exposed ref1 with write permissions
    // under proper exposed provenance this would be UB as the only tag wild can assume to not
    // invalidate ref2 is ref3, which we just invalidated
    //
    // disables ref2,ref3
    unsafe { wild.write(43) };

    // fails because ref2 is disabled
    let fail = *ref2; //~ ERROR: /read access through .* is forbidden/
}
