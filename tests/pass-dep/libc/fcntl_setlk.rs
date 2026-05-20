//@only-target: solaris illumos # fcntl(F_SETLK) locking shim is gated to Solaris and Illumos
//@compile-flags: -Zmiri-disable-isolation

use std::fs::OpenOptions;
use std::io::Error;
use std::os::fd::AsRawFd;

#[path = "../../utils/mod.rs"]
mod utils;

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

    assert_eq!(unsafe { libc::fcntl(fd1, libc::F_SETLK, &wrlck) }, 0);

    // Attempting to take a second exclusive lock should fail
    unsafe {
        assert_eq!(libc::fcntl(fd2, libc::F_SETLK, &wrlck), -1);
        assert_eq!(Error::last_os_error().raw_os_error(), Some(libc::EAGAIN));
    }

    assert_eq!(unsafe { libc::fcntl(fd1, libc::F_SETLK, &unlck) }, 0);

    assert_eq!(unsafe { libc::fcntl(fd1, libc::F_SETLKW, &rdlck) }, 0);
    assert_eq!(unsafe { libc::fcntl(fd2, libc::F_SETLKW, &rdlck) }, 0);

    assert_eq!(unsafe { libc::fcntl(fd1, libc::F_SETLK, &unlck) }, 0);
    assert_eq!(unsafe { libc::fcntl(fd2, libc::F_SETLK, &unlck) }, 0);
}
