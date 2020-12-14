// ignore-windows: Concurrency on Windows is not supported yet.

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
    let b = a.clone();
    let c = a.clone();
    let d = a.clone();
    let j1 = spawn(move || {
        spin_await(&a, 1)
    });
    let j2 = spawn(move || {
        spin_await(&b, 1)
    });
    let j3 = spawn(move || {
        spin_await(&c, 1)
    });
    let j4 = spawn(move || {
        d.store(2, Ordering::Release);
        d.store(3, Ordering::Release);
        d.store(4, Ordering::Release);
    });
    j1.join().unwrap();
    j2.join().unwrap();
    j3.join().unwrap();
    j4.join().unwrap();
}
