//@compile-flags: -Zmiri-always-two-phase

fn main() {
    let data = &mut [0, 1];
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), data.as_mut_ptr().add(1), 1);
    }
    assert_eq!(data, &[0, 0]);
}
