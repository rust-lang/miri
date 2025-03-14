//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc
//@revisions: order123 order321 order312 order231

#![no_main]

#[path = "../../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

static X: AtomicU64 = AtomicU64::new(0);

use crate::utils_dep::*;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_order = if cfg!(order123) {
        [thread_1, thread_2, thread_3]
    } else if cfg!(order321) {
        [thread_3, thread_2, thread_1]
    } else if cfg!(order312) {
        [thread_3, thread_1, thread_2]
    } else if cfg!(order231) {
        [thread_2, thread_3, thread_1]
    } else {
        unimplemented!();
    };

    let _ids = unsafe { create_pthreads_no_params(thread_order) };

    0
}

extern "C" fn thread_1(_value: *mut c_void) -> *mut c_void {
    X.store(1, Ordering::Release);
    std::ptr::null_mut()
}

extern "C" fn thread_2(_value: *mut c_void) -> *mut c_void {
    X.load(Ordering::Acquire);
    std::ptr::null_mut()
}

extern "C" fn thread_3(_value: *mut c_void) -> *mut c_void {
    X.load(Ordering::Acquire);
    std::ptr::null_mut()
}
