//@only-target-linux
//@only-on-host

extern "C" {
    fn foo();
}

fn main() {
    unsafe {
        foo(); //~ ERROR: unsupported operation: can't call foreign function: foo
    }
}
