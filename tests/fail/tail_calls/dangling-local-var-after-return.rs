#![feature(explicit_tail_calls)]
#![allow(incomplete_features)]

fn g(x: *const i32, values: [u8; 32]) -> *const i32 {
    // Non-scalar arguments must be copied before the caller's locals are deallocated.
    assert_eq!(values, [42; 32]);
    x
}

fn f(_x: *const i32, _values: [u8; 32]) -> *const i32 {
    let local = 42;
    become g(&raw const local, [42; 32])
}

fn main() {
    let ptr = f(std::ptr::null(), [0; 32]);
    // The deallocation span must survive the tail callee returning as well.
    let _val = unsafe { *ptr }; //~ERROR: has been freed, so this pointer is dangling
}
