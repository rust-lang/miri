//@ignore-target: windows # no fcntl on Windows
//@compile-flags: -Zmiri-disable-isolation

use std::fs::OpenOptions;
use std::os::fd::AsRawFd;

#[path = "../../utils/mod.rs"]
mod utils;

#[path = "../../utils/libc.rs"]
mod libc_utils;
use libc_utils::*;

fn make_flock(l_type: libc::c_short) -> libc::flock {
    let mut fl: libc::flock = unsafe { std::mem::zeroed() };
    fl.l_type = l_type;
    fl.l_whence = libc::SEEK_SET as libc::c_short;
    fl
}

fn main() {
    let path = utils::prepare_with_content("miri_fcntl_setlk.txt", b"hello");
    let file1 = OpenOptions::new().read(true).write(true).open(&path).unwrap();
    let file2 = OpenOptions::new().read(true).write(true).open(&path).unwrap();
    let fd1 = file1.as_raw_fd();
    let fd2 = file2.as_raw_fd();

    let wrlck = make_flock(libc::F_WRLCK as libc::c_short);
    let rdlck = make_flock(libc::F_RDLCK as libc::c_short);
    let unlck = make_flock(libc::F_UNLCK as libc::c_short);

    errno_check(unsafe { libc::fcntl(fd1, libc::F_SETLK, &wrlck) });

    // Re-acquiring the same lock on the same FD should succeed (no-op).
    errno_check(unsafe { libc::fcntl(fd1, libc::F_SETLK, &wrlck) });

    // Downgrading to a read lock on the same FD should also succeed.
    errno_check(unsafe { libc::fcntl(fd1, libc::F_SETLK, &rdlck) });

    // Upgrade back to an exclusive lock
    errno_check(unsafe { libc::fcntl(fd1, libc::F_SETLK, &wrlck) });

    // Attempting to take an exclusive lock from a different FD should fail, due to flock-backed emulation.
    unsafe {
        let err = errno_result(libc::fcntl(fd2, libc::F_SETLK, &wrlck)).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::EAGAIN));
    }

    errno_check(unsafe { libc::fcntl(fd1, libc::F_SETLK, &unlck) });

    errno_check(unsafe { libc::fcntl(fd1, libc::F_SETLKW, &rdlck) });
    errno_check(unsafe { libc::fcntl(fd2, libc::F_SETLKW, &rdlck) });

    errno_check(unsafe { libc::fcntl(fd1, libc::F_SETLK, &unlck) });
    errno_check(unsafe { libc::fcntl(fd2, libc::F_SETLK, &unlck) });

    // Unlocking an already-unlocked FD should succeed (no-op).
    errno_check(unsafe { libc::fcntl(fd1, libc::F_SETLK, &unlck) });
}
