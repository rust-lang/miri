//@only-target-linux

fn main() {
    // [The GNU manual notes](https://www.gnu.org/software/gnulib/manual/html_node/Glibc-gnu_002flibc_002dversion_002eh.html):
    // > This function is missing on some platforms: macOS 11.1, FreeBSD 13.0, NetBSD 9.0,
    // > OpenBSD 6.7, Minix 3.1.8, AIX 5.1, HP-UX 11, IRIX 6.5, Solaris 11.4, Cygwin 2.9, mingw, MSVC 14, Android 9.0.
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CStr;
        // Release name.
        let x = unsafe { libc::gnu_get_libc_release() };
        assert!(!x.is_null());
        let ret = unsafe { CStr::from_ptr(x.as_ptr()) };
        let ret = ret.to_str().expect("Ctr to str");
        assert!(ret.len() > 0);

        // Version string.
        let x = unsafe { libc::gnu_get_libc_version() };
        assert!(!x.is_null());
        let ret = unsafe { CStr::from_ptr(x.as_ptr()) };
        let ret = ret.to_str().expect("Ctr to str");
        assert!(ret.len() > 0);
    }
}
