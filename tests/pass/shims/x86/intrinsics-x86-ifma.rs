//@revisions: avx512 avx512vl avx
//@only-target: x86_64 i686
//@[avx512]compile-flags: -C target-feature=+avx512ifma,-avx512vl,-avxifma
//@[avx512vl]compile-flags: -C target-feature=+avx512ifma,+avx512vl,-avxifma
//@[avx]compile-flags: -C target-feature=+avxifma,-avx512f
//@run-native

#[cfg(target_arch = "x86")]
use std::arch::x86 as arch;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64 as arch;
use std::mem::transmute;

use arch::*;
#[cfg(avx)]
use arch::{
    _mm_madd52hi_avx_epu64 as madd52hi_128, _mm_madd52lo_avx_epu64 as madd52lo_128,
    _mm256_madd52hi_avx_epu64 as madd52hi_256, _mm256_madd52lo_avx_epu64 as madd52lo_256,
};
#[cfg(avx512vl)]
use arch::{
    _mm_madd52hi_epu64 as madd52hi_128, _mm_madd52lo_epu64 as madd52lo_128,
    _mm256_madd52hi_epu64 as madd52hi_256, _mm256_madd52lo_epu64 as madd52lo_256,
};

#[rustfmt::skip]
const A: [u64; 8] = [
    0x00000a0000000000, 0xffffffffffffffff, 0x0000000000000000, 0x0000000000000007,
    0x0000000000000000, 0x0000000000000000, 0xffffffffffffffff, 0x8000000000000000,
];
#[rustfmt::skip]
const B: [u64; 8] = [
    0x00000b0000000004, 0x000fffffffffffff, 0xfff0000000000003, 0x0000000000000001,
    0x000fffffffffffff, 0x0000000000000000, 0x0000000000000002, 0x0008000000000001,
];
#[rustfmt::skip]
const C: [u64; 8] = [
    0x00000c0000000003, 0x000fffffffffffff, 0x0000000000000001, 0xfff0000000000003,
    0x000fffffffffffff, 0x0000000000000000, 0x0000000000000003, 0x0008000000000007,
];
#[rustfmt::skip]
const LO: [u64; 8] = [
    0x00005b000000000c, 0x0000000000000000, 0x0000000000000003, 0x000000000000000a,
    0x0000000000000001, 0x0000000000000000, 0x0000000000000005, 0x8000000000000007,
];
#[rustfmt::skip]
const HI: [u64; 8] = [
    0x00000a0840000000, 0x000ffffffffffffd, 0x0000000000000000, 0x0000000000000007,
    0x000ffffffffffffe, 0x0000000000000000, 0xffffffffffffffff, 0x8004000000000004,
];

fn main() {
    #[cfg(avx)]
    if !is_x86_feature_detected!("avxifma") {
        println!("warning: skipping avxifma tests");
        return;
    }
    #[cfg(any(avx512, avx512vl))]
    if !is_x86_feature_detected!("avx512ifma") {
        println!("warning: skipping avx512ifma tests");
        return;
    }
    #[cfg(avx512vl)]
    if !is_x86_feature_detected!("avx512vl") {
        println!("warning: skipping avx512vl tests");
        return;
    }

    #[cfg(miri)]
    {
        #[cfg(avx)]
        assert!(!is_x86_feature_detected!("avx512f"));
        #[cfg(avx512)]
        assert!(!is_x86_feature_detected!("avx512vl"));
        #[cfg(any(avx512, avx512vl))]
        assert!(!is_x86_feature_detected!("avxifma"));
    }

    unsafe {
        #[cfg(any(avx512, avx512vl))]
        test_512();
        #[cfg(any(avx, avx512vl))]
        {
            test_256();
            test_128();
        }
    }
}

#[cfg(any(avx512, avx512vl))]
#[target_feature(enable = "avx512ifma")]
unsafe fn test_512() {
    let a = transmute::<_, __m512i>(A);
    let b = transmute::<_, __m512i>(B);
    let c = transmute::<_, __m512i>(C);
    let lo = _mm512_madd52lo_epu64(a, b, c);
    let hi = _mm512_madd52hi_epu64(a, b, c);
    assert_eq!(transmute::<_, [u64; 8]>(lo), LO);
    assert_eq!(transmute::<_, [u64; 8]>(hi), HI);
}

#[cfg(any(avx, avx512vl))]
#[cfg_attr(avx, target_feature(enable = "avxifma"))]
#[cfg_attr(avx512vl, target_feature(enable = "avx512ifma,avx512vl"))]
unsafe fn test_256() {
    for offset in (0..A.len()).step_by(4) {
        let a = transmute::<[u64; 4], __m256i>(A[offset..offset + 4].try_into().unwrap());
        let b = transmute::<[u64; 4], __m256i>(B[offset..offset + 4].try_into().unwrap());
        let c = transmute::<[u64; 4], __m256i>(C[offset..offset + 4].try_into().unwrap());
        let lo = madd52lo_256(a, b, c);
        let hi = madd52hi_256(a, b, c);
        assert_eq!(transmute::<_, [u64; 4]>(lo), LO[offset..offset + 4]);
        assert_eq!(transmute::<_, [u64; 4]>(hi), HI[offset..offset + 4]);
    }
}

#[cfg(any(avx, avx512vl))]
#[cfg_attr(avx, target_feature(enable = "avxifma"))]
#[cfg_attr(avx512vl, target_feature(enable = "avx512ifma,avx512vl"))]
unsafe fn test_128() {
    for offset in (0..A.len()).step_by(2) {
        let a = transmute::<[u64; 2], __m128i>(A[offset..offset + 2].try_into().unwrap());
        let b = transmute::<[u64; 2], __m128i>(B[offset..offset + 2].try_into().unwrap());
        let c = transmute::<[u64; 2], __m128i>(C[offset..offset + 2].try_into().unwrap());
        let lo = madd52lo_128(a, b, c);
        let hi = madd52hi_128(a, b, c);
        assert_eq!(transmute::<_, [u64; 2]>(lo), LO[offset..offset + 2]);
        assert_eq!(transmute::<_, [u64; 2]>(hi), HI[offset..offset + 2]);
    }
}
