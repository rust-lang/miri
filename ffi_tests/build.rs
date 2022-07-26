extern crate cc;

fn main() {
        cc::Build::new()
                    .file("src/test.c")
                            .compile("libtest.a");
}
