// We're testing x86 target specific features
//@only-target: x86_64 i686
//@compile-flags: -C target-feature=+aes,+vaes,+avx512f
//@run-native

#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

fn main() {
    // Bail out dynamically if target feature is not supported
    if is_x86_feature_detected!("aes") {
        unsafe {
            test_aes_keygen();
            test_aes();
        }

        if is_x86_feature_detected!("vaes") {
            unsafe {
                test_vaes256();
            }

            if is_x86_feature_detected!("avx512f") {
                unsafe {
                    test_vaes512();
                }
            } else {
                println!("warning: skipping VAES+AVX-512 tests");
            }
        } else {
            println!("warning: skipping VAES tests");
        }
    } else {
        println!("warning: skipping AES tests");
    }
}

const START_K: [u128; 4] = [
    0x000102030405060708090A0B0C0D0E0F,
    0x101112131415161718191A1B1C1D1E1F,
    0x202122232425262728292A2B2C2D2E2F,
    0x303132333435363738393A3B3C3D3E3F,
];
const EXPECTED_K: [u128; 4] = [
    0xD21928B8DEB8D0565C1FB92B010E2363,
    0x8D2E2F5521147125F8E24489EA0D8D97,
    0xEE26FC76F1CC0D3D849AAF838A16117B,
    0xA4C8122B4F6C00362E3692B1B7854BF7,
];
const EXPECTED_B: [u128; 4] = [
    0xE74F19EFD8C28BFE3BD285175F3F68FF,
    0xCE70940A752A6E0A23AD14E31C033BBF,
    0xE224ED621B046A2D337BB2EC5020424E,
    0xE617A20C71EC216CF6DE4FF8EDB763B0,
];
const ITERATIONS: u64 = 128;

// Test `_mm_aeskeygenassist_si128` and `_mm_aesimc_si128`
#[target_feature(enable = "aes")]
fn test_aes_keygen() {
    let k = 0x000102030405060708090A0B0C0D0E0F;
    let expected_k = 0x8DD2144D60F310A85C3487481DDC6789;

    let mut k = k.as_mm();
    for _ in 0..ITERATIONS {
        k = _mm_aeskeygenassist_si128(k, 0x36);
        let t = _mm_aesimc_si128(k);
        // use `aesenc` to "randomize" `k`
        k = _mm_aesenc_si128(k, t);
    }

    assert!(expected_k.is_eq(k));
}

/// Test `_mm_aesenc_si128`, `_mm_aesenclast_si128`,
/// `_mm_aesdec_si128`, and `_mm_aesdeclast_si128`
#[target_feature(enable = "aes")]
fn test_aes() {
    for i in 0..4 {
        let mut k = START_K[i].as_mm();
        let mut b = k;
        for _ in 0..ITERATIONS {
            b = _mm_aesenc_si128(b, k);
            k = _mm_aesenclast_si128(k, b);
            b = _mm_aesdec_si128(b, k);
            k = _mm_aesdeclast_si128(k, b);
        }
        assert!(EXPECTED_K[i].is_eq(k));
        assert!(EXPECTED_B[i].is_eq(b));
    }
}

/// Test `_mm256aesenc_epi128`, `_mm256_aesenclast_epi128`,
/// `_mm256_aesdec_epi128`, and `_mm256_aesdeclast_epi128`
#[target_feature(enable = "vaes")]
fn test_vaes256() {
    let (ks, tail) = START_K.as_chunks::<2>();
    assert!(tail.is_empty());
    let (expected_ks, tail) = EXPECTED_K.as_chunks::<2>();
    assert!(tail.is_empty());
    let (expected_bs, tail) = EXPECTED_B.as_chunks::<2>();
    assert!(tail.is_empty());

    for i in 0..2 {
        let mut k = ks[i].as_mm();
        let mut b = k;
        for _ in 0..ITERATIONS {
            b = _mm256_aesenc_epi128(b, k);
            k = _mm256_aesenclast_epi128(k, b);
            b = _mm256_aesdec_epi128(b, k);
            k = _mm256_aesdeclast_epi128(k, b);
        }
        assert!(expected_ks[i].is_eq(k));
        assert!(expected_bs[i].is_eq(b));
    }
}

/// Test `_mm512aesenc_epi128`, `_mm512_aesenclast_epi128`,
/// `_mm512_aesdec_epi128`, and `_mm512_aesdeclast_epi128`
#[target_feature(enable = "avx512f,vaes")]
fn test_vaes512() {
    let mut k = START_K.as_mm();
    let mut b = k;
    for _ in 0..ITERATIONS {
        b = _mm512_aesenc_epi128(b, k);
        k = _mm512_aesenclast_epi128(k, b);
        b = _mm512_aesdec_epi128(b, k);
        k = _mm512_aesdeclast_epi128(k, b);
    }
    assert!(EXPECTED_K.is_eq(k));
    assert!(EXPECTED_B.is_eq(b));
}

/// Trait for casting between `u128/[u128; 2]/[u128; 4]` and `__m128/256/512i` types
///
/// # Safety
/// The trait must not be implemented for any other type pairs.
unsafe trait AsMm: Sized + core::cmp::Eq {
    type Mm;

    fn as_mm(&self) -> Self::Mm {
        unsafe { core::mem::transmute_copy(self) }
    }

    fn is_eq(&self, v: Self::Mm) -> bool {
        let r: Self = unsafe { core::mem::transmute_copy(&v) };
        self.eq(&r)
    }
}

unsafe impl AsMm for u128 {
    type Mm = __m128i;
}

unsafe impl AsMm for [u128; 2] {
    type Mm = __m256i;
}

unsafe impl AsMm for [u128; 4] {
    type Mm = __m512i;
}
