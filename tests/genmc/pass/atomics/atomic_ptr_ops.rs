//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// Test several operations on atomic pointers.

#![no_main]

use std::sync::atomic::AtomicPtr;
use std::sync::atomic::Ordering::*;

static mut X: u64 = 0;
static mut Y: u64 = 0;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    unsafe {
        let atomic_ptr: AtomicPtr<u64> = AtomicPtr::new(&raw mut X);
        // FIXME(genmc,HACK): remove this initializing write once Miri-GenMC supports mixed atomic-non-atomic accesses.
        atomic_ptr.store(&raw mut X, SeqCst);

        // Load from AtomicPtr and use the returned pointer to update the correct memory.
        let x_ptr = atomic_ptr.load(SeqCst);
        *x_ptr = 10;
        if X != 10 {
            std::process::abort();
        }
        // Store to the AtomicPtr, check that future load return the correct pointer.
        atomic_ptr.store(&raw mut Y, SeqCst);
        Y = 42;
        let y_ptr = atomic_ptr.load(SeqCst);
        if *y_ptr != 42 {
            std::process::abort();
        }
        *y_ptr = 1234; // This should not modify X.
        if Y != 1234 || X != 10 {
            std::process::abort();
        }
        // Atomc swap must return the old value, and the pointer must still be usable.
        let a = atomic_ptr.swap(&raw mut X, SeqCst);
        if a != y_ptr || *a != *y_ptr {
            std::process::abort();
        }
        *a = *y_ptr;

        // Test a failing compare-exchange (we swapped in X above).
        match atomic_ptr.compare_exchange(
            y_ptr, // wrong, it should be `x_ptr`, so this should never succeed
            std::ptr::dangling_mut(),
            SeqCst,
            SeqCst,
        ) {
            Ok(_ptr) => std::process::abort(),
            Err(ptr) =>
                if ptr != x_ptr || *ptr != *x_ptr {
                    std::process::abort();
                } else {
                    *ptr = *ptr;
                },
        }

        // Test that pointing to the middle of an allocation also works correctly.
        let mut array: [u64; 10] = [0xAAAA; 10];
        match atomic_ptr.compare_exchange(x_ptr, &raw mut array[2], SeqCst, SeqCst) {
            Ok(ptr) if ptr == x_ptr => {}
            _ => std::process::abort(),
        }
        let ptr = atomic_ptr.load(SeqCst);
        if ptr != &raw mut array[2] {
            std::process::abort();
        }
        // Updates to the pointer or the original should be seen by the other one.
        *ptr = 0xB;
        if array[2] != 0xB {
            std::process::abort();
        }
        array[2] = 0xC;
        if *ptr != 0xC {
            std::process::abort();
        }
        // The other array elements should be unchanged.
        for (i, x) in array.iter().enumerate() {
            if i != 2 && *x != 0xAAAA {
                std::process::abort();
            }
        }
    }
    0
}
