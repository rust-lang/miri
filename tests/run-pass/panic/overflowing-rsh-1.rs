// ignore-windows: Unwind panicking does not currently work on Windows
#![allow(arithmetic_overflow, const_err)]

fn main() {
    let _n = 1i64 >> 64;
}
