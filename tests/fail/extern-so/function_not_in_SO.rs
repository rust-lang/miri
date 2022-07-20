//@only-target-linux
//@only-on-host
//@compile-flags: -Zmiri-extern-so-file=tests/extern-so/libtestlib.so

extern "C" {
    fn foo();
}

fn main() {
    unsafe {
        foo(); //~ ERROR: unsupported operation: can't call foreign function: foo
    }
}
