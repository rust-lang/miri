//@compile-flags: -Zmiri-genmc
//@revisions: u8_ u16_ u32_ u64_ usize_ i8_ i16_ i32_ i64_ isize_

// FIXME(genmc): ensure that 64 bit tests don't run on platforms without 64 bit atomics
// FIXME(genmc): add 128 bit tests for platforms that support it, once GenMC gets 128 bit atomic support

// This test check for correct handling of some edge cases with atomic read-modify-write operations for all integer sizes.
// Atomic max and min should return the previous value, and store the result in the atomic.
// Atomic addition and subtraction should have wrapping semantics.

#![no_main]

#[cfg(u8_)]
type Int = u8;
#[cfg(u8_)]
type AtomicInt = AtomicU8;

#[cfg(u16_)]
type Int = u16;
#[cfg(u16_)]
type AtomicInt = AtomicU16;

#[cfg(u32_)]
type Int = u32;
#[cfg(u32_)]
type AtomicInt = AtomicU32;

#[cfg(u64_)]
type Int = u64;
#[cfg(u64_)]
type AtomicInt = AtomicU64;

#[cfg(usize_)]
type Int = usize;
#[cfg(usize_)]
type AtomicInt = AtomicUsize;


#[cfg(i8_)]
type Int = i8;
#[cfg(i8_)]
type AtomicInt = AtomicI8;

#[cfg(i16_)]
type Int = i16;
#[cfg(i16_)]
type AtomicInt = AtomicI16;

#[cfg(i32_)]
type Int = i32;
#[cfg(i32_)]
type AtomicInt = AtomicI32;

#[cfg(i64_)]
type Int = i64;
#[cfg(i64_)]
type AtomicInt = AtomicI64;

#[cfg(isize_)]
type Int = isize;
#[cfg(isize_)]
type AtomicInt = AtomicIsize;

use std::sync::atomic::*;

const ORD: Ordering = Ordering::SeqCst;

fn assert_eq<T: Eq>(x: T, y: T) {
    if x != y {
        std::process::abort();
    }
}

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let x = AtomicInt::new(123);
    assert_eq(123, x.fetch_max(0, ORD)); // `max` keeps existing value
    assert_eq(123, x.fetch_max(Int::MAX, ORD)); // `max` stores the new value
    assert_eq(Int::MAX, x.fetch_add(10, ORD)); // `fetch_add` should be wrapping
    assert_eq(Int::MAX.wrapping_add(10), x.load(ORD));

    x.store(42, ORD);
    assert_eq(42, x.fetch_min(Int::MAX, ORD)); // `max` keeps existing value
    assert_eq(42, x.fetch_min(Int::MIN, ORD)); // `max` stores the new value
    assert_eq(Int::MIN, x.fetch_sub(10, ORD)); // `fetch_sub` should be wrapping
    assert_eq(Int::MIN.wrapping_sub(10), x.load(ORD));

    0
}
