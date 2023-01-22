//@ignore-target-windows: No libc on Windows
//@ignore-target-wasm: wasm does not support threads

// Joining the main thread is undefined behavior.

use std::{ptr, thread};

fn main() {
    let thread_id: libc::pthread_t = unsafe { libc::pthread_self() };
    let handle = thread::spawn(move || {
        unsafe {
            assert_eq!(libc::pthread_join(thread_id, ptr::null_mut()), 0); //~ ERROR: Undefined Behavior: trying to join a detached thread
        }
    });
    thread::yield_now();
    handle.join().unwrap();
}
