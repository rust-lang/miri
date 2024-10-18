//@only-target: android  # Miri supports prctl for Android only

fn main() {
    let mut buf = vec![0u8; 15];
    unsafe {
        libc::prctl(libc::PR_GET_NAME, buf.as_mut_ptr().cast::<libc::c_char>()); //~ ERROR: Undefined Behavior: `prctl(PR_GET_NAME, name)` requires the `name` argument to be at least 16 bytes long
    }
}
