//@compile-flags: -Zmiri-genmc

#![no_main]

use std::ffi::c_void;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::SeqCst;

use libc::{self, pthread_attr_t, pthread_t};

static FLAG: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let mut thread_id: pthread_t = 0;

    let attr: *const pthread_attr_t = std::ptr::null();
    let value: *mut c_void = std::ptr::null_mut();

    assert!(0 == unsafe { libc::pthread_create(&raw mut thread_id, attr, thread_func, value) });

    FLAG.store(1, SeqCst);

    assert!(0 == unsafe { libc::pthread_join(thread_id, std::ptr::null_mut()) });

    let flag = FLAG.load(SeqCst);
    assert!(flag == 1 || flag == 2);
    return 0;
}

extern "C" fn thread_func(_value: *mut c_void) -> *mut c_void {
    FLAG.store(2, SeqCst);
    std::ptr::null_mut()
}
