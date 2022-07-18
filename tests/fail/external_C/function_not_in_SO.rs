//@only-target-linux
//@only-on-host
//@compile-flags: -Zmiri-external_c_so_file=tests/external_C/libtestlib.so

extern "C" {
    fn foo();
}

fn main() {
    unsafe {
        foo(); //~ ERROR: unsupported operation: can't call foreign function: foo; try specifying a shared object file with the flag -Zmiri-external_c_so_file=path/to/SOfile
    }
}
