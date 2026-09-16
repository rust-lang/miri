//@ run-native
#![feature(cfg_target_has_reliable_f16_f128, complex_numbers, f16, f128)]

use core::num::Complex;

fn main() {
    complex_multiplication();
    complex_division();
}

unsafe extern "C" {
    pub(crate) safe fn __mulsc3(a: f32, b: f32, c: f32, d: f32) -> Complex<f32>;
    pub(crate) safe fn __muldc3(a: f64, b: f64, c: f64, d: f64) -> Complex<f64>;

    pub(crate) safe fn __divsc3(a: f32, b: f32, c: f32, d: f32) -> Complex<f32>;
    pub(crate) safe fn __divdc3(a: f64, b: f64, c: f64, d: f64) -> Complex<f64>;
}

unsafe extern "Rust" {
    pub(crate) safe fn __rust_mulhc3(a: f16, b: f16, c: f16, d: f16) -> Complex<f16>;
    pub(crate) safe fn __rust_multc3(a: f128, b: f128, c: f128, d: f128) -> Complex<f128>;

    pub(crate) safe fn __rust_divhc3(a: f16, b: f16, c: f16, d: f16) -> Complex<f16>;
    pub(crate) safe fn __rust_divtc3(a: f128, b: f128, c: f128, d: f128) -> Complex<f128>;
}

macro_rules! assert_is_infinity {
    ($e:expr) => {
        let e = $e;
        // A complex or imaginary value with at least one infinite part is regarded as an infinity
        // (even if its other part is a quiet NaN).
        assert!(e.re.is_infinite() || e.im.is_infinite(), "{e:?} is not infinite");
    };
}

fn complex_multiplication() {
    #[cfg(target_has_reliable_f16)]
    assert_eq!(__rust_mulhc3(1.0, 2.0, 3.0, 4.0), Complex::new(-5.0, 10.0));
    assert_eq!(__mulsc3(1.0, 2.0, 3.0, 4.0), Complex::new(-5.0, 10.0));
    assert_eq!(__muldc3(1.0, 2.0, 3.0, 4.0), Complex::new(-5.0, 10.0));
    #[cfg(target_has_reliable_f128)]
    assert_eq!(__rust_multc3(1.0, 2.0, 3.0, 4.0), Complex::new(-5.0, 10.0));

    // The naive algorithm would return NaN + NaNi for these inputs, but the libcall handles it.
    #[cfg(target_has_reliable_f16)]
    assert_eq!(
        __rust_mulhc3(1.0, 0.0, f16::INFINITY, f16::INFINITY),
        Complex::new(f16::INFINITY, f16::INFINITY)
    );
    assert_eq!(
        __mulsc3(1.0, 0.0, f32::INFINITY, f32::INFINITY),
        Complex::new(f32::INFINITY, f32::INFINITY)
    );
    assert_eq!(
        __muldc3(1.0, 0.0, f64::INFINITY, f64::INFINITY),
        Complex::new(f64::INFINITY, f64::INFINITY)
    );
    #[cfg(target_has_reliable_f128)]
    assert_eq!(
        __rust_multc3(1.0, 0.0, f128::INFINITY, f128::INFINITY),
        Complex::new(f128::INFINITY, f128::INFINITY)
    );

    let finite_nonzero = Complex::new(3.0f32, 7.0f32);

    fn mul(lhs: Complex<f32>, rhs: Complex<f32>) -> Complex<f32> {
        __mulsc3(lhs.re, lhs.im, rhs.re, rhs.im)
    }

    // If one operand is an infinity and the other operand is a nonzero finite number or an
    // infinity, then the result of the * operator is an infinity.
    assert_is_infinity!(mul(Complex::new(f32::INFINITY, f32::INFINITY), finite_nonzero));
    assert_is_infinity!(mul(finite_nonzero, Complex::new(f32::INFINITY, f32::INFINITY)));

    assert_is_infinity!(mul(Complex::new(f32::INFINITY, f32::NAN), finite_nonzero));
    assert_is_infinity!(mul(finite_nonzero, Complex::new(f32::INFINITY, f32::NAN)));

    assert_is_infinity!(mul(Complex::new(f32::NAN, f32::INFINITY), finite_nonzero));
    assert_is_infinity!(mul(finite_nonzero, Complex::new(f32::NAN, f32::INFINITY)));

    assert_is_infinity!(mul(Complex::new(f32::INFINITY, 3.0f32), finite_nonzero));
    assert_is_infinity!(mul(finite_nonzero, Complex::new(f32::INFINITY, 3.0f32)));
}

fn complex_division() {
    #[cfg(target_has_reliable_f16)]
    assert_eq!(__rust_divhc3(2.0, 11.0, 2.0, 1.0), Complex::new(3.0, 4.0));
    assert_eq!(__divsc3(2.0, 11.0, 2.0, 1.0), Complex::new(3.0, 4.0));
    assert_eq!(__divdc3(2.0, 11.0, 2.0, 1.0), Complex::new(3.0, 4.0));
    #[cfg(target_has_reliable_f128)]
    assert_eq!(__rust_divtc3(2.0, 11.0, 2.0, 1.0), Complex::new(3.0, 4.0));

    // The naive algorithm would return NaN + NaNi for these inputs, but the libcall handles it.
    // Check that all libcalls apply this correction.
    #[cfg(target_has_reliable_f16)]
    assert_eq!(
        __rust_divhc3(f16::INFINITY, 0.0, 1.0, 1.0),
        Complex::new(f16::INFINITY, f16::NEG_INFINITY)
    );
    assert_eq!(
        __divsc3(f32::INFINITY, 0.0, 1.0, 1.0),
        Complex::new(f32::INFINITY, f32::NEG_INFINITY)
    );
    assert_eq!(
        __divdc3(f64::INFINITY, 0.0, 1.0, 1.0),
        Complex::new(f64::INFINITY, f64::NEG_INFINITY)
    );
    #[cfg(target_has_reliable_f128)]
    assert_eq!(
        __rust_divtc3(f128::INFINITY, 0.0, 1.0, 1.0),
        Complex::new(f128::INFINITY, f128::NEG_INFINITY)
    );

    let finite_nonzero = Complex::new(3.0f32, 7.0f32);
    let zero = Complex::new(0.0, 0.0);

    fn div(lhs: Complex<f32>, rhs: Complex<f32>) -> Complex<f32> {
        __divsc3(lhs.re, lhs.im, rhs.re, rhs.im)
    }

    // If the first operand is an infinity and the second operand is a finite number,
    // then the result of the / operator is an infinity;
    assert_is_infinity!(div(Complex::new(f32::INFINITY, f32::INFINITY), finite_nonzero));
    assert_is_infinity!(div(Complex::new(f32::INFINITY, f32::NEG_INFINITY), finite_nonzero));
    assert_is_infinity!(div(Complex::new(f32::INFINITY, f32::NAN), finite_nonzero));
    assert_is_infinity!(div(Complex::new(f32::NAN, f32::INFINITY), finite_nonzero));
    assert_is_infinity!(div(Complex::new(3.0, f32::INFINITY), finite_nonzero));

    assert_is_infinity!(div(Complex::new(f32::INFINITY, f32::INFINITY), zero));
    assert_is_infinity!(div(Complex::new(f32::INFINITY, f32::NEG_INFINITY), zero));
    assert_is_infinity!(div(Complex::new(f32::INFINITY, f32::NAN), zero));
    assert_is_infinity!(div(Complex::new(f32::NAN, f32::INFINITY), zero));
    assert_is_infinity!(div(Complex::new(3.0, f32::INFINITY), zero));

    // if the first operand is a finite number and the second operand is an infinity,
    // then the result of the / operator is a zero
    assert_eq!(div(finite_nonzero, Complex::new(f32::INFINITY, f32::INFINITY)), zero);
    assert_eq!(div(finite_nonzero, Complex::new(f32::INFINITY, f32::NAN)), zero);

    assert_eq!(div(zero, Complex::new(f32::INFINITY, f32::INFINITY)), zero);
    assert_eq!(div(zero, Complex::new(f32::INFINITY, f32::NAN)), zero);

    // If the first operand is a nonzero finite number or an infinity and the second operand is a zero,
    // then the result of the / operator is an infinity
    assert_is_infinity!(div(finite_nonzero, zero));
    assert_is_infinity!(div(Complex::new(f32::INFINITY, f32::INFINITY), zero));
    assert_is_infinity!(div(Complex::new(f32::INFINITY, f32::NAN), zero));
}
