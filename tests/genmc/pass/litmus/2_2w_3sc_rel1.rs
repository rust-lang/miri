//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::sync::atomic::{AtomicU64, Ordering};

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

use crate::genmc::*;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let _t0 = unsafe {
        spawn_pthread_closure(|| {
            X.store(1, Ordering::SeqCst);
            Y.store(2, Ordering::SeqCst);
        })
    };
    let _t1 = unsafe {
        spawn_pthread_closure(|| {
            Y.store(1, Ordering::Release);
            X.store(2, Ordering::SeqCst);
        })
    };

    0
}
