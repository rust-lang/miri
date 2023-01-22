//@rustc-env: RUST_BACKTRACE=1
//@ignore-target-wasm: wasm does not support panic=unwind
//@compile-flags: -Zmiri-disable-isolation

fn main() {
    std::panic!("panicking from libstd");
}
