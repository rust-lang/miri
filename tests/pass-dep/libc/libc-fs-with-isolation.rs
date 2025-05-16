//@ignore-target: windows # File handling is not implemented yet

fn main() {
    // test `fcntl(F_DUPFD): should work even with isolation.`
    unsafe {
        assert!(libc::fcntl(1, libc::F_DUPFD, 0) >= 0);
    }
}
