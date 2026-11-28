//@only-target: solaris illumos # This directly tests a solarish function
//@compile-flags: -Zmiri-num-cpus=4

fn main() {
    let mut cpus = 0u32;

    assert_eq!(
        unsafe {
            libc::pset_info(libc::PS_MYID, core::ptr::null_mut(), &mut cpus, core::ptr::null_mut())
        },
        0
    );
    // `cpus` is set to the number of CPUs in the system, which we set to 4 with `-Zmiri-num-cpus`.
    assert_eq!(cpus, 4);
}
