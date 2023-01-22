//@ignore-target-windows: No libc on Windows
//@ignore-target-wasm: wasm does not support threads

fn main() {
    let rw = std::cell::UnsafeCell::new(libc::PTHREAD_RWLOCK_INITIALIZER);
    unsafe {
        libc::pthread_rwlock_unlock(rw.get()); //~ ERROR: was not locked
    }
}
