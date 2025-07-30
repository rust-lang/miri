//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows
//@revisions: acq_rel relaxed

// This test is the equivalent to the `2w2w_seqcst.rs` "pass" test.
// Here we use weaker atomic memory orderings to test if we can encounter
// an execution where (X == 1 && Y == 1).

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::genmc::{join_pthread, spawn_pthread};

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

#[cfg(acq_rel)]
const LOAD_ORD: Ordering = Ordering::Acquire;
#[cfg(acq_rel)]
const STORE_ORD: Ordering = Ordering::Release;

#[cfg(not(acq_rel))]
const LOAD_ORD: Ordering = Ordering::Relaxed;
#[cfg(not(acq_rel))]
const STORE_ORD: Ordering = Ordering::Relaxed;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_id = unsafe { spawn_pthread(thread_func, std::ptr::null_mut()) };

    X.store(1, STORE_ORD);
    Y.store(2, STORE_ORD);

    unsafe { join_pthread(thread_id) };

    let x = X.load(LOAD_ORD);
    let y = Y.load(LOAD_ORD);
    if x == 1 && y == 1 {
        unsafe { std::hint::unreachable_unchecked() }; //~ ERROR: entering unreachable code
    }
    0
}

extern "C" fn thread_func(_value: *mut c_void) -> *mut c_void {
    Y.store(1, STORE_ORD);
    X.store(2, STORE_ORD);
    std::ptr::null_mut()
}
