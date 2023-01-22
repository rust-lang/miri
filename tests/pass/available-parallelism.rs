//@ignore-target-wasm: wasm does not support threads

fn main() {
    assert_eq!(std::thread::available_parallelism().unwrap().get(), 1);
}
