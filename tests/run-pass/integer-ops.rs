#![allow(arithmetic_overflow)]

pub fn main() {
    // This tests that do (not) do sign extension properly when loading integers
    assert_eq!(u32::MAX as i64, 4294967295);
    assert_eq!(i32::MIN as i64, -2147483648);

    assert_eq!(i8::MAX, 127);
    assert_eq!(i8::MIN, -128);

    assert_eq!(i32::from_str_radix("A", 16), Ok(10));

    let n = -0b1000_0000i8;
    assert_eq!(n.count_ones(), 1);

    let n = -0b1000_0000i8;
    assert_eq!(n.count_zeros(), 7);

    let n = -1i16;
    assert_eq!(n.leading_zeros(), 0);

    let n = -4i8;
    assert_eq!(n.trailing_zeros(), 2);

    let n = 0x0123456789ABCDEFi64;
    let m = -0x76543210FEDCBA99i64;
    assert_eq!(n.rotate_left(32), m);

    let n = 0x0123456789ABCDEFi64;
    let m = -0xFEDCBA987654322i64;
    assert_eq!(n.rotate_right(4), m);

    let n =  0x0123456789ABCDEFi64;
    let m = -0x1032547698BADCFFi64;
    assert_eq!(n.swap_bytes(), m);

    let n = 0x0123456789ABCDEFi64;
    if cfg!(target_endian = "big") {
        assert_eq!(i64::from_be(n), n)
    } else {
        assert_eq!(i64::from_be(n), n.swap_bytes())
    }

    let n = 0x0123456789ABCDEFi64;
    if cfg!(target_endian = "little") {
        assert_eq!(i64::from_le(n), n)
    } else {
        assert_eq!(i64::from_le(n), n.swap_bytes())
    }

    let n = 0x0123456789ABCDEFi64;
    if cfg!(target_endian = "big") {
        assert_eq!(n.to_be(), n)
    } else {
        assert_eq!(n.to_be(), n.swap_bytes())
    }

    let n = 0x0123456789ABCDEFi64;
    if cfg!(target_endian = "little") {
        assert_eq!(n.to_le(), n)
    } else {
        assert_eq!(n.to_le(), n.swap_bytes())
    }

    assert_eq!(7i16.checked_add(32760), Some(32767));
    assert_eq!(8i16.checked_add(32760), None);

    assert_eq!((-127i8).checked_sub(1), Some(-128));
    assert_eq!((-128i8).checked_sub(1), None);

    assert_eq!(6i8.checked_mul(21), Some(126));
    assert_eq!(6i8.checked_mul(22), None);

    assert_eq!((-127i8).checked_div(-1), Some(127));
    assert_eq!((-128i8).checked_div(-1), None);
    assert_eq!((1i8).checked_div(0), None);

    assert_eq!(5i32.checked_rem(2), Some(1));
    assert_eq!(5i32.checked_rem(0), None);
    assert_eq!(i32::MIN.checked_rem(-1), None);

    assert_eq!(5i32.checked_neg(), Some(-5));
    assert_eq!(i32::MIN.checked_neg(), None);

    assert_eq!(0x10i32.checked_shl(4), Some(0x100));
    assert_eq!(0x10i32.checked_shl(33), None);

    assert_eq!(0x10i32.checked_shr(4), Some(0x1));
    assert_eq!(0x10i32.checked_shr(33), None);

    assert_eq!((-5i32).checked_abs(), Some(5));
    assert_eq!(i32::MIN.checked_abs(), None);

    assert_eq!(100i8.saturating_add(1), 101);
    assert_eq!(100i8.saturating_add(127), 127);

    assert_eq!(100i8.saturating_sub(127), -27);
    assert_eq!((-100i8).saturating_sub(127), -128);

    assert_eq!(100i32.saturating_mul(127), 12700);
    assert_eq!((1i32 << 23).saturating_mul(1 << 23), i32::MAX);
    assert_eq!((-1i32 << 23).saturating_mul(1 << 23), i32::MIN);

    assert_eq!(100i8.wrapping_add(27), 127);
    assert_eq!(100i8.wrapping_add(127), -29);

    assert_eq!(0i8.wrapping_sub(127), -127);
    assert_eq!((-2i8).wrapping_sub(127), 127);

    assert_eq!(10i8.wrapping_mul(12), 120);
    assert_eq!(11i8.wrapping_mul(12), -124);

    assert_eq!(100u8.wrapping_div(10), 10);
    assert_eq!((-128i8).wrapping_div(-1), -128);

    assert_eq!(100i8.wrapping_rem(10), 0);
    assert_eq!((-128i8).wrapping_rem(-1), 0);

    assert_eq!(100i8.wrapping_neg(), -100);
    assert_eq!((-128i8).wrapping_neg(), -128);

    assert_eq!((-1i8).wrapping_shl(7), -128);
    assert_eq!((-1i8).wrapping_shl(8), -1);

    assert_eq!((-128i8).wrapping_shr(7), -1);
    assert_eq!((-128i8).wrapping_shr(8), -128);

    assert_eq!(100i8.wrapping_abs(), 100);
    assert_eq!((-100i8).wrapping_abs(), 100);
    assert_eq!((-128i8).wrapping_abs(), -128);
    assert_eq!((-128i8).wrapping_abs() as u8, 128);

    assert_eq!(5i32.overflowing_add(2), (7, false));
    assert_eq!(i32::MAX.overflowing_add(1), (i32::MIN, true));

    assert_eq!(5i32.overflowing_sub(2), (3, false));
    assert_eq!(i32::MIN.overflowing_sub(1), (i32::MAX, true));

    assert_eq!(5i32.overflowing_mul(2), (10, false));
    assert_eq!(1_000_000_000i32.overflowing_mul(10), (1410065408, true));

    assert_eq!(5i32.overflowing_div(2), (2, false));
    assert_eq!(i32::MIN.overflowing_div(-1), (i32::MIN, true));

    assert_eq!(5i32.overflowing_rem(2), (1, false));
    assert_eq!(i32::MIN.overflowing_rem(-1), (0, true));

    assert_eq!(2i32.overflowing_neg(), (-2, false));
    assert_eq!(i32::MIN.overflowing_neg(), (i32::MIN, true));

    assert_eq!(0x10i32.overflowing_shl(4), (0x100, false));
    assert_eq!(0x10i32.overflowing_shl(36), (0x100, true));

    assert_eq!(0x10i32.overflowing_shr(4), (0x1, false));
    assert_eq!(0x10i32.overflowing_shr(36), (0x1, true));

    assert_eq!(10i8.overflowing_abs(), (10,false));
    assert_eq!((-10i8).overflowing_abs(), (10,false));
    assert_eq!((-128i8).overflowing_abs(), (-128,true));
}
