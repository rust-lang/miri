//@only-target: never # disabled due to https://github.com/rust-lang/miri/issues/5678

// We should be able to spawn and join standard library threads in GenMC mode.
// Since these threads do nothing, we should only explore 1 program execution.

const N: usize = 2;

fn main() {
    let handles: Vec<_> = (0..N).map(|_| std::thread::spawn(thread_func)).collect();
    handles.into_iter().for_each(|handle| handle.join().unwrap());
}

fn thread_func() {}
