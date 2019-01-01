// FIXME: make the test suite work with dependencies
#![feature(rustc_private)]

extern crate libc;
use libc::{
    write,
    c_void,
    c_char,
    size_t,
    close,
};

extern {
    fn memfd_create(name: *const c_char, flags: size_t) -> i32;
}

fn main() {
    unsafe {
        let fd = memfd_create(b"foo\0".as_ptr() as *const c_char, 0);
        assert_eq!(3, write(fd, b"bar".as_ptr() as *const c_void, 3));
        assert_eq!(2, write(fd, b"xy".as_ptr() as *const c_void, 2));
        assert_eq!(close(fd), 0);
        assert_eq!(close(fd), -1);
        assert_eq!(-1, write(fd, b"asdf".as_ptr() as *const c_void, 4));
    }
}
