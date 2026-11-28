// Miri only emulates spurious wakeups for futex waits. On these targets, std implements `Condvar`
// on top of a futex (`FUTEX_WAIT`, `_umtx_op`, or `WaitOnAddress`), so `Condvar::wait` can wake up
// spuriously.
//@only-target: linux android freebsd windows
// Make every blocked futex waiter wake up spuriously at the next scheduling point.
//@compile-flags: -Zmiri-spurious-wakeup-rate=1.0

// `Condvar::wait` can return spuriously, so checking the condition with `if` instead of `while`
// is a bug. Nobody ever notifies the condvar here, so `wait` only returns due to a spurious
// wakeup, and the assertion fails.

use std::sync::{Condvar, Mutex};

fn main() {
    let ready = Mutex::new(false);
    let cvar = Condvar::new();

    let mut guard = ready.lock().unwrap();
    if !*guard {
        guard = cvar.wait(guard).unwrap();
    }
    assert!(*guard, "woke up without being notified");
}
