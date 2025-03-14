//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// This test check for correct handling of some edge cases with atomic read-modify-write operations for all integer sizes.
// Atomic max and min should return the previous value, and store the result in the atomic.
// Atomic addition and subtraction should have wrapping semantics.

// FIXME(genmc): add 128 bit atomics for platforms that support it, once GenMC gets 128 bit atomic support

#![no_main]

use std::sync::atomic::*;

const ORD: Ordering = Ordering::SeqCst;

fn assert_eq<T: Eq>(x: T, y: T) {
    if x != y {
        std::process::abort();
    }
}

macro_rules! test_rmw_edge_cases {
    ($int:ty, $atomic:ty) => {{
        let x = <$atomic>::new(123);
        // FIXME(genmc,HACK): remove this initializing write once Miri-GenMC supports mixed atomic-non-atomic accesses.
        x.store(123, ORD);

        assert_eq(123, x.fetch_max(0, ORD)); // `max` keeps existing value
        assert_eq(123, x.fetch_max(<$int>::MAX, ORD)); // `max` stores the new value
        assert_eq(<$int>::MAX, x.fetch_add(10, ORD)); // `fetch_add` should be wrapping
        assert_eq(<$int>::MAX.wrapping_add(10), x.load(ORD));

        x.store(42, ORD);
        assert_eq(42, x.fetch_min(<$int>::MAX, ORD)); // `min` keeps existing value
        assert_eq(42, x.fetch_min(<$int>::MIN, ORD)); // `min` stores the new value
        assert_eq(<$int>::MIN, x.fetch_sub(10, ORD)); // `fetch_sub` should be wrapping
        assert_eq(<$int>::MIN.wrapping_sub(10), x.load(ORD));
    }};
}

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    test_rmw_edge_cases!(u8, AtomicU8);
    test_rmw_edge_cases!(u16, AtomicU16);
    test_rmw_edge_cases!(u32, AtomicU32);
    test_rmw_edge_cases!(u64, AtomicU64);
    test_rmw_edge_cases!(usize, AtomicUsize);
    test_rmw_edge_cases!(i8, AtomicI8);
    test_rmw_edge_cases!(i16, AtomicI16);
    test_rmw_edge_cases!(i32, AtomicI32);
    test_rmw_edge_cases!(i64, AtomicI64);
    test_rmw_edge_cases!(isize, AtomicIsize);

    0
}
