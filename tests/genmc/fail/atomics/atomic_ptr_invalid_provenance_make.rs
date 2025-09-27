//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// Test that we can distinguish two pointers with the same address, but different provenance, after they are sent to GenMC and back.
// We have two variants, one where we send such a pointer to GenMC, and one where we make it on the GenMC side.

#![no_main]

use std::sync::atomic::AtomicPtr;
use std::sync::atomic::Ordering::*;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    unsafe {
        let atomic_ptr = AtomicPtr::new(std::ptr::null_mut());
        let mut a = Box::new(0u64);
        let mut b = Box::new(0u64);
        // FIXME: use `Box::as_mut_ptr` once stabilized (`https://github.com/rust-lang/rust/issues/129090`).
        let a_ptr: *mut u64 = a.as_mut() as *mut u64;
        let b_ptr: *mut u64 = b.as_mut() as *mut u64;

        // Store valid pointer to `a`:
        atomic_ptr.store(a_ptr, Relaxed);
        let ptr = atomic_ptr.load(Relaxed);
        *ptr = 42;
        if *a != 42 {
            std::process::abort();
        }
        // Store valid pointer to `b`:
        atomic_ptr.store(b_ptr, Relaxed);
        let ptr = atomic_ptr.load(Relaxed);
        *ptr = 43;
        if *b != 43 {
            std::process::abort();
        }

        // Create a pointer with the provenance of `b_ptr`, but the address of `a_ptr` on the GenMC side.
        atomic_ptr.store(b_ptr, Relaxed);
        atomic_ptr.fetch_byte_add(a_ptr as usize, Relaxed);
        atomic_ptr.fetch_byte_sub(b_ptr as usize, Relaxed);
        let ptr = atomic_ptr.load(Relaxed);
        if a_ptr as usize != ptr as usize {
            std::process::abort();
        }
        *ptr = 44; //~ ERROR: Undefined Behavior

        0
    }
}
