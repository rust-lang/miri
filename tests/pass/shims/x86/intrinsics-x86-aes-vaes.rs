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
            test_aesimc();
            test_aeskeygenassist();

            test_aesenc();
            test_aesenclast();
            test_aesdec();
            test_aesdeclast();

            loop_test_aes_keygen();
            loop_test_aes();
        }

        if is_x86_feature_detected!("vaes") {
            unsafe {
                test_aesenc256();
                test_aesenclast256();
                test_aesdec256();
                test_aesdeclast256();

                loop_test_vaes256();
            }

            if is_x86_feature_detected!("avx512f") {
                unsafe {
                    test_aesenc512();
                    test_aesenclast512();
                    test_aesdec512();
                    test_aesdeclast512();

                    loop_test_vaes512();
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

/// Number of 128 bit values in test vectors.
const N: usize = 4;

// Initial value of `k` used in tests
const K: [u128; N] = [
    0x000102030405060708090A0B0C0D0E0F,
    0x101112131415161718191A1B1C1D1E1F,
    0x202122232425262728292A2B2C2D2E2F,
    0x303132333435363738393A3B3C3D3E3F,
];
// Initial value of `b` used in tests
const B: [u128; N] = [
    0x404142434445464748494A4B4C4D4E4F,
    0x505152535455565758595A5B5C5D5E5F,
    0x606162636465666768696A6B6C6D6E6F,
    0x707172737475767778797A7B7C7D7E7F,
];

// Expected values after applying `aesimc` to `K`
const EXPECTED_AESIMC: [u128; N] = [
    0x0E0B0C090A0F080D0603040102070005,
    0x1E1B1C191A1F181D1613141112171015,
    0x2E2B2C292A2F282D2623242122272025,
    0x3E3B3C393A3F383D3633343132373035,
];
// Expected values after applying `aeskeygenassist` to `K` with `rcon` equal to 0x36
const EXPECTED_AESKEYGENASSIST: [u128; N] = [
    0x7B637C41637C777B2B3001513001672B,
    0x7DCA82FFCA82C97DAFADD494ADD4A2AF,
    0x26B7FDA5B7FD9326F134A5D334A5E5F1,
    0xC304C71504C723C3E20712B6071280E2,
];
// Expected values after applying `aesenc` to `B` and `K`
const EXPECTED_AESENC: [u128; N] = [
    0x0C6F106694A292994D86BA323198861A,
    0xEF4929D16168F387A7F6783AB27AFAEC,
    0xD669AFCEACBDEDAAD550493F3B7685FF,
    0xC097131CECBAE545E04D8582B4E7AE39,
];
// Expected values after applying `aesenclast` to `B` and `K`
const EXPECTED_AESENCLAST: [u128; N] = [
    0x1B3A2D1956E62AA7218A50B80563D88B,
    0x30DA4AFE7E59164C52C8AB224FE1A0D0,
    0x63D8BDD861198CA278C61954FC602C87,
    0xA287C1BC88CA76C2289A021A6DA0E4ED,
];
// Expected values after applying `aesdec` to `B` and `K`
const EXPECTED_AESDEC: [u128; N] = [
    0x3B4CF684851A13D1FEF9C4C79E8115D2,
    0x8FC125301FBECD1178BD94161F98F50D,
    0xDF5BB3B842D66A8F10B4F343DE4E5721,
    0x4B1C1B62454ED2A55A44C6B75C118C4A,
];
// Expected values after applying `aesdeclast` to `B` and `K`
const EXPECTED_AESDECLAST: [u128; N] = [
    0x5DA59A6776605A118EF1BCC7D865F89D,
    0xB704AB43789850CDE569874C42F0569B,
    0x98C5F123B4967E2DA4F16F2EDB918529,
    0x319E3DBCE4268B35F215B038FD022054,
];
// Expected value of `k` in loop tests
const EXPECTED_LOOP_K: [u128; N] = [
    0xD21928B8DEB8D0565C1FB92B010E2363,
    0x8D2E2F5521147125F8E24489EA0D8D97,
    0xEE26FC76F1CC0D3D849AAF838A16117B,
    0xA4C8122B4F6C00362E3692B1B7854BF7,
];
// Expected value of `b` in loop tests
const EXPECTED_LOOP_B: [u128; N] = [
    0xE74F19EFD8C28BFE3BD285175F3F68FF,
    0xCE70940A752A6E0A23AD14E31C033BBF,
    0xE224ED621B046A2D337BB2EC5020424E,
    0xE617A20C71EC216CF6DE4FF8EDB763B0,
];
/// Number of iternations used in loop tests
const ITERATIONS: u64 = 128;

#[target_feature(enable = "aes")]
fn test_aesimc() {
    for i in 0..N {
        let r = _mm_aesimc_si128(K[i].as_mm());
        assert!(EXPECTED_AESIMC[i].is_eq(r));
    }
}

#[target_feature(enable = "aes")]
fn test_aeskeygenassist() {
    for i in 0..N {
        let r = _mm_aeskeygenassist_si128(K[i].as_mm(), 0x36);
        assert!(EXPECTED_AESKEYGENASSIST[i].is_eq(r));
    }
}

#[target_feature(enable = "aes")]
fn test_aesenc() {
    for i in 0..N {
        let r = _mm_aesenc_si128(B[i].as_mm(), K[i].as_mm());
        assert!(EXPECTED_AESENC[i].is_eq(r));
    }
}

#[target_feature(enable = "aes")]
fn test_aesenclast() {
    for i in 0..N {
        let r = _mm_aesenclast_si128(B[i].as_mm(), K[i].as_mm());
        assert!(EXPECTED_AESENCLAST[i].is_eq(r));
    }
}

#[target_feature(enable = "aes")]
fn test_aesdec() {
    for i in 0..N {
        let r = _mm_aesdec_si128(B[i].as_mm(), K[i].as_mm());
        assert!(EXPECTED_AESDEC[i].is_eq(r));
    }
}

#[target_feature(enable = "aes")]
fn test_aesdeclast() {
    for i in 0..N {
        let r = _mm_aesdeclast_si128(B[i].as_mm(), K[i].as_mm());
        assert!(EXPECTED_AESDECLAST[i].is_eq(r));
    }
}

#[target_feature(enable = "aes")]
fn loop_test_aes_keygen() {
    let k = K[0];
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

#[target_feature(enable = "aes")]
fn loop_test_aes() {
    for i in 0..N {
        let mut k = K[i].as_mm();
        let mut b = k;
        for _ in 0..ITERATIONS {
            b = _mm_aesenc_si128(b, k);
            k = _mm_aesenclast_si128(k, b);
            b = _mm_aesdec_si128(b, k);
            k = _mm_aesdeclast_si128(k, b);
        }
        assert!(EXPECTED_LOOP_K[i].is_eq(k));
        assert!(EXPECTED_LOOP_B[i].is_eq(b));
    }
}

#[target_feature(enable = "vaes")]
fn test_aesenc256() {
    let (b, _) = B.as_chunks::<2>();
    let (k, _) = K.as_chunks::<2>();
    let (e, _) = EXPECTED_AESENC.as_chunks::<2>();
    for i in 0..N / 2 {
        let r = _mm256_aesenc_epi128(b[i].as_mm(), k[i].as_mm());
        assert!(e[i].is_eq(r));
    }
}

#[target_feature(enable = "vaes")]
fn test_aesenclast256() {
    let (b, _) = B.as_chunks::<2>();
    let (k, _) = K.as_chunks::<2>();
    let (e, _) = EXPECTED_AESENCLAST.as_chunks::<2>();
    for i in 0..N / 2 {
        let r = _mm256_aesenclast_epi128(b[i].as_mm(), k[i].as_mm());
        assert!(e[i].is_eq(r));
    }
}

#[target_feature(enable = "vaes")]
fn test_aesdec256() {
    let (b, _) = B.as_chunks::<2>();
    let (k, _) = K.as_chunks::<2>();
    let (e, _) = EXPECTED_AESDEC.as_chunks::<2>();
    for i in 0..N / 2 {
        let r = _mm256_aesdec_epi128(b[i].as_mm(), k[i].as_mm());
        assert!(e[i].is_eq(r));
    }
}

#[target_feature(enable = "vaes")]
fn test_aesdeclast256() {
    let (b, _) = B.as_chunks::<2>();
    let (k, _) = K.as_chunks::<2>();
    let (e, _) = EXPECTED_AESDECLAST.as_chunks::<2>();
    for i in 0..N / 2 {
        let r = _mm256_aesdeclast_epi128(b[i].as_mm(), k[i].as_mm());
        assert!(e[i].is_eq(r));
    }
}

#[target_feature(enable = "vaes")]
fn loop_test_vaes256() {
    let (ks, tail) = K.as_chunks::<2>();
    assert!(tail.is_empty());
    let (expected_ks, tail) = EXPECTED_LOOP_K.as_chunks::<2>();
    assert!(tail.is_empty());
    let (expected_bs, tail) = EXPECTED_LOOP_B.as_chunks::<2>();
    assert!(tail.is_empty());

    for i in 0..N / 2 {
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

#[target_feature(enable = "avx512f,vaes")]
fn test_aesenc512() {
    let r = _mm512_aesenc_epi128(B.as_mm(), K.as_mm());
    assert!(EXPECTED_AESENC.is_eq(r));
}

#[target_feature(enable = "avx512f,vaes")]
fn test_aesenclast512() {
    let r = _mm512_aesenclast_epi128(B.as_mm(), K.as_mm());
    assert!(EXPECTED_AESENCLAST.is_eq(r));
}

#[target_feature(enable = "avx512f,vaes")]
fn test_aesdec512() {
    let r = _mm512_aesdec_epi128(B.as_mm(), K.as_mm());
    assert!(EXPECTED_AESDEC.is_eq(r));
}

#[target_feature(enable = "avx512f,vaes")]
fn test_aesdeclast512() {
    let r = _mm512_aesdeclast_epi128(B.as_mm(), K.as_mm());
    assert!(EXPECTED_AESDECLAST.is_eq(r));
}

#[target_feature(enable = "avx512f,vaes")]
fn loop_test_vaes512() {
    let mut k = K.as_mm();
    let mut b = k;
    for _ in 0..ITERATIONS {
        b = _mm512_aesenc_epi128(b, k);
        k = _mm512_aesenclast_epi128(k, b);
        b = _mm512_aesdec_epi128(b, k);
        k = _mm512_aesdeclast_epi128(k, b);
    }
    assert!(EXPECTED_LOOP_K.is_eq(k));
    assert!(EXPECTED_LOOP_B.is_eq(b));
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
