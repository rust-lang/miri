//@compile-flags: -Zmiri-genmc

#![no_main]

use std::sync::atomic::*;

static mut X: u64 = 0;
static mut Y: u64 = 0;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    unsafe {
        let atomic_ptr: AtomicPtr<u64> = AtomicPtr::new(&raw mut X);

        let x_ptr = atomic_ptr.load(Ordering::SeqCst);
        *x_ptr = 10;
        if X != 10 {
            std::process::abort();
        }
        atomic_ptr.store(&raw mut Y, Ordering::SeqCst);
        Y = 42;
        let y_ptr = atomic_ptr.load(Ordering::SeqCst);
        if *y_ptr != 42 {
            std::process::abort();
        }
        *y_ptr = 1234;
        if Y != 1234 {
            std::process::abort();
        } else if X != 10 {
            std::process::abort();
        }
        let y_ptr_ = atomic_ptr.swap(&raw mut X, Ordering::SeqCst);
        if y_ptr_ != y_ptr {
            std::process::abort();
        }
        // To make sure also the provenance info is correctly restored, we need to use the pointers:
        if *y_ptr_ != *y_ptr {
            std::process::abort();
        }
        *y_ptr_ = *y_ptr;

        match atomic_ptr.compare_exchange(
            y_ptr, // wrong, it should be `x_ptr`, so this should never succeed
            std::ptr::dangling_mut(),
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_ptr) => std::process::abort(),
            Err(ptr) =>
                if ptr != x_ptr {
                    std::process::abort();
                } else if *ptr != *x_ptr {
                    std::process::abort();
                } else {
                    *ptr = *ptr;
                },
        }

        let mut array: [u64; 10] = [0xAAAA; 10];
        match atomic_ptr.compare_exchange(
            x_ptr,
            &raw mut array[2],
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(ptr) =>
                if ptr != x_ptr {
                    std::process::abort();
                },
            Err(_ptr) => std::process::abort(),
        }
        let ptr = atomic_ptr.load(Ordering::SeqCst);
        *ptr = 0xB;
        if array[2] != 0xB {
            std::process::abort();
        }
        array[2] = 0xC;
        if *ptr != 0xC {
            std::process::abort();
        }
    }
    0
}
