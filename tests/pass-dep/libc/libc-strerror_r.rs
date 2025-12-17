//@ignore-target: windows # Supported only on unixes

#[path = "../../utils/libc.rs"]
mod libc_utils;
use libc_utils::errno_check;

fn main() {
    unsafe {
        let mut buf = vec![0u8; 32];
        errno_check(libc::strerror_r(libc::EPERM, buf.as_mut_ptr().cast(), buf.len()));
        let mut buf2 = vec![0u8; 64];
        errno_check(libc::strerror_r(-1i32, buf2.as_mut_ptr().cast(), buf2.len()));
        // This buffer is deliberately too small so this triggers ERANGE.
        let mut buf3 = vec![0u8; 2];
        assert_eq!(
            libc::strerror_r(libc::E2BIG, buf3.as_mut_ptr().cast(), buf3.len()),
            libc::ERANGE
        );
    }
}
