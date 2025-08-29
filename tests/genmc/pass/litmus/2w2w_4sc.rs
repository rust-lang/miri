//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::*;

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

use crate::genmc::*;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    unsafe {
        let ids = [
            spawn_pthread_closure(|| {
                X.store(1, SeqCst);
                Y.store(2, SeqCst);
            }),
            spawn_pthread_closure(|| {
                Y.store(1, SeqCst);
                X.store(2, SeqCst);
            }),
        ];
        // Join so we can read the final values.
        join_pthreads(ids);

        // Print the final values:
        let result = (X.load(Relaxed), Y.load(Relaxed));
        if !matches!(result, (2, 1) | (2, 2) | (1, 2)) {
            std::process::abort();
        }

        0
    }
}
