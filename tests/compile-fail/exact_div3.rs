#![feature(core_intrinsics)]
fn main() {
    // signed divison with a remainder
    unsafe { std::intrinsics::exact_div(-19i8, 2); } //~ ERROR -19 cannot be divided by 2 without remainder
}
