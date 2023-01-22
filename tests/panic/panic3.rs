//@ignore-target-wasm: wasm does not support panic=unwind

fn main() {
    core::panic!("panicking from libcore");
}
