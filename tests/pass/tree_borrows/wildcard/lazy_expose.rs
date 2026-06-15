//@compile-flags: -Zmiri-tree-borrows -Zmiri-provenance-gc=0 -Zmiri-permissive-provenance

// This program contains a false positive bug found in lazy allocation. Single-node
// trees should still be able to be exposed, even if they are `Uninit`.

fn main() {
    let mut x: u32 = 0;

    // `&raw mut x` is a raw-pointer retag and x's tree stays `Uninit`.
    let p: *mut u32 = &raw mut x;

    // Integer cast exposes the root tag via, but it is still `Uninit`.
    // `expose_tag` now writes `exposed` to true (used to be no-op).
    let addr = p as usize;

    // Creating `&mut x` triggers a retag so the tree is now `Init`!
    let _r = &mut x;

    // `addr` was derived from an exposed pointer, so this wildcard read is valid.
    // The root tag should be in the `ExposedCache` with write-level access.
    // Previously, lazy alloc falsely reported UB here:
    let y = addr as *mut u32;
    let _ = unsafe { *y };
}
