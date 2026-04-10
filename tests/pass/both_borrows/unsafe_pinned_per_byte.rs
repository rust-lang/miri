#![feature(unsafe_pinned)]
use std::pin::UnsafePinned;
#[repr(C)]
struct Mixed {
    before: u32,
    pinned: UnsafePinned<u32>,
    after: u32,
}

fn main() {
    let m = Mixed { before: 1, pinned: UnsafePinned::new(2), after: 3 };
    // 'before' and 'after' should still be protected by aliasing rules
    // 'pinned' should not — Miri must not flag this as UB
    let _ref1 = &m.before;
    let _ref2 = &m.after;
}
