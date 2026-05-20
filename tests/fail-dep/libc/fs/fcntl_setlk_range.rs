//@only-target: solaris illumos # fcntl(F_SETLK) locking shim is gated to Solaris and Illumos
//@compile-flags: -Zmiri-disable-isolation

use std::fs::OpenOptions;
use std::os::fd::AsRawFd;

#[path = "../../../utils/mod.rs"]
mod utils;

fn main() {
    // flock only supports whole-file locks while fcntl supports range-based locking
    // Our miri shim translates fcntl calls to flock calls, therefore we disallow range locks
    let path = utils::prepare_with_content("miri_fcntl_range_lock.txt", b"hello");
    let file = OpenOptions::new().read(true).write(true).open(&path).unwrap();

    let mut fl: libc::flock = unsafe { std::mem::zeroed() };
    fl.l_type = libc::F_WRLCK as libc::c_short;
    fl.l_whence = libc::SEEK_SET as libc::c_short;
    fl.l_start = 0;
    fl.l_len = 100; // non-zero length = partial range, deliberately rejected by the shim

    unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &fl) };
    //~^ ERROR: unsupported operation: fcntl: range locks are not supported
}
