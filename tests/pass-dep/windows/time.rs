//@only-target: windows # this directly tests windows-only functions
//@revisions: with_isolation without_isolation
//@[without_isolation] compile-flags: -Zmiri-disable-isolation
//@run-native

use std::mem::MaybeUninit;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

fn performance_counter() -> i64 {
    let mut counter = MaybeUninit::uninit();
    assert_ne!(unsafe { QueryPerformanceCounter(counter.as_mut_ptr()) }, 0);
    unsafe { counter.assume_init() }
}

fn performance_frequency() -> i64 {
    let mut frequency = MaybeUninit::uninit();
    assert_ne!(unsafe { QueryPerformanceFrequency(frequency.as_mut_ptr()) }, 0);
    unsafe { frequency.assume_init() }
}

fn main() {
    let frequency = performance_frequency();
    assert!(frequency > 0);

    // The frequency is fixed and shared across threads.
    assert_eq!(thread::spawn(performance_frequency).join().unwrap(), frequency);

    let before = performance_counter();
    let after = performance_counter();
    assert!(after >= before);

    thread::sleep(Duration::from_millis(10));
    let elapsed = performance_counter() - after;
    let expected = frequency / 100; // Expected ticks in 10ms.
    // Allow up to 10x the expected duration for scheduling delays.
    assert!(expected <= elapsed && elapsed <= 10 * expected);
    assert_eq!(performance_frequency(), frequency);
}
