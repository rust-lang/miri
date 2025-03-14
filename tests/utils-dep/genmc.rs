use std::ffi::c_void;

use libc::{self, pthread_attr_t, pthread_t};

/// Spawn 1 thread using `pthread_create`, abort the process on any errors.
pub unsafe fn spawn_pthread(
    f: extern "C" fn(*mut c_void) -> *mut c_void,
    value: *mut c_void,
) -> pthread_t {
    let mut thread_id: pthread_t = 0;

    let attr: *const pthread_attr_t = std::ptr::null();

    if 0 != unsafe { libc::pthread_create(&raw mut thread_id, attr, f, value) } {
        std::process::abort();
    }
    thread_id
}

// Join the given pthread, abort the process on any errors.
pub unsafe fn join_pthread(thread_id: pthread_t) {
    if 0 != unsafe { libc::pthread_join(thread_id, std::ptr::null_mut()) } {
        std::process::abort();
    }
}

/// Spawn `N` threads using `pthread_create` without any arguments, abort the process on any errors.
pub unsafe fn create_pthreads_no_params<const N: usize>(
    functions: [extern "C" fn(*mut c_void) -> *mut c_void; N],
) -> [pthread_t; N] {
    let value = std::ptr::null_mut();
    functions.map(|func| spawn_pthread(func, value))
}

// Join the `N` given pthreads, abort the process on any errors.
pub unsafe fn join_pthreads<const N: usize>(thread_ids: [pthread_t; N]) {
    let _ = thread_ids.map(|id| join_pthread(id));
}
