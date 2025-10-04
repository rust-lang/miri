//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows
//@error-in-other-file: unsupported operation

// Test that we can detect a deadlock involving `std::sync::Mutex` in GenMC mode.
// FIXME(genmc): The error message for such deadlocks is currently not good, it does not show both involved lock call locations.

#![no_main]
#![feature(abort_unwind)]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::sync::Mutex;

use crate::genmc::*;

static X: Mutex<u64> = Mutex::new(0);
static Y: Mutex<u64> = Mutex::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    unsafe {
        let t0 = spawn_pthread_closure(|| {
            let mut x = X.lock().unwrap();
            let mut y = Y.lock().unwrap();
            *x += 1;
            *y += 1;
        });
        let t1 = spawn_pthread_closure(|| {
            let mut y = Y.lock().unwrap();
            let mut x = X.lock().unwrap();
            *x += 1;
            *y += 1;
        });
        join_pthreads([t0, t1]);
        0
    }
}
