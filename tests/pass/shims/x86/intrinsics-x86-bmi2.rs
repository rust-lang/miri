// Ignore everything except x86 and x86_64
// Any new targets that are added to CI should be ignored here.
// (We cannot use `cfg`-based tricks here since the `target-feature` flags below only work on x86.)
//@ignore-target-aarch64
//@ignore-target-arm
//@ignore-target-avr
//@ignore-target-s390x
//@ignore-target-thumbv7em
//@ignore-target-wasm32
//@compile-flags: -C target-feature=+bmi2

#[cfg(target_arch = "x86")]
use std::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

fn main() {
    assert!(is_x86_feature_detected!("bmi2"));

    unsafe {
        run_x86();
        #[cfg(target_arch = "x86_64")]
        run_x86_64();
    };
}

#[target_feature(enable = "bmi2")]
unsafe fn run_x86() {
    assert_eq!(_pdep_u32(0x00012567, 0xff00fff0), 0x12005670);

    assert_eq!(_pext_u32(0x12345678, 0xff00fff0), 0x00012567);
}

#[cfg(target_arch = "x86_64")]
unsafe fn run_x86_64() {
    assert_eq!(_pdep_u64(0x00012567, 0xff00fff0), 0x12005670);
    assert_eq!(_pdep_u64(0x0000_0134_5678_9CDE, 0xff0f_ffff_ff00_fff0), 0x0103_4567_8900_CDE0);

    assert_eq!(_pext_u64(0x12345678, 0xff00fff0), 0x00012567);
    assert_eq!(_pext_u64(0x0123_4567_89AB_CDEF, 0xff0f_ffff_ff00_fff0), 0x0000_0134_5678_9CDE);
}
