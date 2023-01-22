#![allow(unconditional_panic)]
//@ignore-target-wasm: wasm does not support panic=unwind

fn main() {
    let _n = 1 / 0;
}
