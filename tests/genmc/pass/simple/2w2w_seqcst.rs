//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// TODO GENMC: this test currently takes 3 iterations, it this correct?

#![no_main]

#[path = "../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::utils_dep::{join_pthread, spawn_pthread};

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

const LOAD_ORD: Ordering = Ordering::SeqCst;
const STORE_ORD: Ordering = Ordering::SeqCst;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_id = unsafe { spawn_pthread(thread_func, std::ptr::null_mut()) };

    X.store(1, STORE_ORD);
    Y.store(2, STORE_ORD);

    unsafe { join_pthread(thread_id) };

    let x = X.load(LOAD_ORD);
    let y = Y.load(LOAD_ORD);
    if x == 1 && y == 1 {
        unsafe { std::hint::unreachable_unchecked() };
    }
    0
}

extern "C" fn thread_func(_value: *mut c_void) -> *mut c_void {
    Y.store(1, STORE_ORD);
    X.store(2, STORE_ORD);
    std::ptr::null_mut()
}
