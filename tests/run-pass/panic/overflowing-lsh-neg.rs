// ignore-windows: Unwind panicking does not currently work on Windows
#![allow(arithmetic_overflow, const_err)]

fn main() {
    let _n = 2i64 << -1;
}
