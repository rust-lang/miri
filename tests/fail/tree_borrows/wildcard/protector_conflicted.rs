//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

#[allow(unused_variables)]
pub fn main() {
    let mut x: u32 = 42;

    let ptr_base = &mut x as *mut u32;
    let ref1 = unsafe { &mut *ptr_base };
    let ref2 = unsafe { &mut *ptr_base };

    let protect = |arg: &mut u32| {
        // Expose arg.
        let int = arg as *mut u32 as usize;
        let wild = int as *mut u32;

        // Does a foreign read to arg marking it as conflicted and making child_writes UB while it's protected.
        let _x = *ref2;

        // UB because it tries to write through arg.
        unsafe { *wild = 4 }; //~ ERROR: /write access through wildcard at .* is forbidden/
    };

    protect(ref1);
}
