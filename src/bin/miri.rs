#![feature(rustc_private)]

extern crate getopts;
extern crate miri;
extern crate rustc;
extern crate rustc_metadata;
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_codegen_utils;
extern crate rustc_interface;
extern crate env_logger;
extern crate log_settings;
extern crate syntax;

#[macro_use]
extern crate log;

use std::str::FromStr;
use std::env;

use rustc_interface::interface;
use rustc::hir::def_id::LOCAL_CRATE;

struct MiriCompilerCalls {
    /// Whether to enforce the validity invariant.
    validate: bool,
}

impl rustc_driver::Callbacks for MiriCompilerCalls {
    fn after_parsing(&mut self, compiler: &interface::Compiler<'_>) -> bool {
        let attr = (
            String::from("miri"),
            syntax::feature_gate::AttributeType::Whitelisted,
        );
        compiler.session().plugin_attributes.borrow_mut().push(attr);

        // Continue execution
        true
    }

    fn after_analysis(&mut self, compiler: &interface::Compiler<'_>) -> bool {
        init_late_loggers();
        compiler.session().abort_if_errors();

        compiler.global_ctxt().unwrap().peek_mut().enter(|tcx| {
            let (entry_def_id, _) = tcx.entry_fn(LOCAL_CRATE).expect("no main function found!");

            miri::eval_main(tcx, entry_def_id, self.validate);
        });

        compiler.session().abort_if_errors();

        // Don't continue execution
        false
    }
}

fn init_early_loggers() {
    // Notice that our `extern crate log` is NOT the same as rustc's!  So we have to initialize
    // them both.  We always initialize miri early.
    let env = env_logger::Env::new().filter("MIRI_LOG").write_style("MIRI_LOG_STYLE");
    env_logger::init_from_env(env);
    // We only initialize rustc if the env var is set (so the user asked for it).
    // If it is not set, we avoid initializing now so that we can initialize
    // later with our custom settings, and NOT log anything for what happens before
    // miri gets started.
    if env::var("RUST_LOG").is_ok() {
        rustc_driver::init_rustc_env_logger();
    }
}

fn init_late_loggers() {
    // Initializing loggers right before we start evaluation.  We overwrite the RUST_LOG
    // env var if it is not set, control it based on MIRI_LOG.
    if let Ok(var) = env::var("MIRI_LOG") {
        if env::var("RUST_LOG").is_err() {
            // We try to be a bit clever here: If MIRI_LOG is just a single level
            // used for everything, we only apply it to the parts of rustc that are
            // CTFE-related.  Otherwise, we use it verbatim for RUST_LOG.
            // This way, if you set `MIRI_LOG=trace`, you get only the right parts of
            // rustc traced, but you can also do `MIRI_LOG=miri=trace,rustc_mir::interpret=debug`.
            if log::Level::from_str(&var).is_ok() {
                env::set_var("RUST_LOG",
                    &format!("rustc::mir::interpret={0},rustc_mir::interpret={0}", var));
            } else {
                env::set_var("RUST_LOG", &var);
            }
            rustc_driver::init_rustc_env_logger();
        }
    }

    // If MIRI_BACKTRACE is set and RUST_CTFE_BACKTRACE is not, set RUST_CTFE_BACKTRACE.
    // Do this late, so we really only apply this to miri's errors.
    if let Ok(var) = env::var("MIRI_BACKTRACE") {
        if env::var("RUST_CTFE_BACKTRACE") == Err(env::VarError::NotPresent) {
            env::set_var("RUST_CTFE_BACKTRACE", &var);
        }
    }
}

fn find_sysroot() -> String {
    if let Ok(sysroot) = std::env::var("MIRI_SYSROOT") {
        return sysroot;
    }

    // Taken from https://github.com/Manishearth/rust-clippy/pull/911.
    let home = option_env!("RUSTUP_HOME").or(option_env!("MULTIRUST_HOME"));
    let toolchain = option_env!("RUSTUP_TOOLCHAIN").or(option_env!("MULTIRUST_TOOLCHAIN"));
    match (home, toolchain) {
        (Some(home), Some(toolchain)) => format!("{}/toolchains/{}", home, toolchain),
        _ => {
            option_env!("RUST_SYSROOT")
                .expect(
                    "Could not find sysroot. Either set MIRI_SYSROOT at run-time, or at \
                     build-time specify RUST_SYSROOT env var or use rustup or multirust",
                )
                .to_owned()
        }
    }
}

fn main() {
    init_early_loggers();
    let mut args: Vec<String> = std::env::args().collect();

    // Parse our own -Z flags and remove them before rustc gets their hand on them.
    let mut validate = true;
    args.retain(|arg| {
        match arg.as_str() {
            "-Zmiri-disable-validation" => {
                validate = false;
                false
            },
            _ => true
        }
    });

    // Determine sysroot and let rustc know about it
    let sysroot_flag = String::from("--sysroot");
    if !args.contains(&sysroot_flag) {
        args.push(sysroot_flag);
        args.push(find_sysroot());
    }
    // Finally, add the default flags all the way in the beginning, but after the binary name.
    args.splice(1..1, miri::miri_default_args().iter().map(ToString::to_string));

    trace!("rustc arguments: {:?}", args);
    let result = rustc_driver::report_ices_to_stderr_if_any(move || {
        rustc_driver::run_compiler(&args, &mut MiriCompilerCalls { validate }, None, None)
    }).and_then(|result| result);
    std::process::exit(result.is_err() as i32);
}
