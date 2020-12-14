// ignore-windows: No libc on Windows

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::spawn;

extern "Rust" {
    fn miri_yield_thread();
}
fn spin_await(lock: &AtomicUsize, target: usize) {
    while lock.load(Ordering::Acquire) != target { unsafe { miri_yield_thread(); } } //~ERROR livelock
}


fn main() {
    let a = Arc::new(AtomicUsize::new(0));
    let b = Arc::new(AtomicUsize::new(0));
    let c = Arc::new(AtomicUsize::new(0));
    let j1 = spawn(move || {
        spin_await(&a, 1)
    });
    let j2 = spawn(move || {
        spin_await(&b, 2)
    });
    let j3 = spawn(move || {
        for i in 0..256 {
            c.store(i, Ordering::Release)
        }
    });
    j1.join().unwrap();
    j2.join().unwrap();
    j3.join().unwrap();
}
