//! Make sure we detect erroneous constants post-monomorphization even when they are unused.
//! (https://github.com/rust-lang/miri/issues/1382)
// Inlining changes the error location
// compile-flags: -Zmir-opt-level=0
#![feature(const_panic)]
#![feature(never_type)]
#![warn(warnings, const_err)]

struct PrintName<T>(T);
impl<T> PrintName<T> {
    const VOID: ! = panic!(); //~WARN any use of this value will cause an error
}

fn no_codegen<T>() {
    if false {
        let _ = PrintName::<T>::VOID; //~ERROR referenced constant has errors
    }
}
fn main() {
    no_codegen::<i32>();
}
