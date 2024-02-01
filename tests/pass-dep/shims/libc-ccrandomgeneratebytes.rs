//@ignore-target-windows: no libc
//@ignore-target-linux: macOs only
//@ignore-target-freebsd: macOs only

fn main() {
    let mut buf = [0u8; 1024];

    unsafe {
        assert_eq!(
            libc::CCRandomGenerateBytes(buf.as_mut_ptr() as *mut libc::c_void, buf.len()),
            libc::kCCSuccess
        );
    }
}
