//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc

#![no_main]

#[path = "../../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

// Note: the GenMC equivalent of this test (genmc/tests/correct/litmus/MPU+rels+acq/mpu+rels+acq.c) uses non-atomic accesses for `X` with disabled race detection.
static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

use crate::utils_dep::*;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_order = [thread_1, thread_2, thread_3];
    let _ids = unsafe { create_pthreads_no_params(thread_order) };

    0
}

extern "C" fn thread_1(_value: *mut c_void) -> *mut c_void {
    X.store(1, Ordering::Relaxed);

    Y.store(0, Ordering::Release);
    Y.store(1, Ordering::Relaxed);
    std::ptr::null_mut()
}

extern "C" fn thread_2(_value: *mut c_void) -> *mut c_void {
    Y.fetch_add(1, Ordering::Relaxed);
    std::ptr::null_mut()
}

extern "C" fn thread_3(_value: *mut c_void) -> *mut c_void {
    if Y.load(Ordering::Acquire) > 1 {
        X.store(2, Ordering::Relaxed);
    }
    std::ptr::null_mut()
}
