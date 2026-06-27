//@only-target: aarch64
//@compile-flags: -C target-feature=+sha2

use std::arch::aarch64::*;
use std::fmt::Write;

fn main() {
    assert!(std::arch::is_aarch64_feature_detected!("sha2"));
    unsafe {
        test_sha256();
    }
}

#[target_feature(enable = "sha2")]
unsafe fn test_sha256() {
    const INITIAL_STATE: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // same message as the x86 SHA test.
    let first_block = *b"Rust is awesome!Rust is awesome!Rust is awesome!Rust is awesome!";

    // SHA256 padding: 0x80 byte, zeros, then message length in bits as big endian u64.
    let mut final_block = [0u8; 64];
    final_block[0] = 0x80;
    final_block[56..].copy_from_slice(&(8u64 * 64).to_be_bytes());

    let mut state = INITIAL_STATE;
    sha256_compress(&mut state, &[first_block, final_block]);

    let mut hash = String::new();
    for word in &state {
        write!(hash, "{:08x}", word).expect("writing to String doesn't fail");
    }
    assert_eq!(hash, "1b2293d21b17a0cb0c18737307c37333dea775eded18cefed45e50389f9f8184");
}

// Adapted from RustCrypto's sha256 aarch64 implementation:
// https://github.com/RustCrypto/hashes/blob/master/sha2/src/sha256/aarch64_sha2.rs
#[target_feature(enable = "sha2")]
unsafe fn sha256_compress(state: &mut [u32; 8], blocks: &[[u8; 64]]) {
    #[rustfmt::skip]
    const K32: [u32; 64] = [
        0x428A2F98, 0x71374491, 0xB5C0FBCF, 0xE9B5DBA5,
        0x3956C25B, 0x59F111F1, 0x923F82A4, 0xAB1C5ED5,
        0xD807AA98, 0x12835B01, 0x243185BE, 0x550C7DC3,
        0x72BE5D74, 0x80DEB1FE, 0x9BDC06A7, 0xC19BF174,
        0xE49B69C1, 0xEFBE4786, 0x0FC19DC6, 0x240CA1CC,
        0x2DE92C6F, 0x4A7484AA, 0x5CB0A9DC, 0x76F988DA,
        0x983E5152, 0xA831C66D, 0xB00327C8, 0xBF597FC7,
        0xC6E00BF3, 0xD5A79147, 0x06CA6351, 0x14292967,
        0x27B70A85, 0x2E1B2138, 0x4D2C6DFC, 0x53380D13,
        0x650A7354, 0x766A0ABB, 0x81C2C92E, 0x92722C85,
        0xA2BFE8A1, 0xA81A664B, 0xC24B8B70, 0xC76C51A3,
        0xD192E819, 0xD6990624, 0xF40E3585, 0x106AA070,
        0x19A4C116, 0x1E376C08, 0x2748774C, 0x34B0BCB5,
        0x391C0CB3, 0x4ED8AA4A, 0x5B9CCA4F, 0x682E6FF3,
        0x748F82EE, 0x78A5636F, 0x84C87814, 0x8CC70208,
        0x90BEFFFA, 0xA4506CEB, 0xBEF9A3F7, 0xC67178F2,
    ];

    // Load state into vectors.
    let mut abcd = vld1q_u32(state[0..4].as_ptr());
    let mut efgh = vld1q_u32(state[4..8].as_ptr());

    for block in blocks {
        // Keep original state values.
        let abcd_orig = abcd;
        let efgh_orig = efgh;

        // Load the message block into vectors, assuming little endianness.
        let mut s0 = vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block[0..16].as_ptr())));
        let mut s1 = vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block[16..32].as_ptr())));
        let mut s2 = vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block[32..48].as_ptr())));
        let mut s3 = vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block[48..64].as_ptr())));

        let mut tmp;
        let mut abcd_prev;

        // Rounds 0 to 3
        tmp = vaddq_u32(s0, vld1q_u32(K32[0..4].as_ptr()));
        abcd_prev = abcd;
        abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
        efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

        // Rounds 4 to 7
        tmp = vaddq_u32(s1, vld1q_u32(K32[4..8].as_ptr()));
        abcd_prev = abcd;
        abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
        efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

        // Rounds 8 to 11
        tmp = vaddq_u32(s2, vld1q_u32(K32[8..12].as_ptr()));
        abcd_prev = abcd;
        abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
        efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

        // Rounds 12 to 15
        tmp = vaddq_u32(s3, vld1q_u32(K32[12..16].as_ptr()));
        abcd_prev = abcd;
        abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
        efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

        for t in (16..64).step_by(16) {
            // Rounds t to t + 3
            s0 = vsha256su1q_u32(vsha256su0q_u32(s0, s1), s2, s3);
            tmp = vaddq_u32(s0, vld1q_u32(K32[t..t + 4].as_ptr()));
            abcd_prev = abcd;
            abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
            efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

            // Rounds t + 4 to t + 7
            s1 = vsha256su1q_u32(vsha256su0q_u32(s1, s2), s3, s0);
            tmp = vaddq_u32(s1, vld1q_u32(K32[t + 4..t + 8].as_ptr()));
            abcd_prev = abcd;
            abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
            efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

            // Rounds t + 8 to t + 11
            s2 = vsha256su1q_u32(vsha256su0q_u32(s2, s3), s0, s1);
            tmp = vaddq_u32(s2, vld1q_u32(K32[t + 8..t + 12].as_ptr()));
            abcd_prev = abcd;
            abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
            efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

            // Rounds t + 12 to t + 15
            s3 = vsha256su1q_u32(vsha256su0q_u32(s3, s0), s1, s2);
            tmp = vaddq_u32(s3, vld1q_u32(K32[t + 12..t + 16].as_ptr()));
            abcd_prev = abcd;
            abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
            efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);
        }

        // Add the block-specific state to the original state.
        abcd = vaddq_u32(abcd, abcd_orig);
        efgh = vaddq_u32(efgh, efgh_orig);
    }

    // Store vectors into state.
    vst1q_u32(state[0..4].as_mut_ptr(), abcd);
    vst1q_u32(state[4..8].as_mut_ptr(), efgh);
}
