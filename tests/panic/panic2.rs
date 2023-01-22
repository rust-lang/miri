//@ignore-target-wasm: wasm does not support panic=unwind

fn main() {
    std::panic!("{}-panicking from libstd", 42);
}
