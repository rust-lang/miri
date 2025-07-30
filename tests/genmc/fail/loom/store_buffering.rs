//@ revisions: non_genmc non_genmc_std genmc genmc_std
//@[non_genmc,non_genmc_std] compile-flags:
//@[genmc,genmc_std] compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// This is the test `store_buffering` from `loom/test/litmus.rs`, adapted for Miri-GenMC.
// https://github.com/tokio-rs/loom/blob/dbf32b04bae821c64be44405a0bb72ca08741558/tests/litmus.rs

// This test doubles as a comparison between using std threads and pthreads, and normal Miri vs Miri-GenMC.

#![no_main]

#[cfg(not(any(non_genmc_std, genmc_std)))]
#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};

static X: AtomicUsize = AtomicUsize::new(0);
static Y: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    // For normal Miri, we need multiple repetitions, but GenMC should find the bug with only 1.
    const REPS: usize = if cfg!(any(non_genmc, non_genmc_std)) { 128 } else { 1 };

    for _ in 0..REPS {
        X.store(0, SeqCst);
        Y.store(0, SeqCst);

        if test() == (0, 0) {
            unsafe { std::hint::unreachable_unchecked() }; //~ ERROR: entering unreachable code
        }
    }

    0
}

#[cfg(any(non_genmc_std, genmc_std))]
fn test() -> (usize, usize) {
    let thread = std::thread::spawn(thread_0);

    let b = thread_1();

    let a = thread.join().unwrap();

    (a, b)
}

#[cfg(not(any(non_genmc_std, genmc_std)))]
fn test() -> (usize, usize) {
    use std::ffi::c_void;

    use crate::genmc::{join_pthread, spawn_pthread};

    extern "C" fn thread_func(value: *mut c_void) -> *mut c_void {
        let a_ptr = value as *mut usize;
        let a = thread_0();
        unsafe { *a_ptr = a };
        std::ptr::null_mut()
    }

    let mut a: usize = 0;
    let thread_id = unsafe { spawn_pthread(thread_func, &raw mut a as *mut c_void) };

    let b = thread_1();

    unsafe { join_pthread(thread_id) };

    (a, b)
}

/// Returns the value for `a`
fn thread_0() -> usize {
    X.store(1, Relaxed);
    Y.load(Relaxed)
}

/// Returns the value for `b`
fn thread_1() -> usize {
    Y.store(1, Relaxed);
    X.load(Relaxed)
}
