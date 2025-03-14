//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc
//@revisions: order1234 order4321 order4123 order3412 order2341

#![no_main]

#[path = "../../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

static X: AtomicU64 = AtomicU64::new(0);

use crate::utils_dep::*;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_order = if cfg!(order1234) {
        [thread_1, thread_2, thread_3, thread_4]
    } else if cfg!(order4321) {
        [thread_4, thread_3, thread_2, thread_1]
    } else if cfg!(order4123) {
        [thread_4, thread_1, thread_2, thread_3]
    } else if cfg!(order3412) {
        [thread_3, thread_4, thread_1, thread_2]
    } else if cfg!(order2341) {
        [thread_2, thread_3, thread_4, thread_1]
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
    X.store(2, Ordering::Release);
    std::ptr::null_mut()
}

extern "C" fn thread_3(_value: *mut c_void) -> *mut c_void {
    X.load(Ordering::Acquire);
    X.load(Ordering::Acquire);
    std::ptr::null_mut()
}

extern "C" fn thread_4(_value: *mut c_void) -> *mut c_void {
    X.load(Ordering::Acquire);
    X.load(Ordering::Acquire);
    std::ptr::null_mut()
}
