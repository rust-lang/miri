// ignore-windows: Concurrency on Windows is not supported yet.

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread::spawn;

#[derive(Copy, Clone)]
struct EvilSend<T>(pub T);

unsafe impl<T> Send for EvilSend<T> {}
unsafe impl<T> Sync for EvilSend<T> {}

pub fn main() {
    let mut a = AtomicUsize::new(0);
    let b = &mut a as *mut AtomicUsize;
    let c = EvilSend(b);
    unsafe {
        let j1 = spawn(move || {
            let atomic_ref = &mut *c.0;
            atomic_ref.load(Ordering::SeqCst)
        });

        let j2 = spawn(move || {
            let atomic_ref = &mut *c.0;
            *atomic_ref.get_mut() = 32; //~ ERROR Data race detected between Write on Thread(id = 2) and Atomic Load on Thread(id = 1)
        });

        j1.join().unwrap();
        j2.join().unwrap();
    }
}
