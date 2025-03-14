//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc

#![no_main]

#[path = "../../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

static LOCK: AtomicU64 = AtomicU64::new(0);

use crate::utils_dep::*;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_order = [thread_ra, thread_r, thread_rr, thread_rs];
    let _ids = unsafe { create_pthreads_no_params(thread_order) };

    0
}

extern "C" fn thread_ra(_value: *mut c_void) -> *mut c_void {
    LOCK.fetch_add(1, Ordering::Acquire);
    LOCK.fetch_add(1, Ordering::Relaxed);
    std::ptr::null_mut()
}

extern "C" fn thread_r(_value: *mut c_void) -> *mut c_void {
    LOCK.fetch_add(1, Ordering::Relaxed);
    LOCK.fetch_add(1, Ordering::Relaxed);
    std::ptr::null_mut()
}

extern "C" fn thread_rr(_value: *mut c_void) -> *mut c_void {
    LOCK.fetch_add(1, Ordering::Release);
    std::ptr::null_mut()
}

extern "C" fn thread_rs(_value: *mut c_void) -> *mut c_void {
    LOCK.fetch_add(1, Ordering::Relaxed);
    std::ptr::null_mut()
}
