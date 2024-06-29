// libc::read should initialize its buffer
//@ignore-target-windows: we have no file deletion on Windows
//@compile-flags: -Zmiri-disable-isolation

use std::ffi::CString;
use std::fs::remove_file;
use std::mem::MaybeUninit;

#[path = "../../utils/mod.rs"]
mod utils;

fn main() {
    let path =
        utils::prepare_with_content("fail-libc-read-and-uninit-premature-eof.txt", &[1u8, 2, 3]);
    let cpath = CString::new(path.clone().into_os_string().into_encoded_bytes()).unwrap();
    unsafe {
        let fd = libc::open(cpath.as_ptr(), libc::O_RDONLY);
        assert_ne!(fd, -1);
        let mut buf: MaybeUninit<[u8; 4]> = std::mem::MaybeUninit::uninit();
        assert_eq!(libc::read(fd, buf.as_mut_ptr().cast::<std::ffi::c_void>(), 4), 3);
        buf.assume_init(); //~ERROR: Undefined Behavior: constructing invalid value at .value[3]: encountered uninitialized memory, but expected an integer
        assert_eq!(libc::close(fd), 0);
    }
    remove_file(&path).unwrap();
}
