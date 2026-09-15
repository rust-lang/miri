// Counterexample showing why the provenance-GC tree compaction may *only* splice out a dead
// node that has a single child, never one with several children.
//
// Interior-mutable `Reserved` (`ReservedIM`) is the witness: it survives foreign accesses
// unchanged, while the parent it shares is driven to `Unique` (by the child write through `a`)
// and then to `Frozen` (by a foreign read). A later child write through `b` is then forbidden by
// the `Frozen` parent, even though `b` itself (still `ReservedIM`) would accept it. Splicing the
// parent away loses that UB and wrongly accepts this program.
//
//@compile-flags: -Zmiri-tree-borrows -Zmiri-tree-gc-visits=1 -Zmiri-tree-gc-min-nodes=0
#[path = "../../utils/mod.rs"]
mod utils;

use std::cell::Cell;

// Build the dead parent `p` (a child of `g`) with two interior-mutable children `a` and `b`,
// returning them as raw pointers. `p`'s tag dies when this function returns, since no live local
// keeps its provenance — so the GC is free to try to compact `p` away.
#[inline(never)]
unsafe fn make(g_ptr: *mut u8) -> (*mut u8, *mut u8) {
    let p_raw = (&mut *(g_ptr as *mut Cell<u8>)) as *mut Cell<u8>;
    let a = (&mut *p_raw) as *mut Cell<u8> as *mut u8;
    let b = (&mut *p_raw) as *mut Cell<u8> as *mut u8;
    (a, b)
}

fn main() {
    let mut root = Cell::new(0u8);

    // `g` is an ancestor we keep live so we can issue a *foreign* read to the subtree below it.
    let g_ptr = &mut root as *mut Cell<u8> as *mut u8;

    let (a, b) = unsafe { make(g_ptr) };

    // Compact the tree. A correct GC keeps `p` (it has two children); the unsound
    // multi-child variant splices `p` out and reparents `a` and `b` onto `g`.
    utils::run_provenance_gc();

    unsafe {
        // Child write through `a`: activates `a` (and `p`, if present) to `Unique`; the foreign
        // write leaves `b` at `ReservedIM`.
        *a = 1;
        // Foreign read through the ancestor `g`: freezes the `Unique` nodes (`a` and `p`) but
        // leaves `b` writable. (A `read_volatile` so the load actually happens — `let _ = *g_ptr`
        // would bind the place without reading it.)
        let _v = std::ptr::read_volatile(g_ptr);
        // Child write through `b`: forbidden because the (kept) `Frozen` parent `p` rejects it.
        // If `p` had been spliced out, this write would be wrongly accepted.
        *b = 2; //~ ERROR: /write access through .* is forbidden/
    }
}
