//@ignore-target-windows: No libc on Windows
//@ignore-target-apple: `syscall` is not supported on macOS
//@ignore-target-wasm: wasm does not support panic=unwind
//@compile-flags: -Zmiri-panic-on-unsupported

fn main() {
    unsafe {
        libc::syscall(0);
    }
}
