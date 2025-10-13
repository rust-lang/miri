//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance
// NOTE: this file documents UB that is not detected by wildcard provenance.

pub fn main() {
    protected_exposed();
    protected_wildcard();
}

// if a reference is protected, all foreign writes to it cause UB, this effectively means any
// write needs to happen through a child of the protected reference. this information would allow
// us to further narrow the possible candidates for a wildcard write.
#[allow(unused_variables)]
pub fn protected_exposed() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let ref1 = unsafe { &mut *ptr_base };
    let ref2 = unsafe { &mut *ptr_base };

    let int2 = ref2 as *mut u32 as usize;

    let wild = int2 as *mut u32;
    fn protect(ref3: &mut u32) {
        let int3 = ref3 as *mut u32 as usize;
        //    ┌────────────┐
        //    │            │
        //    │  ptr_base  ├──────────────┐
        //    │            │              │
        //    └──────┬─────┘              │
        //           │                    │
        //           │                    │
        //           ▼                    ▼
        //    ┌────────────┐       ┌────────────┐
        //    │            │       │            │
        //    │ ref1(Res)  │       │ ref2(Res)* │
        //    │            │       │            │
        //    └──────┬─────┘       └────────────┘
        //           │
        //           │
        //           ▼
        //    ┌────────────┐
        //    │            │
        //    │ ref3(Res)* │
        //    │            │
        //    └────────────┘

        // since ref3 is protected, we know that every write from outside it will be UB
        // this means we know that the access is through ref3 disabling ref2
        let wild = int3 as *mut u32;
        unsafe { wild.write(13) }
    }
    protect(ref1);

    // ref 2 is disabled, so this read causes UB
    let fail = *ref2;
}

// we currently ignore, if a wildcard pointer has a protector
#[allow(unused_variables)]
pub fn protected_wildcard() {
    let mut x: u32 = 32;
    let ref1 = &mut x;
    let ref2 = &mut *ref1;

    let int = ref2 as *mut u32 as usize;
    let wild = int as *mut u32;
    let wild_ref = unsafe { &mut *wild };

    let mut protect = |arg: &mut u32| {
        // arg is a protected pointer with wildcard provenance
        //    ┌────────────┐
        //    │            │
        //    │ ref1(Res)  │
        //    │            │
        //    └──────┬─────┘
        //           │
        //           │
        //           ▼
        //    ┌────────────┐
        //    │            │
        //    │ ref2(Res)* │
        //    │            │
        //    └────────────┘

        // writes to ref1 disabling ref2, which also disables all wildcard references.
        // since a wildcard reference is protected this is UB.
        *ref1 = 13;
    };

    //we pass a pointer with wildcard provenance
    protect(wild_ref);
}
