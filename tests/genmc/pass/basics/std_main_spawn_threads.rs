//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

const N: usize = 2;

fn main() {
    let handles: Vec<_> = (0..N).map(|_| std::thread::spawn(thread_func)).collect();
    handles.into_iter().for_each(|handle| handle.join().unwrap());
}

fn thread_func() {}
