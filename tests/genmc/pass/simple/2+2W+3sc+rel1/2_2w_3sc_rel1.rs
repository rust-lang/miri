//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc

#![no_main]

// #[path = "../../../../utils-dep/mod.rs"]
// mod utils_dep;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

// use crate::utils_dep::*;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    // let thread_order = [thread_1, thread_2];
    // let _ids = unsafe { create_pthreads_no_params(thread_order) };

    let spawn = |func| {
        use libc::{pthread_attr_t, pthread_t};

        let mut thread_id: pthread_t = 0;

        let attr: *const pthread_attr_t = std::ptr::null();
        let value: *mut c_void = std::ptr::null_mut();

        let ret = unsafe { libc::pthread_create(&raw mut thread_id, attr, func, value) };
        if 0 != ret {
            std::process::abort();
        }
        thread_id
    };

    let _t1 = spawn(thread_1);
    let _t2 = spawn(thread_2);

    0
}

extern "C" fn thread_1(_value: *mut c_void) -> *mut c_void {
    X.store(1, Ordering::SeqCst);
    Y.store(2, Ordering::SeqCst);
    std::ptr::null_mut()
}

extern "C" fn thread_2(_value: *mut c_void) -> *mut c_void {
    Y.store(1, Ordering::Release);
    X.store(2, Ordering::SeqCst);
    std::ptr::null_mut()
}
