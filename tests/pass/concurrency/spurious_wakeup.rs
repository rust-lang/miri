// Miri only emulates spurious wakeups for futex waits. On these targets, std implements `Condvar`,
// `Mutex`, and thread parking on top of a futex (`FUTEX_WAIT`, `_umtx_op`, or `WaitOnAddress`), so
// they can wake up spuriously. On other targets such as macOS, std uses pthread primitives and
// dispatch semaphores instead, which Miri does not wake up spuriously.
//@only-target: linux android freebsd windows
// Make every blocked futex waiter wake up spuriously at the next scheduling point.
//@compile-flags: -Zmiri-spurious-wakeup-rate=1.0

// Code that correctly handles spurious wakeups must still work when every wait wakes up
// spuriously.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

fn park_loop() {
    static FLAG: AtomicBool = AtomicBool::new(false);

    let main = thread::current();
    let t = thread::spawn(move || {
        FLAG.store(true, Ordering::Release);
        main.unpark();
    });

    while !FLAG.load(Ordering::Acquire) {
        thread::park();
    }
    t.join().unwrap();
}

fn park_timeout_loop() {
    static FLAG: AtomicBool = AtomicBool::new(false);

    let main = thread::current();
    let t = thread::spawn(move || {
        FLAG.store(true, Ordering::Release);
        main.unpark();
    });

    // `park_timeout` returns on a spurious wakeup, long before the timeout.
    while !FLAG.load(Ordering::Acquire) {
        thread::park_timeout(Duration::from_secs(3600));
    }
    t.join().unwrap();
}

fn condvar_wait_while() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair2 = Arc::clone(&pair);

    let t = thread::spawn(move || {
        let (lock, cvar) = &*pair2;
        *lock.lock().unwrap() = true;
        cvar.notify_one();
    });

    let (lock, cvar) = &*pair;
    let guard = cvar.wait_while(lock.lock().unwrap(), |ready| !*ready).unwrap();
    assert!(*guard);
    drop(guard);
    t.join().unwrap();
}

fn condvar_wait_timeout_loop() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair2 = Arc::clone(&pair);

    let t = thread::spawn(move || {
        let (lock, cvar) = &*pair2;
        *lock.lock().unwrap() = true;
        cvar.notify_one();
    });

    let (lock, cvar) = &*pair;
    let mut guard = lock.lock().unwrap();
    while !*guard {
        // A spurious wakeup is reported as "not timed out", so we must re-check the condition.
        guard = cvar.wait_timeout(guard, Duration::from_secs(3600)).unwrap().0;
    }
    drop(guard);
    t.join().unwrap();
}

fn contended_mutex() {
    let counter = Arc::new(Mutex::new(0));

    let threads: Vec<_> = (0..4)
        .map(|_| {
            let counter = Arc::clone(&counter);
            thread::spawn(move || {
                for _ in 0..10 {
                    let mut guard = counter.lock().unwrap();
                    // Hold the lock across a yield so that other threads block on it.
                    thread::yield_now();
                    *guard += 1;
                }
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(*counter.lock().unwrap(), 40);
}

fn main() {
    park_loop();
    park_timeout_loop();
    condvar_wait_while();
    condvar_wait_timeout_loop();
    contended_mutex();
}
