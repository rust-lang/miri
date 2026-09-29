//@only-target: x86_64 i686
//@revisions: missing_128 missing_256 missing_512 vl_128 vl_256 avx_512
//@[missing_128,missing_256,missing_512]compile-flags: -C target-feature=-avx512ifma,-avxifma
//@[vl_128,vl_256]compile-flags: -C target-feature=+avx512ifma,-avx512vl,-avxifma
//@[avx_512]compile-flags: -C target-feature=+avxifma,-avx512f

#![feature(abi_unadjusted, link_llvm_intrinsics, simd_ffi)]

#[cfg(target_arch = "x86")]
use std::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;
use std::mem::transmute;

fn main() {
    unsafe {
        #[cfg(any(missing_128, vl_128))]
        {
            let zero = transmute::<[u64; 2], __m128i>([0; 2]);
            madd52_128(zero, zero, zero);
            //~[missing_128]^ ERROR: requires missing target feature avx512ifma
            //~[vl_128]| ERROR: requires missing target feature avx512vl
        }
        #[cfg(any(missing_256, vl_256))]
        {
            let zero = transmute::<[u64; 4], __m256i>([0; 4]);
            madd52_256(zero, zero, zero);
            //~[missing_256]^ ERROR: requires missing target feature avx512ifma
            //~[vl_256]| ERROR: requires missing target feature avx512vl
        }
        #[cfg(any(missing_512, avx_512))]
        {
            let zero = transmute::<[u64; 8], __m512i>([0; 8]);
            madd52_512(zero, zero, zero);
            //~[missing_512,avx_512]^ ERROR: requires missing target feature avx512ifma
        }
    }
}

#[allow(improper_ctypes)]
unsafe extern "unadjusted" {
    #[cfg(any(missing_128, vl_128))]
    #[link_name = "llvm.x86.avx512.vpmadd52l.uq.128"]
    fn madd52_128(a: __m128i, b: __m128i, c: __m128i) -> __m128i;

    #[cfg(any(missing_256, vl_256))]
    #[link_name = "llvm.x86.avx512.vpmadd52l.uq.256"]
    fn madd52_256(a: __m256i, b: __m256i, c: __m256i) -> __m256i;

    #[cfg(any(missing_512, avx_512))]
    #[link_name = "llvm.x86.avx512.vpmadd52l.uq.512"]
    fn madd52_512(a: __m512i, b: __m512i, c: __m512i) -> __m512i;
}
