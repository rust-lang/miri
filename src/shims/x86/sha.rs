use rustc_span::Symbol;
use rustc_target::spec::abi::Abi;

use crate::*;

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub(super) trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn emulate_x86_sha_intrinsic(
        &mut self,
        link_name: Symbol,
        abi: Abi,
        args: &[OpTy<'tcx>],
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx, EmulateItemResult> {
        let this = self.eval_context_mut();
        this.expect_target_feature_for_intrinsic(link_name, "sha")?;
        // Prefix should have already been checked.
        let unprefixed_name = link_name.as_str().strip_prefix("llvm.x86.sha").unwrap();

        match unprefixed_name {
            // Used to implement the _mm_sha256rnds2_epu32 function.
            "256rnds2" => {
                let [a, b, k] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                let (a_reg, a_len) = this.operand_to_simd(a)?;
                let (b_reg, b_len) = this.operand_to_simd(b)?;
                let (k_reg, k_len) = this.operand_to_simd(k)?;
                let (dest, dest_len) = this.mplace_to_simd(dest)?;

                assert_eq!(a_len, 4);
                assert_eq!(b_len, 4);
                assert_eq!(k_len, 4);
                assert_eq!(dest_len, 4);

                let mut a = [0; 4];
                for (i, dst) in a.iter_mut().enumerate() {
                    *dst = this.read_scalar(&this.project_index(&a_reg, i as u64)?)?.to_u32()?
                }
                let mut b = [0; 4];
                for (i, dst) in b.iter_mut().enumerate() {
                    *dst = this.read_scalar(&this.project_index(&b_reg, i as u64)?)?.to_u32()?
                }
                let mut k = [0; 4];
                for (i, dst) in k.iter_mut().enumerate() {
                    *dst = this.read_scalar(&this.project_index(&k_reg, i as u64)?)?.to_u32()?
                }

                let result = sha256rnds2_epu32(a, b, k);
                for (i, part) in result.into_iter().enumerate() {
                    this.write_scalar(Scalar::from_u32(part), &this.project_index(&dest, i as u64)?)?;
                }
            }
            // Used to implement the _mm_sha256msg1_epu32 function.
            "256msg1" => {
                let [a, b] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                let (a_reg, a_len) = this.operand_to_simd(a)?;
                let (b_reg, b_len) = this.operand_to_simd(b)?;
                let (dest, dest_len) = this.mplace_to_simd(dest)?;

                assert_eq!(a_len, 4);
                assert_eq!(b_len, 4);
                assert_eq!(dest_len, 4);

                let mut a = [0; 4];
                for (i, dst) in a.iter_mut().enumerate() {
                    *dst = this.read_scalar(&this.project_index(&a_reg, i as u64)?)?.to_u32()?
                }
                // least significant part of b
                let b_ls = this.read_scalar(&this.project_index(&b_reg, 0)?)?.to_u32()?;

                let result = sha256msg1_epu32(a, b_ls);
                for (i, part) in result.into_iter().enumerate() {
                    this.write_scalar(Scalar::from_u32(part), &this.project_index(&dest, i as u64)?)?;
                }
            }
            // Used to implement the _mm_sha256msg2_epu32 function.
            "256msg2" => {
                let [a, b] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                let (a_reg, a_len) = this.operand_to_simd(a)?;
                let (b_reg, b_len) = this.operand_to_simd(b)?;
                let (dest, dest_len) = this.mplace_to_simd(dest)?;

                assert_eq!(a_len, 4);
                assert_eq!(b_len, 4);
                assert_eq!(dest_len, 4);

                let mut a = [0; 4];
                for (i, dst) in a.iter_mut().enumerate() {
                    *dst = this.read_scalar(&this.project_index(&a_reg, i as u64)?)?.to_u32()?
                }
                // most significant part of b
                let mut b_ms = [0; 2];
                for (i, dst) in b_ms.iter_mut().enumerate() {
                    *dst = this.read_scalar(&this.project_index(&b_reg, i as u64 + 2)?)?.to_u32()?
                }

                let result = sha256msg2_epu32(a, b_ms);
                for (i, part) in result.into_iter().enumerate() {
                    this.write_scalar(Scalar::from_u32(part), &this.project_index(&dest, i as u64)?)?;
                }
            }
            _ => return Ok(EmulateItemResult::NotSupported),
        }
        Ok(EmulateItemResult::NeedsReturn)
    }
}

#[allow(non_snake_case)]
fn sha256rnds2_epu32(a: [u32; 4], b: [u32; 4], k: [u32; 4]) -> [u32; 4] {
    // Translated from the Intel's documentation by Chat GPT.
    // It works, don't ask why it looks horrible.
    let mut A: [u32; 3] = [0; 3];
    let mut B: [u32; 3] = [0; 3];
    let mut C: [u32; 3] = [0; 3];
    let mut D: [u32; 3] = [0; 3];
    let mut E: [u32; 3] = [0; 3];
    let mut F: [u32; 3] = [0; 3];
    let mut G: [u32; 3] = [0; 3];
    let mut H: [u32; 3] = [0; 3];
    let mut W_K: [u32; 2] = [0; 2];

    A[0] = b[3];
    B[0] = b[2];
    C[0] = a[3];
    D[0] = a[2];
    E[0] = b[1];
    F[0] = b[0];
    G[0] = a[1];
    H[0] = a[0];

    W_K[0] = k[0];
    W_K[1] = k[1];

    for i in 0..2 {
        A[i + 1] = ch(E[i], F[i], G[i])
            .wrapping_add(sum1(E[i]))
            .wrapping_add(W_K[i])
            .wrapping_add(H[i])
            .wrapping_add(maj(A[i], B[i], C[i]))
            .wrapping_add(sum0(A[i]));

        B[i + 1] = A[i];
        C[i + 1] = B[i];
        D[i + 1] = C[i];

        E[i + 1] = ch(E[i], F[i], G[i])
            .wrapping_add(sum1(E[i]))
            .wrapping_add(W_K[i])
            .wrapping_add(H[i])
            .wrapping_add(D[i]);

        F[i + 1] = E[i];
        G[i + 1] = F[i];
        H[i + 1] = G[i];
    }

    [F[2], E[2], B[2], A[2]]
}

#[allow(non_snake_case)]
fn sha256msg1_epu32(a: [u32; 4], b_ls: u32) -> [u32; 4] {
    let W4 = b_ls;
    let [W0, W1, W2, W3] = a;
    [
        W0.wrapping_add(sigma0(W1)),
        W1.wrapping_add(sigma0(W2)),
        W2.wrapping_add(sigma0(W3)),
        W3.wrapping_add(sigma0(W4)),
    ]
}

#[allow(non_snake_case)]
fn sha256msg2_epu32(a: [u32; 4], b_ms: [u32; 2]) -> [u32; 4] {
    let [W14, W15] = b_ms;
    let W16 = a[0].wrapping_add(sigma1(W14));
    let W17 = a[1].wrapping_add(sigma1(W15));
    let W18 = a[2].wrapping_add(sigma1(W16));
    let W19 = a[3].wrapping_add(sigma1(W17));
    [W16, W17, W18, W19]
}

fn ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

fn maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

fn sum0(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}

fn sum1(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}

fn sigma0(x: u32) -> u32 {
    x.rotate_left(25) ^ x.rotate_left(14) ^ (x >> 3)
}

fn sigma1(x: u32) -> u32 {
    x.rotate_left(15) ^ x.rotate_left(13) ^ (x >> 10)
}
