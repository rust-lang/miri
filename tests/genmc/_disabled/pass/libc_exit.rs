//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows
//@revisions: order0123 order3012 order2301 order1230 order3210 order1032

#![no_main]

// NOTE: Disabled due to incomplete support for `libc::exit`.

// Copied from `tests/genmc/pass/litmus/inc2w.rs`

#[path = "../../../../utils/genmc.rs"]
mod genmc;

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::genmc::*;

static X: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_order = if cfg!(order0123) {
        [thread_0_exit, thread_1, thread_2, thread_3]
    } else if cfg!(order3012) {
        [thread_3, thread_0_exit, thread_1, thread_2]
    } else if cfg!(order2301) {
        [thread_2, thread_3, thread_0_exit, thread_1]
    } else if cfg!(order1230) {
        [thread_1, thread_2, thread_3, thread_0_exit]
    } else if cfg!(order3210) {
        [thread_3, thread_2, thread_1, thread_0_exit]
    } else if cfg!(order1032) {
        [thread_1, thread_0_exit, thread_3, thread_2]
    } else {
        unimplemented!();
    };

    let _ids = unsafe { create_pthreads_no_params(thread_order) };

    0
}

pub extern "C" fn thread_0_exit(_value: *mut c_void) -> *mut c_void {
    unsafe { libc::exit(0) }
}

pub extern "C" fn thread_1(_value: *mut c_void) -> *mut c_void {
    X.fetch_add(1, Ordering::Relaxed);
    null_mut()
}

pub extern "C" fn thread_2(_value: *mut c_void) -> *mut c_void {
    X.store(4, Ordering::Release);
    null_mut()
}

pub extern "C" fn thread_3(_value: *mut c_void) -> *mut c_void {
    X.fetch_add(2, Ordering::Relaxed);
    null_mut()
}
