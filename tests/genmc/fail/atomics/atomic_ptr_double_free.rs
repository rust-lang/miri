//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// Test that double-free bugs involving atomic pointers are detected in GenMC mode.

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::alloc::{Layout, alloc, dealloc};
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::Ordering::*;

use crate::genmc::*;

static X: AtomicPtr<u64> = AtomicPtr::new(std::ptr::null_mut());

unsafe fn free(ptr: *mut u64) {
    dealloc(ptr as *mut u8, Layout::new::<u64>()) //~ ERROR: Undefined Behavior
}

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    unsafe {
        let ids = [
            spawn_pthread_closure(|| {
                let a: *mut u64 = alloc(Layout::new::<u64>()) as *mut u64;
                X.store(a, SeqCst);
                // If the other thread runs here, there will be a double free.
                let b = X.swap(std::ptr::null_mut(), SeqCst);
                if b.is_null() {
                    std::process::abort();
                }
                free(b);
            }),
            spawn_pthread_closure(|| {
                let b = X.load(SeqCst);
                if !b.is_null() {
                    free(b);
                }
            }),
        ];
        join_pthreads(ids);
        0
    }
}
