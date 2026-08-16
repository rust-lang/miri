// We're testing x86 target specific features
//@only-target: x86_64 i686
//@compile-flags: -C target-feature=+avxifma
//@run-native

#[cfg(target_arch = "x86")]
use std::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;
use std::mem::transmute;

fn main() {
    // AVX-IFMA is VEX-encoded and reaches the same instruction without any AVX-512.
    if !is_x86_feature_detected!("avxifma") {
        println!("warning: skipping avxifma tests");
        return;
    }

    unsafe {
        test_avxifma();
    }
}

#[target_feature(enable = "avxifma")]
unsafe fn test_avxifma() {
    // The same lanes as the AVX-512 IFMA test. Lane 0 is the stdarch test vector, lane 1
    // wraps the 64-bit accumulator, lanes 2 and 3 put garbage in the upper 12 bits of a
    // multiplicand. The 128-bit case reuses the first 2 lanes.
    const A: [u64; 4] =
        [0x00000a0000000000, 0xffffffffffffffff, 0x0000000000000000, 0x0000000000000007];
    const B: [u64; 4] =
        [0x00000b0000000004, 0x000fffffffffffff, 0x8000000000000003, 0x0000000000000001];
    const C: [u64; 4] =
        [0x00000c0000000003, 0x000fffffffffffff, 0x0000000000000001, 0x8000000000000003];
    const LO: [u64; 4] =
        [0x00005b000000000c, 0x0000000000000000, 0x0000000000000003, 0x000000000000000a];
    const HI: [u64; 4] =
        [0x00000a0840000000, 0x000ffffffffffffd, 0x0000000000000000, 0x0000000000000007];

    let a = transmute::<_, __m256i>(A);
    let b = transmute::<_, __m256i>(B);
    let c = transmute::<_, __m256i>(C);
    assert_eq_m256i(_mm256_madd52lo_avx_epu64(a, b, c), transmute::<_, __m256i>(LO));
    assert_eq_m256i(_mm256_madd52hi_avx_epu64(a, b, c), transmute::<_, __m256i>(HI));

    let a = transmute::<_, __m128i>([A[0], A[1]]);
    let b = transmute::<_, __m128i>([B[0], B[1]]);
    let c = transmute::<_, __m128i>([C[0], C[1]]);
    let lo = transmute::<_, __m128i>([LO[0], LO[1]]);
    let hi = transmute::<_, __m128i>([HI[0], HI[1]]);
    assert_eq_m128i(_mm_madd52lo_avx_epu64(a, b, c), lo);
    assert_eq_m128i(_mm_madd52hi_avx_epu64(a, b, c), hi);
}

#[track_caller]
unsafe fn assert_eq_m256i(a: __m256i, b: __m256i) {
    assert_eq!(transmute::<_, [u64; 4]>(a), transmute::<_, [u64; 4]>(b))
}

#[track_caller]
unsafe fn assert_eq_m128i(a: __m128i, b: __m128i) {
    assert_eq!(transmute::<_, [u64; 2]>(a), transmute::<_, [u64; 2]>(b))
}
