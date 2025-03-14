//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// Translated from GenMC's "litmus/IRIWish" test.

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::*;

use crate::genmc::*;

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    unsafe {
        spawn_pthread_closure(|| {
            X.store(1, Relaxed);
        });
        spawn_pthread_closure(|| {
            let r1 = X.load(Relaxed);
            Y.store(r1, Release);
        });
        spawn_pthread_closure(|| {
            let _r1 = X.load(Relaxed);
            std::sync::atomic::fence(AcqRel);
            let _r2 = Y.load(Relaxed);
        });
        spawn_pthread_closure(|| {
            let _r1 = Y.load(Relaxed);
            std::sync::atomic::fence(AcqRel);
            let _r2 = X.load(Relaxed);
        });
        0
    }
}
