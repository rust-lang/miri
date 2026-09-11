//@only-target: aarch64
//@rustc-env: MIRI_DISABLE_UNSUPPORTED_TARGET_FEATURES=1

use std::arch::is_aarch64_feature_detected;

fn main() {
    assert!(!cfg!(target_feature = "aes"));
    assert!(!is_aarch64_feature_detected!("aes"));
}
