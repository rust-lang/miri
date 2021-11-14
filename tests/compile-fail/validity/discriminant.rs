// error-pattern: type validation failed: encountered 0x02, but expected a boolean
#[allow(enum_intrinsics_non_enums)]
fn main() {
    let i = 2u8;
    // See https://github.com/rust-lang/rust/pull/89764: reading the discriminant asserts full validity.
    std::mem::discriminant(unsafe { &*(&i as *const _ as *const bool) });
}
