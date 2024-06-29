// libc::read should initialize its buffer
//@ignore-target-windows: we have no file deletion on Windows
//@compile-flags: -Zmiri-disable-isolation

use std::ffi::CString;
use std::fs::remove_file;
use std::mem::MaybeUninit;

#[path = "../../utils/mod.rs"]
mod utils;

fn main() {
    {
        let path = utils::prepare_with_content("pass-libc-read-and-uninit.txt", &[1u8, 2, 3]);
        let cpath = CString::new(path.clone().into_os_string().into_encoded_bytes()).unwrap();
        unsafe {
            let fd = libc::open(cpath.as_ptr(), libc::O_RDONLY);
            assert_ne!(fd, -1);
            let mut buf: MaybeUninit<[u8; 2]> = std::mem::MaybeUninit::uninit();
            assert_eq!(libc::read(fd, buf.as_mut_ptr().cast::<std::ffi::c_void>(), 2), 2);
            let buf = buf.assume_init();
            assert_eq!(buf, [1, 2]);
            assert_eq!(libc::close(fd), 0);
        }
        remove_file(&path).unwrap();
    }
    /*{
        let path = utils::prepare_with_content("pass-libc-read-and-uninit-2.txt", &[1u8, 2, 3]);
        let cpath = CString::new(path.clone().into_os_string().into_encoded_bytes()).unwrap();
        unsafe {
            let fd = libc::open(cpath.as_ptr(), libc::O_RDONLY);
            assert_ne!(fd, -1);
            let mut buf = [42u8; 5];
            assert_eq!(libc::read(fd, buf.as_mut_ptr().cast::<std::ffi::c_void>(), 4), 3);
            assert_eq!(buf, [1, 2, 3, 42, 42]);
            assert_eq!(libc::close(fd), 0);
        }
        remove_file(&path).unwrap();
    }*/
}
