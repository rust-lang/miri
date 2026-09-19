#![feature(complex_numbers)]

use core::num::Complex;

fn main() {
    complex_multiplication();
    complex_division();
}

fn complex_multiplication() {
    #[cfg(target_has_reliable_f16)]
    assert_eq!(Complex::new(1.0f16, 2.0) * Complex::new(3.0, 4.0), Complex::new(-5.0, 10.0));
    assert_eq!(Complex::new(1.0f32, 2.0) * Complex::new(3.0, 4.0), Complex::new(-5.0, 10.0));
    assert_eq!(Complex::new(1.0f64, 2.0) * Complex::new(3.0, 4.0), Complex::new(-5.0, 10.0));
    #[cfg(target_has_reliable_f128)]
    assert_eq!(Complex::new(1.0f128, 2.0) * Complex::new(3.0, 4.0), Complex::new(-5.0, 10.0));

    // The naive algorithm would return NaN + NaNi for these inputs, but the libcall handles it.
    #[cfg(target_has_reliable_f16)]
    assert_eq!(
        Complex::new(1.0, 0.0) * Complex::new(f16::INFINITY, f16::INFINITY),
        Complex::new(f16::INFINITY, f16::INFINITY)
    );
    assert_eq!(
        Complex::new(1.0, 0.0) * Complex::new(f32::INFINITY, f32::INFINITY),
        Complex::new(f32::INFINITY, f32::INFINITY)
    );
    assert_eq!(
        Complex::new(1.0, 0.0) * Complex::new(f64::INFINITY, f64::INFINITY),
        Complex::new(f64::INFINITY, f64::INFINITY)
    );
    #[cfg(target_has_reliable_f128)]
    assert_eq!(
        Complex::new(1.0, 0.0) * Complex::new(f128::INFINITY, f128::INFINITY),
        Complex::new(f128::INFINITY, f128::INFINITY)
    );
}

fn complex_division() {
    #[cfg(target_has_reliable_f16)]
    assert_eq!(Complex::new(2.0f16, 11.0) / Complex::new(2.0, 1.0), Complex::new(3.0, 4.0));
    assert_eq!(Complex::new(2.0f32, 11.0) / Complex::new(2.0, 1.0), Complex::new(3.0, 4.0));
    assert_eq!(Complex::new(2.0f64, 11.0) / Complex::new(2.0, 1.0), Complex::new(3.0, 4.0));
    #[cfg(target_has_reliable_f128)]
    assert_eq!(Complex::new(2.0f128, 11.0) / Complex::new(2.0, 1.0), Complex::new(3.0, 4.0));

    // The naive algorithm would return NaN + NaNi for these inputs, but the libcall handles it.
    #[cfg(target_has_reliable_f16)]
    assert_eq!(
        Complex::new(f16::INFINITY, 0.0) / Complex::new(1.0, 1.0),
        Complex::new(f16::INFINITY, f16::NEG_INFINITY)
    );
    assert_eq!(
        Complex::new(f32::INFINITY, 0.0) / Complex::new(1.0, 1.0),
        Complex::new(f32::INFINITY, f32::NEG_INFINITY)
    );
    assert_eq!(
        Complex::new(f64::INFINITY, 0.0) / Complex::new(1.0, 1.0),
        Complex::new(f64::INFINITY, f64::NEG_INFINITY)
    );
    #[cfg(target_has_reliable_f128)]
    assert_eq!(
        Complex::new(f128::INFINITY, 0.0) / Complex::new(1.0, 1.0),
        Complex::new(f128::INFINITY, f128::NEG_INFINITY)
    );
}
