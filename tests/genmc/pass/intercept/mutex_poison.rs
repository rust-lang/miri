//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// Test that Mutex poisoning still works in GenMC mode.

#![no_main]

use std::sync::{LockResult, Mutex};

static LOCK: Mutex<u64> = Mutex::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    *LOCK.lock().unwrap() = 1234;
    let handle = std::thread::spawn(|| {
        let mut guard = LOCK.lock().unwrap();
        *guard = 42;
        panic!(); // This will poison the mutex.
    });
    if handle.join().is_ok() {
        std::process::abort();
    }
    // The mutex should now be poisoned and contain the value the other thread wrote:
    match LOCK.lock() {
        LockResult::Err(poison) if **poison.get_ref() == 42 => {}
        _ => std::process::abort(),
    }
    0
}
