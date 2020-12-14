// ignore-windows: No libc on Windows

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::spawn;

extern "Rust" {
    fn miri_yield_thread();
}
fn spin_lock(lock: &AtomicUsize) {
    while lock.compare_and_swap(0, 1, Ordering::AcqRel) != 0 { unsafe { miri_yield_thread(); } } //~ERROR livelock
}


fn main() {
    let a1 = Arc::new(AtomicUsize::new(0));
    let a2 = a1.clone();
    let b1 = Arc::new(AtomicUsize::new(0));
    let b2 = b1.clone();
    let c1 = Arc::new(AtomicUsize::new(1));
    let c2 = c1.clone();
    let j1 = spawn(move || {
        spin_lock(&a1); //Acquire a
        spin_lock(&c1); //Waits for c2.store to execute
        spin_lock(&b1); //Livelock wait for b
    });
    let j2 = spawn(move || {
        c2.store(0, Ordering::Release);
        spin_lock(&b2); //Acquire b
        spin_lock(&a2); //Livelock wait for a
    });
    j1.join().unwrap();
    j2.join().unwrap();
}
