#![allow(arithmetic_overflow)]
//@ignore-target-wasm: wasm does not support panic=unwind

fn main() {
    let _n = 1i64 >> 64;
}
