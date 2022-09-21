//@compile-flags: -Zmiri-strict-provenance
#![feature(portable_simd, platform_intrinsics)]
use std::simd::*;

fn simd_ops_f32() {
    let a = f32x4::splat(10.0);
    let b = f32x4::from_array([1.0, 2.0, 3.0, -4.0]);
    assert_eq!(-b, f32x4::from_array([-1.0, -2.0, -3.0, 4.0]));
    assert_eq!(a + b, f32x4::from_array([11.0, 12.0, 13.0, 6.0]));
    assert_eq!(a - b, f32x4::from_array([9.0, 8.0, 7.0, 14.0]));
    assert_eq!(a * b, f32x4::from_array([10.0, 20.0, 30.0, -40.0]));
    assert_eq!(b / a, f32x4::from_array([0.1, 0.2, 0.3, -0.4]));
    assert_eq!(a / f32x4::splat(2.0), f32x4::splat(5.0));
    assert_eq!(a % b, f32x4::from_array([0.0, 0.0, 1.0, 2.0]));
    assert_eq!(b.abs(), f32x4::from_array([1.0, 2.0, 3.0, 4.0]));
    assert_eq!(a.simd_max(b * f32x4::splat(4.0)), f32x4::from_array([10.0, 10.0, 12.0, 10.0]));
    assert_eq!(a.simd_min(b * f32x4::splat(4.0)), f32x4::from_array([4.0, 8.0, 10.0, -16.0]));

    assert_eq!(a.mul_add(b, a), (a * b) + a);
    assert_eq!(b.mul_add(b, a), (b * b) + a);
    assert_eq!(a.mul_add(b, b), (a * b) + b);
    assert_eq!(
        f32x4::splat(-3.2).mul_add(b, f32x4::splat(f32::NEG_INFINITY)),
        f32x4::splat(f32::NEG_INFINITY)
    );
    assert_eq!((a * a).sqrt(), a);
    assert_eq!((b * b).sqrt(), b.abs());

    assert_eq!(a.simd_eq(f32x4::splat(5.0) * b), Mask::from_array([false, true, false, false]));
    assert_eq!(a.simd_ne(f32x4::splat(5.0) * b), Mask::from_array([true, false, true, true]));
    assert_eq!(a.simd_le(f32x4::splat(5.0) * b), Mask::from_array([false, true, true, false]));
    assert_eq!(a.simd_lt(f32x4::splat(5.0) * b), Mask::from_array([false, false, true, false]));
    assert_eq!(a.simd_ge(f32x4::splat(5.0) * b), Mask::from_array([true, true, false, true]));
    assert_eq!(a.simd_gt(f32x4::splat(5.0) * b), Mask::from_array([true, false, false, true]));

    assert_eq!(a.reduce_sum(), 40.0);
    assert_eq!(b.reduce_sum(), 2.0);
    assert_eq!(a.reduce_product(), 100.0 * 100.0);
    assert_eq!(b.reduce_product(), -24.0);
    assert_eq!(a.reduce_max(), 10.0);
    assert_eq!(b.reduce_max(), 3.0);
    assert_eq!(a.reduce_min(), 10.0);
    assert_eq!(b.reduce_min(), -4.0);

    assert_eq!(
        f32x2::from_array([0.0, f32::NAN]).simd_max(f32x2::from_array([f32::NAN, 0.0])),
        f32x2::from_array([0.0, 0.0])
    );
    assert_eq!(f32x2::from_array([0.0, f32::NAN]).reduce_max(), 0.0);
    assert_eq!(f32x2::from_array([f32::NAN, 0.0]).reduce_max(), 0.0);
    assert_eq!(
        f32x2::from_array([0.0, f32::NAN]).simd_min(f32x2::from_array([f32::NAN, 0.0])),
        f32x2::from_array([0.0, 0.0])
    );
    assert_eq!(f32x2::from_array([0.0, f32::NAN]).reduce_min(), 0.0);
    assert_eq!(f32x2::from_array([f32::NAN, 0.0]).reduce_min(), 0.0);
}

fn simd_ops_f64() {
    let a = f64x4::splat(10.0);
    let b = f64x4::from_array([1.0, 2.0, 3.0, -4.0]);
    assert_eq!(-b, f64x4::from_array([-1.0, -2.0, -3.0, 4.0]));
    assert_eq!(a + b, f64x4::from_array([11.0, 12.0, 13.0, 6.0]));
    assert_eq!(a - b, f64x4::from_array([9.0, 8.0, 7.0, 14.0]));
    assert_eq!(a * b, f64x4::from_array([10.0, 20.0, 30.0, -40.0]));
    assert_eq!(b / a, f64x4::from_array([0.1, 0.2, 0.3, -0.4]));
    assert_eq!(a / f64x4::splat(2.0), f64x4::splat(5.0));
    assert_eq!(a % b, f64x4::from_array([0.0, 0.0, 1.0, 2.0]));
    assert_eq!(b.abs(), f64x4::from_array([1.0, 2.0, 3.0, 4.0]));
    assert_eq!(a.simd_max(b * f64x4::splat(4.0)), f64x4::from_array([10.0, 10.0, 12.0, 10.0]));
    assert_eq!(a.simd_min(b * f64x4::splat(4.0)), f64x4::from_array([4.0, 8.0, 10.0, -16.0]));

    assert_eq!(a.mul_add(b, a), (a * b) + a);
    assert_eq!(b.mul_add(b, a), (b * b) + a);
    assert_eq!(a.mul_add(b, b), (a * b) + b);
    assert_eq!(
        f64x4::splat(-3.2).mul_add(b, f64x4::splat(f64::NEG_INFINITY)),
        f64x4::splat(f64::NEG_INFINITY)
    );
    assert_eq!((a * a).sqrt(), a);
    assert_eq!((b * b).sqrt(), b.abs());

    assert_eq!(a.simd_eq(f64x4::splat(5.0) * b), Mask::from_array([false, true, false, false]));
    assert_eq!(a.simd_ne(f64x4::splat(5.0) * b), Mask::from_array([true, false, true, true]));
    assert_eq!(a.simd_le(f64x4::splat(5.0) * b), Mask::from_array([false, true, true, false]));
    assert_eq!(a.simd_lt(f64x4::splat(5.0) * b), Mask::from_array([false, false, true, false]));
    assert_eq!(a.simd_ge(f64x4::splat(5.0) * b), Mask::from_array([true, true, false, true]));
    assert_eq!(a.simd_gt(f64x4::splat(5.0) * b), Mask::from_array([true, false, false, true]));

    assert_eq!(a.reduce_sum(), 40.0);
    assert_eq!(b.reduce_sum(), 2.0);
    assert_eq!(a.reduce_product(), 100.0 * 100.0);
    assert_eq!(b.reduce_product(), -24.0);
    assert_eq!(a.reduce_max(), 10.0);
    assert_eq!(b.reduce_max(), 3.0);
    assert_eq!(a.reduce_min(), 10.0);
    assert_eq!(b.reduce_min(), -4.0);

    assert_eq!(
        f64x2::from_array([0.0, f64::NAN]).simd_max(f64x2::from_array([f64::NAN, 0.0])),
        f64x2::from_array([0.0, 0.0])
    );
    assert_eq!(f64x2::from_array([0.0, f64::NAN]).reduce_max(), 0.0);
    assert_eq!(f64x2::from_array([f64::NAN, 0.0]).reduce_max(), 0.0);
    assert_eq!(
        f64x2::from_array([0.0, f64::NAN]).simd_min(f64x2::from_array([f64::NAN, 0.0])),
        f64x2::from_array([0.0, 0.0])
    );
    assert_eq!(f64x2::from_array([0.0, f64::NAN]).reduce_min(), 0.0);
    assert_eq!(f64x2::from_array([f64::NAN, 0.0]).reduce_min(), 0.0);
}

fn simd_ops_i32() {
    let a = i32x4::splat(10);
    let b = i32x4::from_array([1, 2, 3, -4]);
    assert_eq!(-b, i32x4::from_array([-1, -2, -3, 4]));
    assert_eq!(a + b, i32x4::from_array([11, 12, 13, 6]));
    assert_eq!(a - b, i32x4::from_array([9, 8, 7, 14]));
    assert_eq!(a * b, i32x4::from_array([10, 20, 30, -40]));
    assert_eq!(a / b, i32x4::from_array([10, 5, 3, -2]));
    assert_eq!(a / i32x4::splat(2), i32x4::splat(5));
    assert_eq!(i32x2::splat(i32::MIN) / i32x2::splat(-1), i32x2::splat(i32::MIN));
    assert_eq!(a % b, i32x4::from_array([0, 0, 1, 2]));
    assert_eq!(i32x2::splat(i32::MIN) % i32x2::splat(-1), i32x2::splat(0));
    assert_eq!(b.abs(), i32x4::from_array([1, 2, 3, 4]));
    assert_eq!(a.simd_max(b * i32x4::splat(4)), i32x4::from_array([10, 10, 12, 10]));
    assert_eq!(a.simd_min(b * i32x4::splat(4)), i32x4::from_array([4, 8, 10, -16]));

    assert_eq!(
        i8x4::from_array([i8::MAX, -23, 23, i8::MIN]).saturating_add(i8x4::from_array([
            1,
            i8::MIN,
            i8::MAX,
            28
        ])),
        i8x4::from_array([i8::MAX, i8::MIN, i8::MAX, -100])
    );
    assert_eq!(
        i8x4::from_array([i8::MAX, -28, 27, 42]).saturating_sub(i8x4::from_array([
            1,
            i8::MAX,
            i8::MAX,
            -80
        ])),
        i8x4::from_array([126, i8::MIN, -100, 122])
    );
    assert_eq!(
        u8x4::from_array([u8::MAX, 0, 23, 42]).saturating_add(u8x4::from_array([
            1,
            1,
            u8::MAX,
            200
        ])),
        u8x4::from_array([u8::MAX, 1, u8::MAX, 242])
    );
    assert_eq!(
        u8x4::from_array([u8::MAX, 0, 23, 42]).saturating_sub(u8x4::from_array([
            1,
            1,
            u8::MAX,
            200
        ])),
        u8x4::from_array([254, 0, 0, 0])
    );

    assert_eq!(!b, i32x4::from_array([!1, !2, !3, !-4]));
    assert_eq!(b << i32x4::splat(2), i32x4::from_array([4, 8, 12, -16]));
    assert_eq!(b >> i32x4::splat(1), i32x4::from_array([0, 1, 1, -2]));
    assert_eq!(b & i32x4::splat(2), i32x4::from_array([0, 2, 2, 0]));
    assert_eq!(b | i32x4::splat(2), i32x4::from_array([3, 2, 3, -2]));
    assert_eq!(b ^ i32x4::splat(2), i32x4::from_array([3, 0, 1, -2]));

    assert_eq!(a.simd_eq(i32x4::splat(5) * b), Mask::from_array([false, true, false, false]));
    assert_eq!(a.simd_ne(i32x4::splat(5) * b), Mask::from_array([true, false, true, true]));
    assert_eq!(a.simd_le(i32x4::splat(5) * b), Mask::from_array([false, true, true, false]));
    assert_eq!(a.simd_lt(i32x4::splat(5) * b), Mask::from_array([false, false, true, false]));
    assert_eq!(a.simd_ge(i32x4::splat(5) * b), Mask::from_array([true, true, false, true]));
    assert_eq!(a.simd_gt(i32x4::splat(5) * b), Mask::from_array([true, false, false, true]));

    assert_eq!(a.reduce_sum(), 40);
    assert_eq!(b.reduce_sum(), 2);
    assert_eq!(a.reduce_product(), 100 * 100);
    assert_eq!(b.reduce_product(), -24);
    assert_eq!(a.reduce_max(), 10);
    assert_eq!(b.reduce_max(), 3);
    assert_eq!(a.reduce_min(), 10);
    assert_eq!(b.reduce_min(), -4);

    assert_eq!(a.reduce_and(), 10);
    assert_eq!(b.reduce_and(), 0);
    assert_eq!(a.reduce_or(), 10);
    assert_eq!(b.reduce_or(), -1);
    assert_eq!(a.reduce_xor(), 0);
    assert_eq!(b.reduce_xor(), -4);
}

fn simd_mask() {
    let intmask = Mask::from_int(i32x4::from_array([0, -1, 0, 0]));
    assert_eq!(intmask, Mask::from_array([false, true, false, false]));
    assert_eq!(intmask.to_array(), [false, true, false, false]);

    let values = [
        true, false, false, true, false, false, true, false, true, true, false, false, false, true,
        false, true,
    ];
    let mask = Mask::<i64, 16>::from_array(values);
    let bitmask = mask.to_bitmask();
    assert_eq!(bitmask, 0b1010001101001001);
    assert_eq!(Mask::<i64, 16>::from_bitmask(bitmask), mask);

    let values = [false, false, false, true];
    let mask = Mask::<i64, 4>::from_array(values);
    let bitmask = mask.to_bitmask();
    assert_eq!(bitmask, 0b1000);
    assert_eq!(Mask::<i64, 4>::from_bitmask(bitmask), mask);
}

fn simd_cast() {
    // between integer types
    assert_eq!(i32x4::from_array([1, 2, 3, -4]), i16x4::from_array([1, 2, 3, -4]).cast());
    assert_eq!(i16x4::from_array([1, 2, 3, -4]), i32x4::from_array([1, 2, 3, -4]).cast());
    assert_eq!(i32x4::from_array([1, -1, 3, 4]), u64x4::from_array([1, u64::MAX, 3, 4]).cast());

    // float -> int
    assert_eq!(
        i8x4::from_array([127, -128, 127, -128]),
        f32x4::from_array([127.99, -128.99, 999.0, -999.0]).cast()
    );
    assert_eq!(
        i32x4::from_array([0, 1, -1, 2147483520]),
        f32x4::from_array([
            -0.0,
            /*0x1.19999ap+0*/ f32::from_bits(0x3f8ccccd),
            /*-0x1.19999ap+0*/ f32::from_bits(0xbf8ccccd),
            2147483520.0
        ])
        .cast()
    );
    assert_eq!(
        i32x8::from_array([i32::MAX, i32::MIN, i32::MAX, i32::MIN, i32::MAX, i32::MIN, 0, 0]),
        f32x8::from_array([
            2147483648.0f32,
            -2147483904.0f32,
            f32::MAX,
            f32::MIN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            -f32::NAN,
        ])
        .cast()
    );

    // int -> float
    assert_eq!(
        f32x4::from_array([
            -2147483648.0,
            /*0x1.26580cp+30*/ f32::from_bits(0x4e932c06),
            16777220.0,
            -16777220.0,
        ]),
        i32x4::from_array([-2147483647i32, 1234567890i32, 16777219i32, -16777219i32]).cast()
    );

    // float -> float
    assert_eq!(
        f32x4::from_array([f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY]),
        f64x4::from_array([f64::MAX, f64::INFINITY, f64::MIN, f64::NEG_INFINITY]).cast()
    );

    // unchecked casts
    unsafe {
        assert_eq!(
            i32x4::from_array([0, 1, -1, 2147483520]),
            f32x4::from_array([
                -0.0,
                /*0x1.19999ap+0*/ f32::from_bits(0x3f8ccccd),
                /*-0x1.19999ap+0*/ f32::from_bits(0xbf8ccccd),
                2147483520.0
            ])
            .to_int_unchecked()
        );
        assert_eq!(
            u64x4::from_array([0, 10000000000000000, u64::MAX - 2047, 9223372036854775808]),
            f64x4::from_array([
                -0.99999999999,
                1e16,
                (u64::MAX - 1024) as f64,
                9223372036854775808.0
            ])
            .to_int_unchecked()
        );
    }
}

fn simd_swizzle() {
    use Which::*;

    let a = f32x4::splat(10.0);
    let b = f32x4::from_array([1.0, 2.0, 3.0, -4.0]);

    assert_eq!(simd_swizzle!(b, [3, 0, 0, 2]), f32x4::from_array([-4.0, 1.0, 1.0, 3.0]));
    assert_eq!(simd_swizzle!(b, [1, 2]), f32x2::from_array([2.0, 3.0]));
    assert_eq!(simd_swizzle!(b, a, [First(3), Second(0)]), f32x2::from_array([-4.0, 10.0]));
}

fn simd_gather_scatter() {
    let mut vec: Vec<i16> = vec![10, 11, 12, 13, 14, 15, 16, 17, 18];
    let idxs = Simd::from_array([9, 3, 0, 17]);
    let result = Simd::gather_or_default(&vec, idxs); // Note the lane that is out-of-bounds.
    assert_eq!(result, Simd::from_array([0, 13, 10, 0]));

    let idxs = Simd::from_array([9, 3, 0, 0]);
    Simd::from_array([-27, 82, -41, 124]).scatter(&mut vec, idxs);
    assert_eq!(vec, vec![124, 11, 12, 82, 14, 15, 16, 17, 18]);
}

fn simd_round() {
    assert_eq!(
        f32x4::from_array([0.9, 1.001, 2.0, -4.5]).ceil(),
        f32x4::from_array([1.0, 2.0, 2.0, -4.0])
    );
    assert_eq!(
        f32x4::from_array([0.9, 1.001, 2.0, -4.5]).floor(),
        f32x4::from_array([0.0, 1.0, 2.0, -5.0])
    );
    assert_eq!(
        f32x4::from_array([0.9, 1.001, 2.0, -4.5]).round(),
        f32x4::from_array([1.0, 1.0, 2.0, -5.0])
    );
    assert_eq!(
        f32x4::from_array([0.9, 1.001, 2.0, -4.5]).trunc(),
        f32x4::from_array([0.0, 1.0, 2.0, -4.0])
    );

    assert_eq!(
        f64x4::from_array([0.9, 1.001, 2.0, -4.5]).ceil(),
        f64x4::from_array([1.0, 2.0, 2.0, -4.0])
    );
    assert_eq!(
        f64x4::from_array([0.9, 1.001, 2.0, -4.5]).floor(),
        f64x4::from_array([0.0, 1.0, 2.0, -5.0])
    );
    assert_eq!(
        f64x4::from_array([0.9, 1.001, 2.0, -4.5]).round(),
        f64x4::from_array([1.0, 1.0, 2.0, -5.0])
    );
    assert_eq!(
        f64x4::from_array([0.9, 1.001, 2.0, -4.5]).trunc(),
        f64x4::from_array([0.0, 1.0, 2.0, -4.0])
    );
}

fn simd_intrinsics() {
    extern "platform-intrinsic" {
        fn simd_eq<T, U>(x: T, y: T) -> U;
        fn simd_reduce_any<T>(x: T) -> bool;
        fn simd_reduce_all<T>(x: T) -> bool;
        fn simd_select<M, T>(m: M, yes: T, no: T) -> T;
    }
    unsafe {
        // Make sure simd_eq returns all-1 for `true`
        let a = i32x4::splat(10);
        let b = i32x4::from_array([1, 2, 10, 4]);
        let c: i32x4 = simd_eq(a, b);
        assert_eq!(c, i32x4::from_array([0, 0, -1, 0]));

        assert!(!simd_reduce_any(i32x4::splat(0)));
        assert!(simd_reduce_any(i32x4::splat(-1)));
        assert!(simd_reduce_any(i32x2::from_array([0, -1])));
        assert!(!simd_reduce_all(i32x4::splat(0)));
        assert!(simd_reduce_all(i32x4::splat(-1)));
        assert!(!simd_reduce_all(i32x2::from_array([0, -1])));

        assert_eq!(
            simd_select(i8x4::from_array([0, -1, -1, 0]), a, b),
            i32x4::from_array([1, 10, 10, 4])
        );
        assert_eq!(
            simd_select(i8x4::from_array([0, -1, -1, 0]), b, a),
            i32x4::from_array([10, 2, 10, 10])
        );
    }
}

fn main() {
    simd_mask();
    simd_ops_f32();
    simd_ops_f64();
    simd_ops_i32();
    simd_cast();
    simd_swizzle();
    simd_gather_scatter();
    simd_round();
    simd_intrinsics();
}
