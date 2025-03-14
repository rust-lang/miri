//@compile-flags: -Zmiri-genmc
//@revisions: return1234 return42

#![no_main]

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use libc::{self, pthread_attr_t, pthread_t};

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let mut t0: pthread_t = 0;
    let mut t1: pthread_t = 0;

    let attr: *const pthread_attr_t = std::ptr::null();

    let mut x: AtomicU64 = AtomicU64::new(1);
    *x.get_mut() = 42;
    // x.store(42, STORE_ORD);

    let value: *mut c_void = x.as_ptr() as *mut c_void;

    assert!(0 == unsafe { libc::pthread_create(&raw mut t0, attr, read_relaxed, value) });
    assert!(0 == unsafe { libc::pthread_create(&raw mut t1, attr, write_relaxed, value) });

    assert!(0 == unsafe { libc::pthread_join(t0, std::ptr::null_mut()) });
    assert!(0 == unsafe { libc::pthread_join(t1, std::ptr::null_mut()) });

    0
}

extern "C" fn read_relaxed(value: *mut c_void) -> *mut c_void {
    unsafe {
        let x = (value as *const AtomicU64).as_ref().unwrap();
        let val = x.load(Ordering::Relaxed);

        let mut flag = false;
        if cfg!(return1234) && 1234 == val {
            flag = true;
        }
        if cfg!(return42) && 42 == val {
            flag = true;
        }
        if flag {
            std::hint::unreachable_unchecked(); //~ ERROR: entering unreachable code
        }
        std::ptr::null_mut()
    }
}

extern "C" fn write_relaxed(value: *mut c_void) -> *mut c_void {
    unsafe {
        let x = (value as *const AtomicU64).as_ref().unwrap();
        x.store(1234, Ordering::Relaxed);
        std::ptr::null_mut()
    }
}
