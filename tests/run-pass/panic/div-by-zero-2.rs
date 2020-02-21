// ignore-windows: Unwind panicking does not currently work on Windows
#![allow(unconditional_panic, const_err)]

fn main() {
    let _n = 1 / 0;
}
