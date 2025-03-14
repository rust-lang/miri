//@compile-flags: -Zmiri-genmc

#![no_main]

use std::sync::atomic::*;

static VALUE: AtomicUsize = AtomicUsize::new(0);

const ORD: Ordering = Ordering::SeqCst;

fn assert_eq(x: usize, y: usize) {
    if x != y {
        std::process::abort();
    }
}

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    VALUE.store(1, ORD);

    assert_eq(1, VALUE.fetch_add(7, ORD));
    assert_eq(8, VALUE.fetch_sub(2, ORD));
    assert_eq(6, VALUE.fetch_max(16, ORD));
    assert_eq(16, VALUE.fetch_min(4, ORD));
    assert_eq(4, VALUE.swap(42, ORD));

    assert_eq(42, VALUE.load(ORD));
    0
}
