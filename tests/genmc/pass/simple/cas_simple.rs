//@compile-flags: -Zmiri-genmc

#![no_main]

use std::sync::atomic::*;

static VALUE: AtomicUsize = AtomicUsize::new(0);

const SUCCESS_ORD: Ordering = Ordering::SeqCst;
const FAILURE_ORD: Ordering = Ordering::SeqCst;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    VALUE.store(1, SUCCESS_ORD);

    let current = 1;
    let new_value = 2;
    // Expect success:
    match VALUE.compare_exchange(current, new_value, SUCCESS_ORD, FAILURE_ORD) {
        Ok(old_value) =>
            if old_value != current {
                std::process::abort();
            },
        Err(_value) => std::process::abort(),
    }

    if new_value != VALUE.load(SUCCESS_ORD) {
        std::process::abort()
    }

    let dummy_value = 42;
    let wrong_value = 1234;

    // Expect failure:
    match VALUE.compare_exchange(wrong_value, dummy_value, SUCCESS_ORD, FAILURE_ORD) {
        Ok(_old_value) => std::process::abort(),
        Err(old_value) =>
            if old_value != new_value {
                std::process::abort();
            },
    }

    if new_value != VALUE.load(SUCCESS_ORD) {
        std::process::abort()
    }
    0
}
