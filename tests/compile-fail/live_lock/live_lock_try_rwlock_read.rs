// ignore-windows: Concurrency on Windows is not supported yet.

use std::sync::{Arc, RwLock};
use std::thread::spawn;

extern "Rust" {
    fn miri_yield_thread();
}


fn main() {
    let shared = Arc::new(RwLock::new(0usize));
    let s1 = shared.clone();
    let mut s_guard = shared.write().unwrap();
    let j1 = spawn(move || {
        let _a_guard = loop {
            // yield loop for try-lock.
            if let Ok(guard) = s1.try_read() {
                break guard
            } else {
                unsafe { miri_yield_thread(); } //~ERROR livelock
            }
        };
    });

    j1.join().unwrap();
    *s_guard = 1;
}
