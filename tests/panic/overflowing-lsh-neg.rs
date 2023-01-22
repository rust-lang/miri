#![allow(arithmetic_overflow)]
//@ignore-target-wasm: wasm does not support panic=unwind

fn main() {
    let _n = 2i64 << -1;
}
