// ignore-windows: Concurrency on Windows is not supported yet.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::spawn;

extern "Rust" {
    fn miri_yield_thread();
}

struct SpinLock(AtomicUsize);
impl SpinLock {
    fn new() -> Self {
        Self(AtomicUsize::new(0))
    }
    fn lock(&self) {
        loop {
            if let Ok(_) = self.0.compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed) {
                break
            } else {
                unsafe { miri_yield_thread(); } //~ERROR livelock
            }
        }
    }
    fn unlock(&self) {
        self.0.store(0, Ordering::Release);
    }
}

fn main() {
    // forces a deadlock via yield points
    let shared = Arc::new((SpinLock::new(),SpinLock::new()));
    let s1 = shared.clone();
    let s2 = shared.clone();
    let j1 = spawn(move || {
        s1.0.lock();
        std::thread::yield_now();
        s1.1.lock();
        s1.1.unlock();
        s1.0.unlock();
    });
    let j2 = spawn(move || {
        s2.1.lock();
        std::thread::yield_now();
        s2.0.lock();
        s2.0.unlock();
        s2.1.unlock();
    });
    j1.join().unwrap();
    j2.join().unwrap();
}
