//@compile-flags: -Zmiri-genmc

#![no_main]

const N: usize = 1;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let handles: Vec<_> = (0..N).map(|_| std::thread::spawn(thread_func)).collect();
    handles.into_iter().for_each(|handle| handle.join().unwrap());

    0
}

fn thread_func() {}
