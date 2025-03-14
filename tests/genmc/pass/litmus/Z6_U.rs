//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// Translated from GenMC's "litmus/Z6.U" test.

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
            X.store(1, SeqCst);
            Y.store(1, Release);
        });
        spawn_pthread_closure(|| {
            Y.fetch_add(1, SeqCst);
            Y.load(Relaxed);
        });
        spawn_pthread_closure(|| {
            Y.store(3, SeqCst);
            X.load(SeqCst);
        });
        0
    }
}
