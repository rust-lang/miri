use rand::Rng as _;
use rand::distributions::Distribution as _;
use rustc_apfloat::Float as _;

use crate::operator::EvalContextExt as _;

pub(crate) trait FpMath: rustc_apfloat::Float + rustc_apfloat::FloatConvert<Self> {
    type M: fpmath::FloatMath;

    fn into_fp_math(self) -> Self::M;
    fn from_fp_math(value: Self::M) -> Self;
}

impl FpMath for rustc_apfloat::ieee::Single {
    type M = fpmath::SoftF32;

    fn into_fp_math(self) -> Self::M {
        fpmath::SoftF32::from_bits(self.to_bits().try_into().unwrap())
    }

    fn from_fp_math(value: Self::M) -> Self {
        rustc_apfloat::ieee::Single::from_bits(value.to_bits().into())
    }
}

impl FpMath for rustc_apfloat::ieee::Double {
    type M = fpmath::SoftF64;

    fn into_fp_math(self) -> Self::M {
        fpmath::SoftF64::from_bits(self.to_bits().try_into().unwrap())
    }

    fn from_fp_math(value: Self::M) -> Self {
        rustc_apfloat::ieee::Double::from_bits(value.to_bits().into())
    }
}

/// Disturbes a floating-point result by a relative error on the order of (-2^scale, 2^scale).
pub(crate) fn apply_random_float_error<F: rustc_apfloat::Float>(
    this: &mut crate::MiriInterpCx<'_>,
    val: F,
    err_scale: i32,
) -> F {
    let rng = this.machine.rng.get_mut();
    // Generate a random integer in the range [0, 2^PREC).
    let dist = rand::distributions::Uniform::new(0, 1 << F::PRECISION);
    let err = F::from_u128(dist.sample(rng))
        .value
        .scalbn(err_scale.strict_sub(F::PRECISION.try_into().unwrap()));
    // give it a random sign
    let err = if rng.gen::<bool>() { -err } else { err };
    // multiple the value with (1+err)
    (val * (F::from_u128(1).value + err).value).value
}

fn disturb_result<F: rustc_apfloat::Float>(this: &mut crate::MiriInterpCx<'_>, val: F) -> F {
    apply_random_float_error(this, val, 4 - i32::try_from(F::PRECISION).unwrap())
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Sqrt,
    Cbrt,
    Exp,
    Expm1,
    Exp2,
    Ln,
    Ln1p,
    Log2,
    Log10,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Gamma,
}

pub(crate) fn unary_op<F: FpMath>(this: &mut crate::MiriInterpCx<'_>, op: UnaryOp, x: F) -> F {
    let fp_x = x.into_fp_math();
    let res = match op {
        UnaryOp::Sqrt => fpmath::sqrt(fp_x),
        UnaryOp::Cbrt => fpmath::cbrt(fp_x),
        UnaryOp::Exp => fpmath::exp(fp_x),
        UnaryOp::Expm1 => fpmath::exp_m1(fp_x),
        UnaryOp::Exp2 => fpmath::exp2(fp_x),
        UnaryOp::Ln => fpmath::log(fp_x),
        UnaryOp::Ln1p => fpmath::log_1p(fp_x),
        UnaryOp::Log2 => fpmath::log2(fp_x),
        UnaryOp::Log10 => fpmath::log10(fp_x),
        UnaryOp::Sin => fpmath::sin(fp_x),
        UnaryOp::Cos => fpmath::cos(fp_x),
        UnaryOp::Tan => fpmath::tan(fp_x),
        UnaryOp::Asin => fpmath::asin(fp_x),
        UnaryOp::Acos => fpmath::acos(fp_x),
        UnaryOp::Atan => fpmath::atan(fp_x),
        UnaryOp::Sinh => fpmath::sinh(fp_x),
        UnaryOp::Cosh => fpmath::cosh(fp_x),
        UnaryOp::Tanh => fpmath::tanh(fp_x),
        UnaryOp::Gamma => fpmath::tgamma(fp_x),
    };
    let res = F::from_fp_math(res);
    // Only sqrt has guaranteed precision.
    let res = if op != UnaryOp::Sqrt { disturb_result(this, res) } else { res };
    this.adjust_nan(res, &[x])
}

pub(crate) fn sqrt<F: FpMath>(x: F) -> F {
    F::from_fp_math(fpmath::sqrt(x.into_fp_math()))
}

pub(crate) fn ln_gamma<F: FpMath>(this: &mut crate::MiriInterpCx<'_>, x: F) -> (F, i32) {
    let (res, sign) = fpmath::lgamma(x.into_fp_math());
    let res = F::from_fp_math(res);
    // Precision is not guaranteed.
    let res = disturb_result(this, res);
    let res = this.adjust_nan(res, &[x]);
    (res, sign.into())
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Hypot,
    Powf,
    Atan2,
    Fdim,
}

pub(crate) fn binary_op<F: FpMath + rustc_apfloat::FloatConvert<F>>(
    this: &mut crate::MiriInterpCx<'_>,
    op: BinaryOp,
    x: F,
    y: F,
) -> F {
    let fp_x = x.into_fp_math();
    let fp_y = y.into_fp_math();
    let res = match op {
        BinaryOp::Hypot => F::from_fp_math(fpmath::hypot(fp_x, fp_y)),
        BinaryOp::Powf => F::from_fp_math(fpmath::pow(fp_x, fp_y)),
        BinaryOp::Atan2 => F::from_fp_math(fpmath::atan2(fp_x, fp_y)),
        BinaryOp::Fdim => {
            // If `x` or `y` is NaN, the result is NaN.
            let diff = (x - y).value;
            if diff < F::ZERO { F::ZERO } else { diff }
        }
    };
    // Precision is not guaranteed.
    let res = if op == BinaryOp::Powf {
        // Special case exact 1^nan = 1
        // I'm not sure how to fix float_nan.rs test otherwise.
        if x == F::from_u128(1).value && y.is_nan() { res } else { disturb_result(this, res) }
    } else {
        disturb_result(this, res)
    };
    this.adjust_nan(res, &[x, y])
}

pub(crate) fn powi<F: FpMath + rustc_apfloat::FloatConvert<F>>(
    this: &mut crate::MiriInterpCx<'_>,
    x: F,
    y: i32,
) -> F {
    let fp_x = x.into_fp_math();
    let res = fpmath::powi(fp_x, y);
    let res = F::from_fp_math(res);
    // Precision is not guaranteed.
    let res = disturb_result(this, res);
    this.adjust_nan(res, &[x])
}
