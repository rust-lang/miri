//@ revisions: non_genmc non_genmc_std genmc genmc_std
//@[non_genmc,non_genmc_std] compile-flags:
//@[genmc,genmc_std] compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// This is the test `load_buffering` from `loom/test/litmus.rs`, adapted for Miri-GenMC.
// https://github.com/tokio-rs/loom/blob/dbf32b04bae821c64be44405a0bb72ca08741558/tests/litmus.rs

// Loom uses a memory model like C++11's, which allowed for the `test` function to return `1`.
// This is not allowed in the RC11 memory model, which is what Miri-GenMC uses.

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
    // For normal Miri, we need multiple repetitions, but 1 is enough for GenMC.
    const REPS: usize = if cfg!(any(non_genmc, non_genmc_std)) { 128 } else { 1 };

    for _ in 0..REPS {
        X.store(0, SeqCst);
        Y.store(0, SeqCst);

        if test() == 1 {
            unsafe { std::hint::unreachable_unchecked() };
        }
    }

    0
}

#[cfg(any(non_genmc_std, genmc_std))]
fn test() -> usize {
    let thread = std::thread::spawn(thread_0);
    let a = thread_1();
    thread.join().unwrap();

    a
}

#[cfg(not(any(non_genmc_std, genmc_std)))]
fn test() -> usize {
    use std::ffi::c_void;

    use crate::genmc::{join_pthread, spawn_pthread};

    extern "C" fn thread_func(_value: *mut c_void) -> *mut c_void {
        thread_0();
        std::ptr::null_mut()
    }

    let thread_id = unsafe { spawn_pthread(thread_func, std::ptr::null_mut()) };

    let a = thread_1();

    unsafe { join_pthread(thread_id) };

    a
}

fn thread_0() {
    X.store(Y.load(Relaxed), Relaxed);
}

/// Returns the value for `a`
fn thread_1() -> usize {
    let a = X.load(Relaxed);
    Y.store(1, Relaxed);
    a
}
