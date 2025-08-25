//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::genmc::*;

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let _ids = unsafe { create_pthreads_no_params([thread_1, thread_2, thread_3, thread_4]) };

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
