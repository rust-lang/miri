//@ignore-target-windows: No libc on Windows

//! The thread function must have exactly one argument.

use std::{mem, ptr};

extern "C" fn thread_start() -> *mut libc::c_void {
    panic!() //~ ERROR: callee has fewer arguments than expected
}

fn main() {
    unsafe {
        let mut native: libc::pthread_t = mem::zeroed();
        let attr: libc::pthread_attr_t = mem::zeroed();
        // assert_eq!(libc::pthread_attr_init(&mut attr), 0); FIXME: this function is not yet implemented.
        let thread_start: extern "C" fn() -> *mut libc::c_void = thread_start;
        let thread_start: extern "C" fn(*mut libc::c_void) -> *mut libc::c_void =
            mem::transmute(thread_start);
        assert_eq!(libc::pthread_create(&mut native, &attr, thread_start, ptr::null_mut()), 0);
        assert_eq!(libc::pthread_join(native, ptr::null_mut()), 0);
    }
}
