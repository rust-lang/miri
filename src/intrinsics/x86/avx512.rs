use rustc_span::Symbol;

use super::{
    packssdw, packsswb, packusdw, packuswb, permute, permute2, pmaddbw, pmaddwd, psadbw, pshufb,
};
use crate::*;

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub(super) trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn emulate_x86_avx512_intrinsic(
        &mut self,
        link_name: Symbol,
        args: &[OpTy<'tcx>],
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx, EmulateItemResult> {
        let this = self.eval_context_mut();
        // Prefix should have already been checked.
        let unprefixed_name = link_name.as_str().strip_prefix("llvm.x86.avx512.").unwrap();

        match unprefixed_name {
            // Used by the ternarylogic functions.
            "pternlog.d.128" | "pternlog.d.256" | "pternlog.d.512" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512f")?;
                if matches!(unprefixed_name, "pternlog.d.128" | "pternlog.d.256") {
                    this.expect_target_feature_for_intrinsic(link_name, "avx512vl")?;
                }

                let [a, b, c, imm8] = this.check_shim_sig_unadjusted(link_name, args)?;

                assert_eq!(dest.layout, a.layout);
                assert_eq!(dest.layout, b.layout);
                assert_eq!(dest.layout, c.layout);

                // The signatures of these operations are:
                //
                // ```
                // fn vpternlogd(a: i32x16, b: i32x16, c: i32x16, imm8: i32) -> i32x16;
                // fn vpternlogd256(a: i32x8, b: i32x8, c: i32x8, imm8: i32) -> i32x8;
                // fn vpternlogd128(a: i32x4, b: i32x4, c: i32x4, imm8: i32) -> i32x4;
                // ```
                //
                // The element type is always a 32-bit integer, the width varies.

                let (a, _a_len) = this.project_to_simd(a)?;
                let (b, _b_len) = this.project_to_simd(b)?;
                let (c, _c_len) = this.project_to_simd(c)?;
                let (dest, dest_len) = this.project_to_simd(dest)?;

                // Compute one lane with ternary table.
                let tern = |xa: u32, xb: u32, xc: u32, imm: u32| -> u32 {
                    let mut out = 0u32;
                    // At each bit position, select bit from imm8 at index = (a << 2) | (b << 1) | c
                    for bit in 0..32 {
                        let ia = (xa >> bit) & 1;
                        let ib = (xb >> bit) & 1;
                        let ic = (xc >> bit) & 1;
                        let idx = (ia << 2) | (ib << 1) | ic;
                        let v = (imm >> idx) & 1;
                        out |= v << bit;
                    }
                    out
                };

                let imm8 = this.read_scalar(imm8)?.to_u32()? & 0xFF;
                for i in 0..dest_len {
                    let a_lane = this.project_index(&a, i)?;
                    let b_lane = this.project_index(&b, i)?;
                    let c_lane = this.project_index(&c, i)?;
                    let d_lane = this.project_index(&dest, i)?;

                    let va = this.read_scalar(&a_lane)?.to_u32()?;
                    let vb = this.read_scalar(&b_lane)?.to_u32()?;
                    let vc = this.read_scalar(&c_lane)?.to_u32()?;

                    let r = tern(va, vb, vc, imm8);
                    this.write_scalar(Scalar::from_u32(r), &d_lane)?;
                }
            }
            // Used to implement the _mm512_sad_epu8 function.
            "psad.bw.512" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512bw")?;

                let [left, right] = this.check_shim_sig_unadjusted(link_name, args)?;

                psadbw(this, left, right, dest)?
            }
            // Used to implement the _mm512_madd_epi16 function.
            "pmaddw.d.512" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512bw")?;

                let [left, right] = this.check_shim_sig_unadjusted(link_name, args)?;

                pmaddwd(this, left, right, dest)?;
            }
            // Used to implement the _mm512_maddubs_epi16 function.
            "pmaddubs.w.512" => {
                let [left, right] = this.check_shim_sig_unadjusted(link_name, args)?;

                pmaddbw(this, left, right, dest)?;
            }
            // Used to implement the _mm512_permutexvar_epi32/_mm512_permutexvar_epi64 functions.
            "permvar.si.512" | "permvar.di.512" => {
                let [left, right] = this.check_shim_sig_unadjusted(link_name, args)?;

                permute(this, left, right, dest)?;
            }
            "permvar.qi.512" | "permvar.qi.256" | "permvar.qi.128" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512vbmi")?;
                if !unprefixed_name.ends_with("512") {
                    this.expect_target_feature_for_intrinsic(link_name, "avx512vl")?;
                }

                let [left, right] = this.check_shim_sig_unadjusted(link_name, args)?;

                permute(this, left, right, dest)?;
            }
            // Used to implement the _mm512_permutex2var_epi64 intrinsic.
            "vpermi2var.q.512" => {
                let [left, indices, right] = this.check_shim_sig_unadjusted(link_name, args)?;

                permute2(this, left, indices, right, dest)?;
            }
            // Used to implement the _mm512_permutex2var_epi8 intrinsic.
            "vpermi2var.qi.512" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512vbmi")?;

                let [left, indices, right] = this.check_shim_sig_unadjusted(link_name, args)?;

                permute2(this, left, indices, right, dest)?;
            }
            // Used to implement the _mm512_shuffle_epi8 intrinsic.
            "pshuf.b.512" => {
                let [left, right] = this.check_shim_sig_unadjusted(link_name, args)?;

                pshufb(this, left, right, dest)?;
            }

            // Used to implement the _mm512_dpbusd_epi32 function.
            "vpdpbusd.512" | "vpdpbusd.256" | "vpdpbusd.128" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512vnni")?;
                if matches!(unprefixed_name, "vpdpbusd.128" | "vpdpbusd.256") {
                    this.expect_target_feature_for_intrinsic(link_name, "avx512vl")?;
                }

                let [src, a, b] = this.check_shim_sig_unadjusted(link_name, args)?;

                vpdpbusd(this, src, a, b, dest)?;
            }
            // Used to implement the _mm512_packs_epi16 function
            "packsswb.512" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512bw")?;

                let [a, b] = this.check_shim_sig_unadjusted(link_name, args)?;

                packsswb(this, a, b, dest)?;
            }
            // Used to implement the _mm512_packus_epi16 function
            "packuswb.512" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512bw")?;

                let [a, b] = this.check_shim_sig_unadjusted(link_name, args)?;

                packuswb(this, a, b, dest)?;
            }
            // Used to implement the _mm512_packs_epi32 function
            "packssdw.512" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512bw")?;

                let [a, b] = this.check_shim_sig_unadjusted(link_name, args)?;

                packssdw(this, a, b, dest)?;
            }
            // Used to implement the _mm512_packus_epi32 function
            "packusdw.512" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512bw")?;

                let [a, b] = this.check_shim_sig_unadjusted(link_name, args)?;

                packusdw(this, a, b, dest)?;
            }
            // Used to implement the _mm512_madd52lo_epu64 and _mm512_madd52hi_epu64
            // functions (and their 128/256-bit variants).
            "vpmadd52l.uq.512" | "vpmadd52h.uq.512" | "vpmadd52l.uq.256" | "vpmadd52h.uq.256"
            | "vpmadd52l.uq.128" | "vpmadd52h.uq.128" => {
                this.expect_target_feature_for_intrinsic(link_name, "avx512ifma")?;
                if !unprefixed_name.ends_with("512") {
                    this.expect_target_feature_for_intrinsic(link_name, "avx512vl")?;
                }

                let [z, x, y] = this.check_shim_sig_unadjusted(link_name, args)?;

                let high = unprefixed_name.starts_with("vpmadd52h");
                vpmadd52uq(this, z, x, y, high, dest)?;
            }
            _ => return interp_ok(EmulateItemResult::NotSupported),
        }
        interp_ok(EmulateItemResult::NeedsReturn)
    }
}

/// Multiply groups of 4 adjacent pairs of unsigned 8-bit integers in `a` with corresponding signed
/// 8-bit integers in `b`, producing 4 intermediate signed 16-bit results. Sum these 4 results with
/// the corresponding 32-bit integer in `src` (using wrapping arighmetic), and store the packed
/// 32-bit results in `dst`.
///
/// <https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_mm_dpbusd_epi32>
/// <https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_mm256_dpbusd_epi32>
/// <https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_mm512_dpbusd_epi32>
/// Multiply the low unsigned 52-bit integers in each 64-bit lane of `x` and
/// `y`, producing a 104-bit product, then add either its low (`high ==
/// false`) or high (`high == true`) 52-bit half to the full 64-bit lane of
/// `z` with wrapping arithmetic.
///
/// <https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_mm512_madd52lo_epu64>
/// <https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_mm512_madd52hi_epu64>
fn vpmadd52uq<'tcx>(
    ecx: &mut crate::MiriInterpCx<'tcx>,
    z: &OpTy<'tcx>,
    x: &OpTy<'tcx>,
    y: &OpTy<'tcx>,
    high: bool,
    dest: &MPlaceTy<'tcx>,
) -> InterpResult<'tcx, ()> {
    let (z, z_len) = ecx.project_to_simd(z)?;
    let (x, x_len) = ecx.project_to_simd(x)?;
    let (y, y_len) = ecx.project_to_simd(y)?;
    let (dest, dest_len) = ecx.project_to_simd(dest)?;

    // fn vpmadd52luq_512(z: i64x8, x: i64x8, y: i64x8) -> i64x8;
    assert_eq!(z_len, dest_len);
    assert_eq!(x_len, dest_len);
    assert_eq!(y_len, dest_len);

    const MASK52: u64 = (1 << 52) - 1;
    for i in 0..dest_len {
        let z = ecx.read_scalar(&ecx.project_index(&z, i)?)?.to_u64()?;
        let x = ecx.read_scalar(&ecx.project_index(&x, i)?)?.to_u64()?;
        let y = ecx.read_scalar(&ecx.project_index(&y, i)?)?.to_u64()?;
        let dest = ecx.project_index(&dest, i)?;

        let product = u128::from(x & MASK52).strict_mul(u128::from(y & MASK52));
        let shifted = if high { product >> 52 } else { product };
        let chunk = u64::try_from(shifted & u128::from(MASK52)).unwrap();
        let res = Scalar::from_u64(z.wrapping_add(chunk));
        ecx.write_scalar(res, &dest)?;
    }

    interp_ok(())
}

fn vpdpbusd<'tcx>(
    ecx: &mut crate::MiriInterpCx<'tcx>,
    src: &OpTy<'tcx>,
    a: &OpTy<'tcx>,
    b: &OpTy<'tcx>,
    dest: &MPlaceTy<'tcx>,
) -> InterpResult<'tcx, ()> {
    let (src, src_len) = ecx.project_to_simd(src)?;
    let (a, a_len) = ecx.project_to_simd(a)?;
    let (b, b_len) = ecx.project_to_simd(b)?;
    let (dest, dest_len) = ecx.project_to_simd(dest)?;

    // fn vpdpbusd(src: i32x16, a: u8x64, b: i8x64) -> i32x16;
    // fn vpdpbusd256(src: i32x8, a: u8x32, b: i8x32) -> i32x8;
    // fn vpdpbusd128(src: i32x4, a: u8x16, b: i8x16) -> i32x4;
    assert_eq!(src_len, dest_len);
    assert_eq!(a_len, dest_len.strict_mul(4));
    assert_eq!(b_len, a_len);

    for i in 0..dest_len {
        let src = ecx.read_scalar(&ecx.project_index(&src, i)?)?.to_i32()?;
        let dest = ecx.project_index(&dest, i)?;

        let mut intermediate_sum: i32 = 0;
        for j in 0..4 {
            let idx = i.strict_mul(4).strict_add(j);
            let a = ecx.read_scalar(&ecx.project_index(&a, idx)?)?.to_u8()?;
            let b = ecx.read_scalar(&ecx.project_index(&b, idx)?)?.to_i8()?;

            let product = i32::from(a).strict_mul(i32::from(b));
            intermediate_sum = intermediate_sum.strict_add(product);
        }

        // Use `wrapping_add` because `src` is an arbitrary i32 and the addition can overflow.
        let res = Scalar::from_i32(intermediate_sum.wrapping_add(src));
        ecx.write_scalar(res, &dest)?;
    }

    interp_ok(())
}
