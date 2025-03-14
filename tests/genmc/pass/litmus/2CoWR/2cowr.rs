//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc
//@revisions: order1234 order2341 order3412 order4123 order4321

#![no_main]

#[path = "../../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::utils_dep::*;

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_order = if cfg!(order1234) {
        [thread_1, thread_2, thread_3, thread_4]
    } else if cfg!(order2341) {
        [thread_2, thread_3, thread_4, thread_1]
    } else if cfg!(order3412) {
        [thread_3, thread_4, thread_1, thread_2]
    } else if cfg!(order4123) {
        [thread_4, thread_1, thread_2, thread_3]
    } else if cfg!(order4321) {
        [thread_4, thread_3, thread_2, thread_1]
    } else {
        unimplemented!();
    };

    let _ids = unsafe { create_pthreads_no_params(thread_order) };

    0
}

pub extern "C" fn thread_1(_value: *mut c_void) -> *mut c_void {
    Y.load(Ordering::Acquire);
    null_mut()
}

pub extern "C" fn thread_2(_value: *mut c_void) -> *mut c_void {
    X.load(Ordering::Acquire);
    null_mut()
}

pub extern "C" fn thread_3(_value: *mut c_void) -> *mut c_void {
    X.store(1, Ordering::Release);
    null_mut()
}

pub extern "C" fn thread_4(_value: *mut c_void) -> *mut c_void {
    Y.store(1, Ordering::Release);
    null_mut()
}
