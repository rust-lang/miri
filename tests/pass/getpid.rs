//@compile-flags: -Zmiri-disable-isolation
//@ignore-target-wasm: wasm does not support threads

fn getpid() -> u32 {
    std::process::id()
}

fn main() {
    getpid();
}
