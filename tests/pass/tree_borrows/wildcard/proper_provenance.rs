//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

// NOTE: this function has UB that is not detected by wildcard provenance.
// we would need proper exposed provenance handling to support it
#[allow(unused_variables)]
pub fn main() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    // we create 2 mutable references each with a unique tag
    let ref1 = unsafe { &mut *ptr_base };
    let ref2 = unsafe { &mut *ptr_base };

    // both references get exposed
    let int1 = ref1 as *mut u32 as usize;
    let int2 = ref2 as *mut u32 as usize;
    //ref1 :        Reserved
    //ref2 :        Reserved

    // we need to pick the "correct" tag for wild from the exposed tags
    let wild = int1 as *mut u32;
    //              wild=ref1   wild=ref2
    //ref1 :        Reserved    Reserved
    //ref2 :        Reserved    Reserved

    // we write to wild, disabling the other tag
    unsafe { wild.write(13) };
    //              wild=ref1   wild=ref2
    //ref1 :        Unique      Disabled
    //ref2 :        Disabled    Unique

    // we access both references, even though one of them should be disabled
    // under proper exposed provenance this is UB
    // however wildcard provenance cannot detect this
    assert_eq!(*ref1, 13);
    //              wild=ref1   wild=ref2
    //ref1 :        Unique      UB
    //ref2 :        Disabled    Frozen
    assert_eq!(*ref2, 13);
    //              wild=ref1   wild=ref2
    //ref1 :        Frozen      UB
    //ref2 :        UB          Frozen
}
