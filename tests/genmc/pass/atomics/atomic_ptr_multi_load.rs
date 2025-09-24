//@ revisions: ptr08 ptr16 ptr32 ptr64
//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// Test that we can pass pointers to different allocation types to GenMC, including pointers not pointing to the start of the allocation.
// There should be one explored execution where the second thread read null, and one execution per possible pointer to read.

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::alloc::{Layout, alloc, dealloc};
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::Ordering::*;

use genmc::*;

#[cfg(ptr08)]
type Int = u8;
#[cfg(ptr16)]
type Int = u16;
#[cfg(ptr32)]
type Int = u32;
#[cfg(not(any(ptr08, ptr16, ptr32)))]
type Int = u64;

static PTR: AtomicPtr<Int> = AtomicPtr::new(std::ptr::null_mut());

// Test different static allocations.
static mut X: Int = 0;
static mut Y: [Int; 4] = [0; 4];
static mut Z: (Int, Int, Int) = (0, 0, 0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    // FIXME(genmc,HACK): remove this initializing write once Miri-GenMC supports mixed atomic-non-atomic accesses.
    PTR.store(std::ptr::null_mut(), SeqCst);

    unsafe {
        // Test heap allocation.
        let a = alloc(Layout::new::<Int>()) as *mut Int;
        *a = 0;
        // Test stack allocation.
        let mut b = 0;
        let mut is_null = false;
        let ids = [
            spawn_pthread_closure(|| {
                PTR.store(a, Relaxed);
                PTR.store(&raw mut b, Relaxed);
                PTR.store(&raw mut X, Relaxed);
                PTR.store(&raw mut Y[2], Relaxed);
                PTR.store(&raw mut Z.1, Relaxed);
            }),
            spawn_pthread_closure(|| {
                let ptr = PTR.load(Relaxed);
                if ptr.is_null() {
                    is_null = true;
                } else {
                    *ptr = 42;
                }
            }),
        ];
        join_pthreads(ids);
        // Either the second thread read null, or one value must be updated now.
        if !is_null && *a != 42 && b != 42 && X != 42 && Y[2] != 42 && Z.1 != 42 {
            std::process::abort();
        }
        dealloc(a as *mut u8, Layout::new::<Int>());
        0
    }
}
