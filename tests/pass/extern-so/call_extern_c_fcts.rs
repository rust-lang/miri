//@only-target-linux
//@only-on-host
//@compile-flags: -Zmiri-extern-so-file=tests/extern-so/libtestlib.so

extern "C" {
    fn double_deref(x: *const *const i32) -> i32;
    fn add_one_int(x: i32) -> i32;
    fn add_int16(x: i16) -> i16;
    fn test_stack_spill(
        a: i32,
        b: i32,
        c: i32,
        d: i32,
        e: i32,
        f: i32,
        g: i32,
        h: i32,
        i: i32,
        j: i32,
        k: i32,
        l: i32,
    ) -> i32;
    fn add_short_to_long(x: i16, y: i64) -> i64;
    fn get_unsigned_int() -> u32;
    fn printer();
}

fn main() {
    unsafe {
        // test function that adds 2 to a provided int
        assert_eq!(add_one_int(1), 3);

        // test function that takes the sum of its 12 arguments
        assert_eq!(test_stack_spill(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12), 78);

        // test function that adds 3 to a 16 bit int
        assert_eq!(add_int16(-1i16), 2i16);

        // test function that adds an i16 to an i64
        assert_eq!(add_short_to_long(-1i16, 123456789123i64), 123456789122i64);

        // test function that returns -10 as an unsigned int
        assert_eq!(get_unsigned_int(), (-10i32) as u32);

        // test void function that prints from C -- call it twice
        printer();
        printer();

        let base: i32 = 42;
        let base_p: *const i32 = &base as *const i32;
        let base_pp: *const *const i32 = &base_p as *const *const i32;
        assert_eq!(double_deref(base_pp), 42);
    }
}
