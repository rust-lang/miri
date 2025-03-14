//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc -Zmiri-disable-stacked-borrows

#![no_main]

#[path = "../../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::utils_dep::genmc::{join_pthread, spawn_pthread};

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    unsafe {
        let value: *mut c_void = std::ptr::null_mut();
        let t1 = spawn_pthread(thread_1, value);
        let t2 = spawn_pthread(thread_2, value);

        join_pthread(t1);
        join_pthread(t2);
    }
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
