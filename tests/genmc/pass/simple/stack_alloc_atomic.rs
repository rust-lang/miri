//@compile-flags: -Zmiri-genmc

#![no_main]

use std::sync::atomic::*;

const ORD: Ordering = Ordering::SeqCst;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let x = AtomicU64::new(1234);
    let a = x.load(ORD);
    if a != 1234 {
        std::process::abort();
    }

    0
}
