//@compile-flags: -Zmiri-panic-on-unsupported
//@ignore-target-wasm: wasm does not support panic=unwind
//@normalize-stderr-test: "OS `.*`" -> "$$OS"

fn main() {
    extern "Rust" {
        fn foo();
    }

    unsafe {
        foo();
    }
}
