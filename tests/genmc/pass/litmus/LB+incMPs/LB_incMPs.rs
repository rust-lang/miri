//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc

#![no_main]

#[path = "../../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);
static W: AtomicU64 = AtomicU64::new(0);
static Z: AtomicU64 = AtomicU64::new(0);

use crate::utils_dep::*;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_order = [thread_1, thread_2, thread_3, thread_4];
    let _ids = unsafe { create_pthreads_no_params(thread_order) };

    0
}

extern "C" fn thread_1(_value: *mut c_void) -> *mut c_void {
    X.load(Ordering::Acquire);
    Z.fetch_add(1, Ordering::AcqRel);
    std::ptr::null_mut()
}

extern "C" fn thread_2(_value: *mut c_void) -> *mut c_void {
    Z.fetch_add(1, Ordering::AcqRel);
    Y.store(1, Ordering::Release);
    std::ptr::null_mut()
}

extern "C" fn thread_3(_value: *mut c_void) -> *mut c_void {
    Y.load(Ordering::Acquire);
    W.fetch_add(1, Ordering::AcqRel);
    std::ptr::null_mut()
}

extern "C" fn thread_4(_value: *mut c_void) -> *mut c_void {
    W.fetch_add(1, Ordering::AcqRel);
    X.store(1, Ordering::Release);
    std::ptr::null_mut()
}
