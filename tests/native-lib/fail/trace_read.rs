//@only-target: linux
//@only-target: gnu
//@only-target: x86
//@only-on-host
//@compile-flags: -Zmiri-native-lib-enable-tracing

extern "C" {
    fn do_nothing();
}

fn main() {
    unexposed_reachable_alloc();
}

fn unexposed_reachable_alloc() {
    let inner = 42;
    let intermediate = &raw const inner;
    let exposed = (&raw const intermediate).expose_provenance();
    unsafe { do_nothing() };
    let invalid: *const i32 = std::ptr::with_exposed_provenance(intermediate.addr());
    let valid: *const *const i32 = std::ptr::with_exposed_provenance(exposed);
    unsafe {
        assert_ne!((*valid).addr(), 0);
        println!("{}", *invalid); //~ ERROR: Undefined Behavior: pointer not dereferenceable: pointer must be dereferenceable for 4 bytes, but got
    }
}
