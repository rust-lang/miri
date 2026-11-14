//@revisions: with_isolation without_isolation
//@[without_isolation] compile-flags: -Zmiri-disable-isolation

fn getpid() -> u32 {
    let pid = std::process::id();
    #[cfg(windows)]
    {
        extern "system" {
            fn GetCurrentProcessId() -> u32;
        }

        let windows_pid = unsafe { GetCurrentProcessId() };
        assert_ne!(windows_pid, 0);
        assert_eq!(windows_pid, pid);
    }
    pid
}

fn main() {
    let pid = getpid();

    std::thread::spawn(move || {
        assert_eq!(getpid(), pid);
    })
    .join()
    .unwrap();

    // Test that in isolation mode a deterministic value will be returned.
    // The value 1000 is not important, we only care that whatever the value
    // is, won't change from execution to execution.
    #[cfg(with_isolation)]
    assert_eq!(pid, 1000);
}
