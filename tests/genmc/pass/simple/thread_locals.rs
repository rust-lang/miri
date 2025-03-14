//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc

#![no_main]

#[path = "../../../utils-dep/mod.rs"]
mod utils_dep;

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::utils_dep::*;

static X: AtomicPtr<u64> = AtomicPtr::new(std::ptr::null_mut());

thread_local! {
    static R: Cell<*mut u64> = Cell::new(std::ptr::null_mut());
}

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let _ids = unsafe { create_pthreads_no_params([thread_1, thread_2, thread_3]) };

    0
}

pub unsafe fn malloc() -> *mut u64 {
    Box::into_raw(Box::<u64>::new_uninit()) as *mut u64
}

extern "C" fn thread_1(_value: *mut c_void) -> *mut c_void {
    unsafe {
        R.set(malloc());
        let r_ptr = R.get();
        let _ = X.compare_exchange(std::ptr::null_mut(), r_ptr, Ordering::SeqCst, Ordering::SeqCst);
        std::ptr::null_mut()
    }
}

extern "C" fn thread_2(_value: *mut c_void) -> *mut c_void {
    unsafe {
        R.set(malloc());
        std::ptr::null_mut()
    }
}

extern "C" fn thread_3(_value: *mut c_void) -> *mut c_void {
    unsafe {
        R.set(malloc());
        let r_ptr = R.get();
        let _ = X.compare_exchange(std::ptr::null_mut(), r_ptr, Ordering::SeqCst, Ordering::SeqCst);
        std::ptr::null_mut()
    }
}
