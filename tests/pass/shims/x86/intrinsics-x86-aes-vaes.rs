// We're testing x86 target specific features
//@only-target: x86_64 i686
//@compile-flags: -C target-feature=+aes,+vaes,+avx512f
//@run-native

#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

fn main() {
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
            }
        }
    }
}

macro_rules! hex {
    ($($s:literal)*) => {{
        const STRINGS: &[&'static [u8]] = &[$($s.as_bytes(),)*];
        const {
            hex_literal::decode::<{ hex_literal::len(STRINGS) }>(STRINGS)
                .expect("Output array length should be correct")
        }
    }};
}

const START_K: [u8; 64] = hex!(
    "000102030405060708090A0B0C0D0E0F"
    "101112131415161718191A1B1C1D1E1F"
    "202122232425262728292A2B2C2D2E2F"
    "303132333435363738393A3B3C3D3E3F"
);
const EXPECTED_K: [u8; 64] = hex!(
    "3E20891643D63F40AB9858F40DF473EB"
    "34FAD322139140B611EC284682B04712"
    "7234EC1DF69957C127300335BB1E1F3F"
    "118378FC9B461ED7CC4BCE7342A23269"
);
const EXPECTED_B: [u8; 64] = hex!(
    "BD12A330F63C66D9220E0A15AE34C1A3"
    "55AE92B160B72676D796AB343539B3F6"
    "BCE5B43E431A685CD7BDC0393D6C7FDD"
    "41F0604A1F8F16027483C220A8BDDD1F"
);
const N: u64 = 1000;

// Test `_mm_aeskeygenassist_si128` and `_mm_aesimc_si128`
#[target_feature(enable = "aes")]
fn test_aes_keygen() {
    let k = hex!("000102030405060708090A0B0C0D0E0F");
    let expected_k = hex!("B5A5445BB9DF01C715DF84737CC240F3");

    let mut k = k.as_mm();
    for _ in 0..N {
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
    let (ks, tail) = START_K.as_chunks::<16>();
    assert!(tail.is_empty());
    let (expected_ks, tail) = EXPECTED_K.as_chunks::<16>();
    assert!(tail.is_empty());
    let (expected_bs, tail) = EXPECTED_B.as_chunks::<16>();
    assert!(tail.is_empty());

    for i in 0..ks.len() {
        let mut k = ks[i].as_mm();
        let mut b = k;
        for _ in 0..N {
            b = _mm_aesenc_si128(b, k);
            k = _mm_aesenclast_si128(k, b);
            b = _mm_aesdec_si128(b, k);
            k = _mm_aesdeclast_si128(k, b);
        }
        assert!(expected_ks[i].is_eq(k));
        assert!(expected_bs[i].is_eq(b));
    }
}

/// Test `_mm256aesenc_epi128`, `_mm256_aesenclast_epi128`,
/// `_mm256_aesdec_epi128`, and `_mm256_aesdeclast_epi128`
#[target_feature(enable = "vaes")]
fn test_vaes256() {
    let (ks, tail) = START_K.as_chunks::<32>();
    assert!(tail.is_empty());
    let (expected_ks, tail) = EXPECTED_K.as_chunks::<32>();
    assert!(tail.is_empty());
    let (expected_bs, tail) = EXPECTED_B.as_chunks::<32>();
    assert!(tail.is_empty());

    for i in 0..ks.len() {
        let mut k = ks[i].as_mm();
        let mut b = k;
        for _ in 0..N {
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
    for _ in 0..N {
        b = _mm512_aesenc_epi128(b, k);
        k = _mm512_aesenclast_epi128(k, b);
        b = _mm512_aesdec_epi128(b, k);
        k = _mm512_aesdeclast_epi128(k, b);
    }
    assert!(EXPECTED_K.is_eq(k));
    assert!(EXPECTED_B.is_eq(b));
}

/// Trait for casting between `[u8; 16/32/64]` and `__m128/256/512i` types
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

unsafe impl AsMm for [u8; 16] {
    type Mm = __m128i;
}

unsafe impl AsMm for [u8; 32] {
    type Mm = __m256i;
}

unsafe impl AsMm for [u8; 64] {
    type Mm = __m512i;
}

// Vendored from the `hex-literal` crate
mod hex_literal {
    const fn next_hex_char(string: &[u8], mut pos: usize) -> Option<(u8, usize)> {
        while pos < string.len() {
            let raw_val = string[pos];
            pos += 1;
            let val = match raw_val {
                b'0'..=b'9' => raw_val - 48,
                b'A'..=b'F' => raw_val - 55,
                b'a'..=b'f' => raw_val - 87,
                b' ' | b':' | b'\r' | b'\n' | b'\t' => continue,
                0..=127 => panic!("Encountered invalid ASCII character"),
                _ => panic!("Encountered non-ASCII character"),
            };
            return Some((val, pos));
        }
        None
    }

    const fn next_byte(string: &[u8], pos: usize) -> Option<(u8, usize)> {
        let (half1, pos) = match next_hex_char(string, pos) {
            Some(v) => v,
            None => return None,
        };
        let (half2, pos) = match next_hex_char(string, pos) {
            Some(v) => v,
            None => panic!("Odd number of hex characters"),
        };
        Some(((half1 << 4) + half2, pos))
    }

    /// Compute length of a byte array which will be decoded from the strings.
    ///
    /// This function is an implementation detail and SHOULD NOT be called directly!
    #[doc(hidden)]
    #[must_use]
    pub const fn len(strings: &[&[u8]]) -> usize {
        let mut i = 0;
        let mut len = 0;
        while i < strings.len() {
            let mut pos = 0;
            while let Some((_, new_pos)) = next_byte(strings[i], pos) {
                len += 1;
                pos = new_pos;
            }
            i += 1;
        }
        len
    }

    /// Decode hex strings into a byte array of pre-computed length.
    ///
    /// This function is an implementation detail and SHOULD NOT be called directly!
    #[doc(hidden)]
    #[must_use]
    pub const fn decode<const LEN: usize>(strings: &[&[u8]]) -> Option<[u8; LEN]> {
        let mut string_pos = 0;
        let mut buf = [0u8; LEN];
        let mut buf_pos = 0;
        while string_pos < strings.len() {
            let mut pos = 0;
            let string = &strings[string_pos];
            string_pos += 1;

            while let Some((byte, new_pos)) = next_byte(string, pos) {
                buf[buf_pos] = byte;
                buf_pos += 1;
                pos = new_pos;
            }
        }
        if LEN == buf_pos { Some(buf) } else { None }
    }
}
