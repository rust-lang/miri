//@compile-flags: -Zmiri-genmc

#![no_main]

use std::sync::atomic::{AtomicU64, Ordering};

static X: AtomicU64 = AtomicU64::new(1234);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    // TODO GENMC: make this a "pass" test
    if 1234 != unsafe { *X.as_ptr() } {
        unsafe { std::hint::unreachable_unchecked() };
    }
    if 1234 == X.load(Ordering::SeqCst) {
        unsafe { std::hint::unreachable_unchecked() }; //~ ERROR: entering unreachable code
    }

    0
}
