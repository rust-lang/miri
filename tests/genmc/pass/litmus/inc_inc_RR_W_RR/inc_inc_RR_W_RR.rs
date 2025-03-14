//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc

#![no_main]

#[path = "../../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::utils_dep::*;

static X: AtomicU64 = AtomicU64::new(0);

static mut A: u64 = 0;
static mut B: u64 = 0;
static mut C: u64 = 0;
static mut D: u64 = 0;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_order = [thread_1, thread_2, thread_3, thread_4, thread_5];
    let ids = unsafe { create_pthreads_no_params(thread_order) };
    unsafe { join_pthreads(ids) };

    if unsafe { A == 42 && B == 2 && C == 1 && D == 42 } {
        std::process::abort();
    }

    0
}

pub extern "C" fn thread_1(_value: *mut c_void) -> *mut c_void {
    X.fetch_add(1, Ordering::Relaxed);
    null_mut()
}

pub extern "C" fn thread_2(_value: *mut c_void) -> *mut c_void {
    X.fetch_add(1, Ordering::Relaxed);
    null_mut()
}

pub extern "C" fn thread_3(_value: *mut c_void) -> *mut c_void {
    unsafe {
        A = X.load(Ordering::Relaxed);
        B = X.load(Ordering::Relaxed);
    }
    null_mut()
}

pub extern "C" fn thread_4(_value: *mut c_void) -> *mut c_void {
    X.store(42, Ordering::Relaxed);
    null_mut()
}

pub extern "C" fn thread_5(_value: *mut c_void) -> *mut c_void {
    unsafe {
        C = X.load(Ordering::Relaxed);
        D = X.load(Ordering::Relaxed);
    }
    null_mut()
}
