//@ignore-target: windows # No libc on Windows target
//@ignore-target: solaris # Does not have flock
//@compile-flags: -Zmiri-disable-isolation
//@run-native
//@only-host: windows # The unsup errors fire only on Windows hosts
//@revisions: shared_to_exclusive exclusive_to_shared

use std::fs::File;
use std::os::fd::AsRawFd;

#[path = "../../utils/libc.rs"]
mod libc_utils;
#[path = "../../utils/mod.rs"]
mod utils;
use libc_utils::*;

fn main() {
    let bytes = b"Hello, World!\n";
    let path = utils::prepare_with_content("miri_test_fs_flock_conversion.txt", bytes);

    let file = File::open(&path).unwrap();
    let fd = file.as_raw_fd();

    #[cfg(shared_to_exclusive)]
    {
        errno_check(unsafe { libc::flock(fd, libc::LOCK_SH) });
        // Converting a shared lock to an exclusive lock is unsupported on Windows hosts.
        unsafe {
            libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB);
            //~[shared_to_exclusive] ERROR: unsupported operation: converting shared `flock` to exclusive is not supported on Windows hosts
        }
    }

    #[cfg(exclusive_to_shared)]
    {
        errno_check(unsafe { libc::flock(fd, libc::LOCK_EX) });
        // Converting an exclusive lock to a shared lock is unsupported on Windows hosts.
        unsafe {
            libc::flock(fd, libc::LOCK_SH | libc::LOCK_NB);
            //~[exclusive_to_shared] ERROR: unsupported operation: converting exclusive `flock` to shared is not supported on Windows hosts
        }
    }
}