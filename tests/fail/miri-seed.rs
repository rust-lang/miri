//@compile-flags: -Zmiri-seed=9223372036854775807
#![feature(core_intrinsics)]

fn main() {
    unsafe {
        core::intrinsics::breakpoint() //~ ERROR: trace/breakpoint trap
    };
}
