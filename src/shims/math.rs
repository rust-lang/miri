use core::num::Complex;

use rustc_abi::{CanonAbi, FieldIdx};
use rustc_apfloat::Float;
use rustc_apfloat::ieee::{DoubleS, HalfS, IeeeFloat, QuadS, Semantics, SingleS};
use rustc_middle::ty::Ty;
use rustc_span::Symbol;
use rustc_target::callconv::FnAbi;

use self::math::{ToHost, ToSoft};
use crate::*;

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn emulate_foreign_item_inner(
        &mut self,
        link_name: Symbol,
        abi: &FnAbi<'tcx, Ty<'tcx>>,
        args: &[OpTy<'tcx>],
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx, EmulateItemResult> {
        let this = self.eval_context_mut();

        // math functions (note that there are also intrinsics for some other functions)
        match link_name.as_str() {
            // math functions (note that there are also intrinsics for some other functions)
            #[rustfmt::skip]
            | "cbrtf"
            | "coshf"
            | "sinhf"
            | "tanf"
            | "tanhf"
            | "acosf"
            | "asinf"
            | "atanf"
            | "acoshf"
            | "asinhf"
            | "log1pf"
            | "expm1f"
            | "tgammaf"
            | "erff"
            | "erfcf"
            => {
                let [f] = this.check_shim_sig_deprecated(abi, CanonAbi::C , link_name, args)?;
                let f = this.read_scalar(f)?.to_f32()?;

                let res = math::fixed_float_value(this, link_name.as_str(), &[f]).unwrap_or_else(|| {
                    // Using host floats (but it's fine, these operations do not have
                    // guaranteed precision).
                    let f_host = f.to_host();
                    let res = match link_name.as_str() {
                        "cbrtf" => f_host.cbrt(),
                        "coshf" => f_host.cosh(),
                        "sinhf" => f_host.sinh(),
                        "tanf" => f_host.tan(),
                        "tanhf" => f_host.tanh(),
                        "acosf" => f_host.acos(),
                        "asinf" => f_host.asin(),
                        "atanf" => f_host.atan(),
                        "acoshf" => f_host.acosh(),
                        "asinhf" => f_host.asinh(),
                        "log1pf" => f_host.ln_1p(),
                        "expm1f" => f_host.exp_m1(),
                        "tgammaf" => f_host.gamma(),
                        "erff" => f_host.erf(),
                        "erfcf" => f_host.erfc(),
                        _ => bug!(),
                    };
                    let res = res.to_soft();
                    // Apply a relative error of 4ULP to introduce some non-determinism
                    // simulating imprecise implementations and optimizations.
                    let res = math::apply_random_float_error_ulp(this, res, 4);

                    // Clamp the result to the guaranteed range of this function according to the C standard,
                    // if any.
                    math::clamp_float_value(link_name.as_str(), res)
                });
                let res = this.adjust_nan(res, &[f]);
                this.write_scalar(res, dest)?;
            }
            #[rustfmt::skip]
            | "_hypotf"
            | "hypotf"
            | "atan2f"
            | "fdimf"
            => {
                let [f1, f2] = this.check_shim_sig_deprecated(abi, CanonAbi::C , link_name, args)?;
                let f1 = this.read_scalar(f1)?.to_f32()?;
                let f2 = this.read_scalar(f2)?.to_f32()?;

                let res = math::fixed_float_value(this, link_name.as_str(), &[f1, f2])
                    .unwrap_or_else(|| {
                        let res = match link_name.as_str() {
                            // underscore case for windows, here and below
                            // (see https://docs.microsoft.com/en-us/cpp/c-runtime-library/reference/floating-point-primitives?view=vs-2019)
                            // Using host floats (but it's fine, these operations do not have guaranteed precision).
                            "_hypotf" | "hypotf" => f1.to_host().hypot(f2.to_host()).to_soft(),
                            "atan2f" => f1.to_host().atan2(f2.to_host()).to_soft(),
                            #[allow(deprecated)]
                            "fdimf" => f1.to_host().abs_sub(f2.to_host()).to_soft(),
                            _ => bug!(),
                        };
                        // Apply a relative error of 4ULP to introduce some non-determinism
                        // simulating imprecise implementations and optimizations.
                        let res = math::apply_random_float_error_ulp(this, res, 4);

                        // Clamp the result to the guaranteed range of this function according to the C standard,
                        // if any.
                        math::clamp_float_value(link_name.as_str(), res)
                    });
                let res = this.adjust_nan(res, &[f1, f2]);
                this.write_scalar(res, dest)?;
            }
            #[rustfmt::skip]
            | "cbrt"
            | "cosh"
            | "sinh"
            | "tan"
            | "tanh"
            | "acos"
            | "asin"
            | "atan"
            | "acosh"
            | "asinh"
            | "log1p"
            | "expm1"
            | "tgamma"
            | "erf"
            | "erfc"
            => {
                let [f] = this.check_shim_sig_deprecated(abi, CanonAbi::C , link_name, args)?;
                let f = this.read_scalar(f)?.to_f64()?;

                let res = math::fixed_float_value(this, link_name.as_str(), &[f]).unwrap_or_else(|| {
                    // Using host floats (but it's fine, these operations do not have
                    // guaranteed precision).
                    let f_host = f.to_host();
                    let res = match link_name.as_str() {
                        "cbrt" => f_host.cbrt(),
                        "cosh" => f_host.cosh(),
                        "sinh" => f_host.sinh(),
                        "tan" => f_host.tan(),
                        "tanh" => f_host.tanh(),
                        "acos" => f_host.acos(),
                        "asin" => f_host.asin(),
                        "atan" => f_host.atan(),
                        "acosh" => f_host.acosh(),
                        "asinh" => f_host.asinh(),
                        "log1p" => f_host.ln_1p(),
                        "expm1" => f_host.exp_m1(),
                        "tgamma" => f_host.gamma(),
                        "erf" => f_host.erf(),
                        "erfc" => f_host.erfc(),
                        _ => bug!(),
                    };
                    let res = res.to_soft();
                    // Apply a relative error of 4ULP to introduce some non-determinism
                    // simulating imprecise implementations and optimizations.
                    let res = math::apply_random_float_error_ulp(this, res, 4);

                    // Clamp the result to the guaranteed range of this function according to the C standard,
                    // if any.
                    math::clamp_float_value(link_name.as_str(), res)
                });
                let res = this.adjust_nan(res, &[f]);
                this.write_scalar(res, dest)?;
            }
            #[rustfmt::skip]
            | "_hypot"
            | "hypot"
            | "atan2"
            | "fdim"
            => {
                let [f1, f2] = this.check_shim_sig_deprecated(abi, CanonAbi::C , link_name, args)?;
                let f1 = this.read_scalar(f1)?.to_f64()?;
                let f2 = this.read_scalar(f2)?.to_f64()?;

                let res = math::fixed_float_value(this, link_name.as_str(), &[f1, f2]).unwrap_or_else(|| {
                    let res = match link_name.as_str() {
                        // underscore case for windows, here and below
                        // (see https://docs.microsoft.com/en-us/cpp/c-runtime-library/reference/floating-point-primitives?view=vs-2019)
                        // Using host floats (but it's fine, these operations do not have guaranteed precision).
                        "_hypot" | "hypot" => f1.to_host().hypot(f2.to_host()).to_soft(),
                        "atan2" => f1.to_host().atan2(f2.to_host()).to_soft(),
                        #[allow(deprecated)]
                        "fdim" => f1.to_host().abs_sub(f2.to_host()).to_soft(),
                        _ => bug!(),
                    };
                    // Apply a relative error of 4ULP to introduce some non-determinism
                    // simulating imprecise implementations and optimizations.
                    let res = math::apply_random_float_error_ulp(this, res, 4);

                    // Clamp the result to the guaranteed range of this function according to the C standard,
                    // if any.
                    math::clamp_float_value(link_name.as_str(), res)
                });
                let res = this.adjust_nan(res, &[f1, f2]);
                this.write_scalar(res, dest)?;
            }
            #[rustfmt::skip]
            | "_ldexp"
            | "ldexp"
            | "scalbn"
            => {
                let [x, exp] = this.check_shim_sig_deprecated(abi, CanonAbi::C , link_name, args)?;
                // For radix-2 (binary) systems, `ldexp` and `scalbn` are the same.
                let x = this.read_scalar(x)?.to_f64()?;
                let exp = this.read_scalar(exp)?.to_i32()?;

                let res = x.scalbn(exp);
                let res = this.adjust_nan(res, &[x]);
                this.write_scalar(res, dest)?;
            }
            "lgammaf_r" => {
                let [x, signp] =
                    this.check_shim_sig_deprecated(abi, CanonAbi::C, link_name, args)?;
                let x = this.read_scalar(x)?.to_f32()?;
                let signp = this.deref_pointer_as(signp, this.machine.layouts.i32)?;

                // Using host floats (but it's fine, these operations do not have guaranteed precision).
                let (res, sign) = x.to_host().ln_gamma();
                this.write_int(sign, &signp)?;

                let res = res.to_soft();
                // Apply a relative error of 4ULP to introduce some non-determinism
                // simulating imprecise implementations and optimizations.
                let res = math::apply_random_float_error_ulp(this, res, 4);
                // Clamp the result to the guaranteed range of this function according to the C standard,
                // if any.
                let res = math::clamp_float_value(link_name.as_str(), res);
                let res = this.adjust_nan(res, &[x]);
                this.write_scalar(res, dest)?;
            }
            "lgamma_r" => {
                let [x, signp] =
                    this.check_shim_sig_deprecated(abi, CanonAbi::C, link_name, args)?;
                let x = this.read_scalar(x)?.to_f64()?;
                let signp = this.deref_pointer_as(signp, this.machine.layouts.i32)?;

                // Using host floats (but it's fine, these operations do not have guaranteed precision).
                let (res, sign) = x.to_host().ln_gamma();
                this.write_int(sign, &signp)?;

                let res = res.to_soft();
                // Apply a relative error of 4ULP to introduce some non-determinism
                // simulating imprecise implementations and optimizations.
                let res = math::apply_random_float_error_ulp(this, res, 4);
                // Clamp the result to the guaranteed range of this function according to the C standard,
                // if any.
                let res = math::clamp_float_value(link_name.as_str(), res);
                let res = this.adjust_nan(res, &[x]);
                this.write_scalar(res, dest)?;
            }

            // Complex multiplication.
            "__rust_mulhc3" => {
                let args = this.check_shim_sig(
                    shim_sig!(extern "Rust" fn(f16, f16, f16, f16) -> Complex<f16>),
                    (link_name, abi, args),
                )?;
                complex_binop::<HalfS>(this, complex_mul, args, dest)?;
            }
            "__mulsc3" => {
                let args = this.check_shim_sig(
                    shim_sig!(extern "C" fn(f32, f32, f32, f32) -> Complex<f32>),
                    (link_name, abi, args),
                )?;
                complex_binop::<SingleS>(this, complex_mul, args, dest)?;
            }
            "__muldc3" => {
                let args = this.check_shim_sig(
                    shim_sig!(extern "C" fn(f64, f64, f64, f64) -> Complex<f64>),
                    (link_name, abi, args),
                )?;
                complex_binop::<DoubleS>(this, complex_mul, args, dest)?;
            }
            "__rust_multc3" => {
                let args = this.check_shim_sig(
                    shim_sig!(extern "Rust" fn(f128, f128, f128, f128) -> Complex<f128>),
                    (link_name, abi, args),
                )?;
                complex_binop::<QuadS>(this, complex_mul, args, dest)?;
            }

            // Complex division.
            "__rust_divhc3" => {
                let args = this.check_shim_sig(
                    shim_sig!(extern "Rust" fn(f16, f16, f16, f16) -> Complex<f16>),
                    (link_name, abi, args),
                )?;
                complex_binop::<HalfS>(this, complex_div, args, dest)?;
            }
            "__divsc3" => {
                let args = this.check_shim_sig(
                    shim_sig!(extern "C" fn(f32, f32, f32, f32) -> Complex<f32>),
                    (link_name, abi, args),
                )?;
                complex_binop::<SingleS>(this, complex_div, args, dest)?;
            }
            "__divdc3" => {
                let args = this.check_shim_sig(
                    shim_sig!(extern "C" fn(f64, f64, f64, f64) -> Complex<f64>),
                    (link_name, abi, args),
                )?;
                complex_binop::<DoubleS>(this, complex_div, args, dest)?;
            }
            "__rust_divtc3" => {
                let args = this.check_shim_sig(
                    shim_sig!(extern "Rust" fn(f128, f128, f128, f128) -> Complex<f128>),
                    (link_name, abi, args),
                )?;
                complex_binop::<QuadS>(this, complex_div, args, dest)?;
            }

            _ => return interp_ok(EmulateItemResult::NotSupported),
        }

        interp_ok(EmulateItemResult::NeedsReturn)
    }
}

type ComplexFloatBinop<S> =
    fn(a: IeeeFloat<S>, b: IeeeFloat<S>, c: IeeeFloat<S>, d: IeeeFloat<S>) -> Complex<IeeeFloat<S>>;

fn complex_binop<'tcx, S: Semantics>(
    this: &mut MiriInterpCx<'tcx>,
    op: ComplexFloatBinop<S>,
    [lhs_re, lhs_im, rhs_re, rhs_im]: &[OpTy<'tcx>; 4],
    dest: &MPlaceTy<'tcx>,
) -> InterpResult<'tcx>
where
    IeeeFloat<S>: Into<Scalar>,
{
    let a: IeeeFloat<S> = this.read_scalar(lhs_re)?.to_float()?;
    let b: IeeeFloat<S> = this.read_scalar(lhs_im)?.to_float()?;
    let c: IeeeFloat<S> = this.read_scalar(rhs_re)?.to_float()?;
    let d: IeeeFloat<S> = this.read_scalar(rhs_im)?.to_float()?;

    let res = op(a, b, c, d);

    this.write_scalar(res.re, &this.project_field(dest, FieldIdx::ZERO)?)?;
    this.write_scalar(res.im, &this.project_field(dest, FieldIdx::ONE)?)?;
    interp_ok(())
}

/// Returns the product of `a + ib` and `c + id`.
///
/// This implementation uses the standard formula:
///
/// a+bi * c+di = ((ac - bd) + (ad + bc)i)
///
/// But with recovery of infinities if the above expresion results in NaN + NaNi.
///
/// Consider inf+NaNi * 0.0+1.0i, for which we expect the result to be 0+infi:
/// geometrically, a multiplication by i rotates the vector by 90 degrees.
///
/// But evaluating the standard formula computes inf*0.0 which results in a spurious
/// NaN, so the naive answer is NaN+NaNi. The slow path catches cases like this and
/// attempts to produce the correct answer.
///
/// This algorithm is defined in the C standard,
/// see <https://www.open-std.org/jtc1/sc22/wg14/www/docs/n3220.pdf#page=556>.
fn complex_mul<S: Semantics>(
    mut a: IeeeFloat<S>,
    mut b: IeeeFloat<S>,
    mut c: IeeeFloat<S>,
    mut d: IeeeFloat<S>,
) -> Complex<IeeeFloat<S>> {
    let ac = (a * c).value;
    let bd = (b * d).value;
    let ad = (a * d).value;
    let bc = (b * c).value;

    let z = Complex::new((ac - bd).value, (ad + bc).value);

    // The fast path: exit when at least one component is not NaN.
    if !(z.re.is_nan() && z.im.is_nan()) {
        return z;
    }

    let zero_if_nan =
        |x: IeeeFloat<S>| if x.is_nan() { IeeeFloat::<S>::ZERO.copy_sign(x) } else { x };

    let signed_unit_if_inf = |x: IeeeFloat<S>| {
        let mag =
            if x.is_infinite() { IeeeFloat::<S>::from_u128(1).value } else { IeeeFloat::<S>::ZERO };
        mag.copy_sign(x)
    };

    // Recover infinities that computed as NaN + iNaN.

    // Will be set to `true` if the double-NaN result is wrong and we have to do infinity recovery.
    let mut recalc = false;

    if a.is_infinite() || b.is_infinite() {
        // Replace infinities with a signed unit value, and NaN with zero.
        // This avoids invalid infinity arithmetic.
        a = signed_unit_if_inf(a);
        b = signed_unit_if_inf(b);
        c = zero_if_nan(c);
        d = zero_if_nan(d);
        recalc = true;
    }

    if c.is_infinite() || d.is_infinite() {
        // Replace infinities with a signed unit value, and NaN with zero.
        // This avoids invalid infinity arithmetic.
        a = zero_if_nan(a);
        b = zero_if_nan(b);
        c = signed_unit_if_inf(c);
        d = signed_unit_if_inf(d);
        recalc = true;
    }

    if !recalc && (ac.is_infinite() || bd.is_infinite() || ad.is_infinite() || bc.is_infinite()) {
        // Recover infinities from overflow by changing NaNs to zero.
        a = zero_if_nan(a);
        b = zero_if_nan(b);
        c = zero_if_nan(c);
        d = zero_if_nan(d);
        recalc = true;
    }

    if !recalc {
        return z;
    }

    let mut z = naive_complex_mul(a, b, c, d);
    z.re = (z.re * IeeeFloat::<S>::INFINITY).value;
    z.im = (z.im * IeeeFloat::<S>::INFINITY).value;

    z
}

/// Textbook complex multiplication, with no correction for NaN.
///
/// (a+bi) * (c+di) = (ac-bd) + (ad+bc)i
fn naive_complex_mul<S: Semantics>(
    a: IeeeFloat<S>,
    b: IeeeFloat<S>,
    c: IeeeFloat<S>,
    d: IeeeFloat<S>,
) -> Complex<IeeeFloat<S>> {
    let ac = (a * c).value;
    let bd = (b * d).value;
    let ad = (a * d).value;
    let bc = (b * c).value;

    Complex::new((ac - bd).value, (ad + bc).value)
}

/// Returns the quotient of `(a + ib)` and `(c + id)`.
///
/// This implementation uses the standard formula:
///
/// a+bi / c+di = ((ac + bd) / (c*c + d*d)) + ((bc - ad) / (c*c + d*d))i
///
/// But with recovery of infinities and zeros if the above expresion results in NaN + NaNi.
/// Consider inf+NaNi / 0.0+1.0i, for which we expect the result to be 0-infi:
/// geometrically, a division by i rotates the vector by -90 degrees.
///
/// But evaluating the standard formula computes inf*0.0 which results in a spurious
/// NaN, so the naive answer is NaN+NaNi. The slow path catches cases like this and
/// attempts to produce the correct answer.
///
/// This algorithm is defined in the C standard,
/// see <https://www.open-std.org/jtc1/sc22/wg14/www/docs/n3220.pdf#page=556>.
fn complex_div<S: Semantics>(
    mut a: IeeeFloat<S>,
    mut b: IeeeFloat<S>,
    mut c: IeeeFloat<S>,
    mut d: IeeeFloat<S>,
) -> Complex<IeeeFloat<S>> {
    // The denominator (c*c + d*d) is prone to overflow, even if the inputs and (exact) output are
    // perfectly representable in the given IeeeFloat.

    let max = IeeeFloat::<S>::max(c.abs(), d.abs());
    let mut neg_ilogbw = 0;

    // Scale c and d so that their base-2 exponent is zero.
    if max.is_finite() && max != IeeeFloat::<S>::ZERO {
        neg_ilogbw = max.ilogb().checked_neg().unwrap();
        c = c.scalbn(neg_ilogbw);
        d = d.scalbn(neg_ilogbw);
    }

    let cc = (c * c).value;
    let dd = (d * d).value;
    let denom = (cc + dd).value;

    let mut z = naive_complex_mul(a, b, c, -d);

    // Divide by the scaled-down denominator and then undo the scaling.
    z.re = (z.re / denom).value.scalbn(neg_ilogbw);
    z.im = (z.im / denom).value.scalbn(neg_ilogbw);

    // The fast path: exit when at least one component is not NaN.
    if !(z.re.is_nan() && z.im.is_nan()) {
        return z;
    }

    // Recover infinities and zeros that computed as NaN+iNaN.
    // The only cases are nonzero/zero, infinite/finite, and finite/infinite.

    let signed_unit_if_inf = |x: IeeeFloat<S>| {
        let mag =
            if x.is_infinite() { IeeeFloat::<S>::from_u128(1).value } else { IeeeFloat::<S>::ZERO };
        mag.copy_sign(x)
    };

    if denom == IeeeFloat::<S>::ZERO && (!a.is_nan() || !b.is_nan()) {
        z.re = (IeeeFloat::<S>::INFINITY.copy_sign(c) * a).value;
        z.im = (IeeeFloat::<S>::INFINITY.copy_sign(c) * b).value;
    } else if (a.is_infinite() || b.is_infinite()) && c.is_finite() && d.is_finite() {
        a = signed_unit_if_inf(a);
        b = signed_unit_if_inf(b);

        z = naive_complex_mul(a, b, c, -d);
        z.re = (IeeeFloat::<S>::INFINITY * z.re).value;
        z.im = (IeeeFloat::<S>::INFINITY * z.im).value;
    } else if max.is_infinite() && a.is_finite() && b.is_finite() {
        c = signed_unit_if_inf(c);
        d = signed_unit_if_inf(d);

        z = naive_complex_mul(a, b, c, -d);
        z.re = (IeeeFloat::<S>::ZERO * z.re).value;
        z.im = (IeeeFloat::<S>::ZERO * z.im).value;
    }

    z
}
