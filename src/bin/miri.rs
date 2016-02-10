#![feature(rustc_private)]

extern crate miri;
extern crate rustc;
extern crate rustc_driver;

use miri::interpreter;
use rustc::session::Session;
use rustc_driver::{driver, CompilerCalls, Compilation};

struct MiriCompilerCalls;

impl<'a> CompilerCalls<'a> for MiriCompilerCalls {
    fn build_controller(&mut self, _: &Session) -> driver::CompileController<'a> {
        let mut control = driver::CompileController::basic();
        control.after_analysis.stop = Compilation::Stop;

        control.after_analysis.callback = Box::new(|state| {
            interpreter::interpret_start_points(state.tcx.unwrap(), state.mir_map.unwrap());
        });

        control
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    args.push(String::from("--sysroot"));
    args.push(format!("{}/.multirust/toolchains/nightly", std::env::var("HOME").unwrap()));
    rustc_driver::run_compiler(&args, &mut MiriCompilerCalls);
}
