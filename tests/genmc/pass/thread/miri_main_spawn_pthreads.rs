//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

#![no_main]

use std::ffi::c_void;

use libc::{self, pthread_attr_t, pthread_t};

const N: usize = 2;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let mut handles: Vec<pthread_t> = vec![0; N];

    let attr: *const pthread_attr_t = std::ptr::null();
    let value: *mut c_void = std::ptr::null_mut();

    handles.iter_mut().for_each(|thread_id| {
        if unsafe { libc::pthread_create(thread_id, attr, thread_func, value) } != 0 {
            std::process::abort();
        }
    });

    handles.into_iter().for_each(|thread_id| {
        if unsafe { libc::pthread_join(thread_id, std::ptr::null_mut()) } != 0 {
            std::process::abort();
        }
    });

    0
}

extern "C" fn thread_func(_value: *mut c_void) -> *mut c_void {
    std::ptr::null_mut()
}
