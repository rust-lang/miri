//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc

#![no_main]

#[path = "../../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::utils_dep::*;

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);
static Z: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let _ids = unsafe { create_pthreads_no_params([thread_1, thread_2]) };

    0
}

extern "C" fn thread_1(_value: *mut c_void) -> *mut c_void {
    let a = X.load(Ordering::Relaxed);
    let _b = Y.compare_exchange(a, 1, Ordering::Relaxed, Ordering::Relaxed);
    Z.store(a, Ordering::Relaxed);
    std::ptr::null_mut()
}

extern "C" fn thread_2(_value: *mut c_void) -> *mut c_void {
    Y.store(2, Ordering::Relaxed);
    std::ptr::null_mut()
}
