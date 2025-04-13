// We want to control preemption here. Stacked borrows interferes by having its own accesses.
//@compile-flags: -Zmiri-fixed-schedule -Zmiri-disable-stacked-borrows
// Avoid accidental synchronization via address reuse inside `thread::spawn`.
//@compile-flags: -Zmiri-address-reuse-cross-thread-rate=0

#![feature(rustc_attrs)]

use std::thread::spawn;

#[derive(Copy, Clone)]
struct EvilSend<T>(pub T);

unsafe impl<T> Send for EvilSend<T> {}
unsafe impl<T> Sync for EvilSend<T> {}

extern "Rust" {
    #[rustc_std_internal_symbol]
    fn __rust_dealloc(ptr: *mut u8, size: usize, align: usize);
}

pub fn main() {
    // Shared atomic pointer
    let pointer: *mut usize = Box::into_raw(Box::new(0usize));
    let ptr = EvilSend(pointer);

    unsafe {
        let j1 = spawn(move || {
            let ptr = ptr; // avoid field capturing
            let _val = *ptr.0;
        });

        let j2 = spawn(move || {
            let ptr = ptr; // avoid field capturing
            __rust_dealloc(
                //~^ ERROR: Data race detected between (1) non-atomic read on thread `unnamed-1` and (2) deallocation on thread `unnamed-2`
                ptr.0 as *mut _,
                std::mem::size_of::<usize>(),
                std::mem::align_of::<usize>(),
            );
        });

        j1.join().unwrap();
        j2.join().unwrap();
    }
}
