// compile-flags: -Zmiri-disable-isolation
// ignore-windows: Concurrency on Windows is not supported yet.

// FIXME: the implicit mutex unlock & lock counts as forward progress with the current detector,
// so this runs forever. Ideally this case should be detected.

// ignore-linux: currently is not detected.
// ignore-macos: currently is not detected.

use std::sync::{Mutex, Condvar};
use std::time::Duration;

extern "Rust" {
    fn miri_yield_thread();
}


fn main() {
    let mutex = Mutex::new(());
    let condvar = Condvar::new();

    let mut lock = mutex.lock().unwrap();
    loop {
        match s1.0.wait_timeout(lock, Duration::from_millis(100)) {
            Ok(_) => break,
            Err(err) => {
                lock = err.into_inner().0;
                unsafe { miri_yield_thread(); } //~ERROR livelock
            }
        }
    }
}
