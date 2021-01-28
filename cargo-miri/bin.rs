use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::iter::TakeWhile;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::ops::Not;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use rustc_version::VersionMeta;

const XARGO_MIN_VERSION: (u32, u32, u32) = (0, 3, 22);

const CARGO_MIRI_HELP: &str = r#"Runs binary crates and tests in Miri

Usage:
    cargo miri [subcommand] [<cargo options>...] [--] [<program/test suite options>...]

Subcommands:
    run                      Run binaries
    test                     Run tests
    setup                    Only perform automatic setup, but without asking questions (for getting a proper libstd)

The cargo options are exactly the same as for `cargo run` and `cargo test`, respectively.

Examples:
    cargo miri run
    cargo miri test -- test-suite-filter
"#;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum MiriCommand {
    Run,
    Test,
    Setup,
}

/// The information to run a crate with the given environment.
#[derive(Serialize, Deserialize)]
struct CrateRunEnv {
    /// The command-line arguments.
    args: Vec<String>,
    /// The environment.
    env: Vec<(OsString, OsString)>,
    /// The current working directory.
    current_dir: OsString,
    /// The contents passed via standard input.
    stdin: Vec<u8>,
}

/// The information Miri needs to run a crate. Stored as JSON when the crate is "compiled".
#[derive(Serialize, Deserialize)]
enum CrateRunInfo {
    /// Run it with the given environment.
    RunWith(CrateRunEnv),
    /// Skip it as Miri does not support interpreting such kind of crates.
    SkipProcMacroTest,
}

impl CrateRunInfo {
    /// Gather all the information we need.
    fn collect(args: env::Args) -> Self {
        let args = args.collect();
        let env = env::vars_os().collect();
        let current_dir = env::current_dir().unwrap().into_os_string();

        let mut stdin = Vec::new();
        if env::var_os("MIRI_CALLED_FROM_RUSTDOC").is_some() {
            std::io::stdin().lock().read_to_end(&mut stdin).expect("cannot read stdin");
        }

        Self::RunWith(CrateRunEnv { args, env, current_dir, stdin })
    }

    fn store(&self, filename: &Path) {
        let file = File::create(filename)
            .unwrap_or_else(|_| show_error(format!("cannot create `{}`", filename.display())));
        let file = BufWriter::new(file);
        serde_json::ser::to_writer(file, self)
            .unwrap_or_else(|_| show_error(format!("cannot write to `{}`", filename.display())));
    }
}

fn show_help() {
    println!("{}", CARGO_MIRI_HELP);
}

fn show_version() {
    println!(
        "miri {} ({} {})",
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_SHA_SHORT"),
        env!("VERGEN_COMMIT_DATE")
    );
}

fn show_error(msg: String) -> ! {
    eprintln!("fatal error: {}", msg);
    std::process::exit(1)
}

// Determines whether a `--flag` is present.
fn has_arg_flag(name: &str) -> bool {
    let mut args = std::env::args().take_while(|val| val != "--");
    args.any(|val| val == name)
}

/// Yields all values of command line flag `name`.
struct ArgFlagValueIter<'a> {
    args: TakeWhile<env::Args, fn(&String) -> bool>,
    name: &'a str,
}

impl<'a> ArgFlagValueIter<'a> {
    fn new(name: &'a str) -> Self {
        Self {
            // Stop searching at `--`.
            args: env::args().take_while(|val| val != "--"),
            name,
        }
    }
}

impl Iterator for ArgFlagValueIter<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let arg = self.args.next()?;
            if !arg.starts_with(self.name) {
                continue;
            }
            // Strip leading `name`.
            let suffix = &arg[self.name.len()..];
            if suffix.is_empty() {
                // This argument is exactly `name`; the next one is the value.
                return self.args.next();
            } else if suffix.starts_with('=') {
                // This argument is `name=value`; get the value.
                // Strip leading `=`.
                return Some(suffix[1..].to_owned());
            }
        }
    }
}

/// Gets the value of a `--flag`.
fn get_arg_flag_value(name: &str) -> Option<String> {
    ArgFlagValueIter::new(name).next()
}

/// Returns the path to the `miri` binary
fn find_miri() -> PathBuf {
    if let Some(path) = env::var_os("MIRI") {
        return path.into();
    }
    let mut path = std::env::current_exe().expect("current executable path invalid");
    path.set_file_name("miri");
    path
}

fn miri() -> Command {
    Command::new(find_miri())
}

fn version_info() -> VersionMeta {
    VersionMeta::for_command(miri()).expect("failed to determine underlying rustc version of Miri")
}

fn cargo() -> Command {
    Command::new(env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
}

fn xargo_check() -> Command {
    Command::new(env::var_os("XARGO_CHECK").unwrap_or_else(|| OsString::from("xargo-check")))
}

/// Execute the command. If it fails, fail this process with the same exit code.
/// Otherwise, continue.
fn exec(mut cmd: Command) {
    let exit_status = cmd.status().expect("failed to run command");
    if exit_status.success().not() {
        std::process::exit(exit_status.code().unwrap_or(-1))
    }
}

/// Execute the command and pipe `input` into its stdin.
/// If it fails, fail this process with the same exit code.
/// Otherwise, continue.
fn exec_with_pipe(mut cmd: Command, input: &[u8]) {
    cmd.stdin(std::process::Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn process");
    {
        let stdin = child.stdin.as_mut().expect("failed to open stdin");
        stdin.write_all(input).expect("failed to write out test source");
    }
    let exit_status = child.wait().expect("failed to run command");
    if exit_status.success().not() {
        std::process::exit(exit_status.code().unwrap_or(-1))
    }
}

fn xargo_version() -> Option<(u32, u32, u32)> {
    let out = xargo_check().arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    // Parse output. The first line looks like "xargo 0.3.12 (b004f1c 2018-12-13)".
    let line = out
        .stderr
        .lines()
        .nth(0)
        .expect("malformed `xargo --version` output: not at least one line")
        .expect("malformed `xargo --version` output: error reading first line");
    let (name, version) = {
        let mut split = line.split(' ');
        (
            split.next().expect("malformed `xargo --version` output: empty"),
            split.next().expect("malformed `xargo --version` output: not at least two words"),
        )
    };
    if name != "xargo" {
        // This is some fork of xargo
        return None;
    }
    let mut version_pieces = version.split('.');
    let major = version_pieces
        .next()
        .expect("malformed `xargo --version` output: not a major version piece")
        .parse()
        .expect("malformed `xargo --version` output: major version is not an integer");
    let minor = version_pieces
        .next()
        .expect("malformed `xargo --version` output: not a minor version piece")
        .parse()
        .expect("malformed `xargo --version` output: minor version is not an integer");
    let patch = version_pieces
        .next()
        .expect("malformed `xargo --version` output: not a patch version piece")
        .parse()
        .expect("malformed `xargo --version` output: patch version is not an integer");
    if !version_pieces.next().is_none() {
        panic!("malformed `xargo --version` output: more than three pieces in version");
    }
    Some((major, minor, patch))
}

fn ask_to_run(mut cmd: Command, ask: bool, text: &str) {
    // Disable interactive prompts in CI (GitHub Actions, Travis, AppVeyor, etc).
    // Azure doesn't set `CI` though (nothing to see here, just Microsoft being Microsoft),
    // so we also check their `TF_BUILD`.
    let is_ci = env::var_os("CI").is_some() || env::var_os("TF_BUILD").is_some();
    if ask && !is_ci {
        let mut buf = String::new();
        print!("I will run `{:?}` to {}. Proceed? [Y/n] ", cmd, text);
        io::stdout().flush().unwrap();
        io::stdin().read_line(&mut buf).unwrap();
        match buf.trim().to_lowercase().as_ref() {
            // Proceed.
            "" | "y" | "yes" => {}
            "n" | "no" => show_error(format!("aborting as per your request")),
            a => show_error(format!("invalid answer `{}`", a)),
        };
    } else {
        println!("Running `{:?}` to {}.", cmd, text);
    }

    if cmd.status().expect(&format!("failed to execute {:?}", cmd)).success().not() {
        show_error(format!("failed to {}", text));
    }
}

/// Performs the setup required to make `cargo miri` work: Getting a custom-built libstd. Then sets
/// `MIRI_SYSROOT`. Skipped if `MIRI_SYSROOT` is already set, in which case we expect the user has
/// done all this already.
fn setup(subcommand: MiriCommand) {
    if std::env::var_os("MIRI_SYSROOT").is_some() {
        if subcommand == MiriCommand::Setup {
            println!("WARNING: MIRI_SYSROOT already set, not doing anything.")
        }
        return;
    }

    // Subcommands other than `setup` will do a setup if necessary, but
    // interactively confirm first.
    let ask_user = subcommand != MiriCommand::Setup;

    // First, we need xargo.
    if xargo_version().map_or(true, |v| v < XARGO_MIN_VERSION) {
        if std::env::var_os("XARGO_CHECK").is_some() {
            // The user manually gave us a xargo binary; don't do anything automatically.
            show_error(format!("xargo is too old; please upgrade to the latest version"))
        }
        let mut cmd = cargo();
        cmd.args(&["install", "xargo"]);
        ask_to_run(cmd, ask_user, "install a recent enough xargo");
    }

    // Determine where the rust sources are located.  `XARGO_RUST_SRC` env var trumps everything.
    let rust_src = match std::env::var_os("XARGO_RUST_SRC") {
        Some(path) => {
            let path = PathBuf::from(path);
            // Make path absolute if possible.
            path.canonicalize().unwrap_or(path)
        }
        None => {
            // Check for `rust-src` rustup component.
            let sysroot = miri()
                .args(&["--print", "sysroot"])
                .output()
                .expect("failed to determine sysroot")
                .stdout;
            let sysroot = std::str::from_utf8(&sysroot).unwrap();
            let sysroot = Path::new(sysroot.trim_end_matches('\n'));
            // Check for `$SYSROOT/lib/rustlib/src/rust/library`; test if that contains `std/Cargo.toml`.
            let rustup_src =
                sysroot.join("lib").join("rustlib").join("src").join("rust").join("library");
            if !rustup_src.join("std").join("Cargo.toml").exists() {
                // Ask the user to install the `rust-src` component, and use that.
                let mut cmd = Command::new("rustup");
                cmd.args(&["component", "add", "rust-src"]);
                ask_to_run(
                    cmd,
                    ask_user,
                    "install the `rust-src` component for the selected toolchain",
                );
            }
            rustup_src
        }
    };
    if !rust_src.exists() {
        show_error(format!("given Rust source directory `{}` does not exist.", rust_src.display()));
    }

    // Next, we need our own libstd. Prepare a xargo project for that purpose.
    // We will do this work in whatever is a good cache dir for this platform.
    let dirs = directories::ProjectDirs::from("org", "rust-lang", "miri").unwrap();
    let dir = dirs.cache_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir).unwrap();
    }
    // The interesting bit: Xargo.toml
    File::create(dir.join("Xargo.toml"))
        .unwrap()
        .write_all(
            br#"
[dependencies.std]
default_features = false
# We support unwinding, so enable that panic runtime.
features = ["panic_unwind", "backtrace"]

[dependencies.test]
"#,
        )
        .unwrap();
    // The boring bits: a dummy project for xargo.
    // FIXME: With xargo-check, can we avoid doing this?
    File::create(dir.join("Cargo.toml"))
        .unwrap()
        .write_all(
            br#"
[package]
name = "miri-xargo"
description = "A dummy project for building libstd with xargo."
version = "0.0.0"

[lib]
path = "lib.rs"
"#,
        )
        .unwrap();
    File::create(dir.join("lib.rs")).unwrap();

    // Determine architectures.
    // We always need to set a target so rustc bootstrap can tell apart host from target crates.
    let host = version_info().host;
    let target = get_arg_flag_value("--target");
    let target = target.as_ref().unwrap_or(&host);
    // Now invoke xargo.
    let mut command = xargo_check();
    command.arg("check").arg("-q");
    command.arg("--target").arg(target);
    command.current_dir(&dir);
    command.env("XARGO_HOME", &dir);
    command.env("XARGO_RUST_SRC", &rust_src);
    // Use Miri as rustc to build a libstd compatible with us (and use the right flags).
    // However, when we are running in bootstrap, we cannot just overwrite `RUSTC`,
    // because we still need bootstrap to distinguish between host and target crates.
    // In that case we overwrite `RUSTC_REAL` instead which determines the rustc used
    // for target crates.
    // We set ourselves (`cargo-miri`) instead of Miri directly to be able to patch the flags
    // for `libpanic_abort` (usually this is done by bootstrap but we have to do it ourselves).
    // The `MIRI_BE_RUSTC` will mean we dispatch to `phase_setup_rustc`.
    let cargo_miri_path = std::env::current_exe().expect("current executable path invalid");
    if env::var_os("RUSTC_STAGE").is_some() {
        command.env("RUSTC_REAL", &cargo_miri_path);
    } else {
        command.env("RUSTC", &cargo_miri_path);
    }
    command.env("MIRI_BE_RUSTC", "1");
    // Make sure there are no other wrappers or flags getting in our way
    // (Cc https://github.com/rust-lang/miri/issues/1421).
    // This is consistent with normal `cargo build` that does not apply `RUSTFLAGS`
    // to the sysroot either.
    command.env_remove("RUSTC_WRAPPER");
    command.env_remove("RUSTFLAGS");
    // Disable debug assertions in the standard library -- Miri is already slow enough.
    // But keep the overflow checks, they are cheap.
    command.env("RUSTFLAGS", "-Cdebug-assertions=off -Coverflow-checks=on");
    // Finally run it!
    if command.status().expect("failed to run xargo").success().not() {
        show_error(format!("failed to run xargo"));
    }

    // That should be it! But we need to figure out where xargo built stuff.
    // Unfortunately, it puts things into a different directory when the
    // architecture matches the host.
    let sysroot = if target == &host { dir.join("HOST") } else { PathBuf::from(dir) };
    std::env::set_var("MIRI_SYSROOT", &sysroot); // pass the env var to the processes we spawn, which will turn it into "--sysroot" flags
    // Figure out what to print.
    let print_sysroot = subcommand == MiriCommand::Setup && has_arg_flag("--print-sysroot"); // whether we just print the sysroot path
    if print_sysroot {
        // Print just the sysroot and nothing else; this way we do not need any escaping.
        println!("{}", sysroot.display());
    } else if subcommand == MiriCommand::Setup {
        println!("A libstd for Miri is now available in `{}`.", sysroot.display());
    }
}

fn phase_setup_rustc(args: env::Args) {
    // Mostly we just forward everything.
    // `MIRI_BE_RUST` is already set.
    let mut cmd = miri();
    cmd.args(args);

    // Patch the panic runtime for `libpanic_abort` (mirroring what bootstrap usually does).
    if get_arg_flag_value("--crate-name").as_deref() == Some("panic_abort") {
        cmd.arg("-C").arg("panic=abort");
    }

    // Run it!
    exec(cmd);
}

fn phase_cargo_miri(mut args: env::Args) {
    // Check for version and help flags even when invoked as `cargo-miri`.
    if has_arg_flag("--help") || has_arg_flag("-h") {
        show_help();
        return;
    }
    if has_arg_flag("--version") || has_arg_flag("-V") {
        show_version();
        return;
    }

    // Require a subcommand before any flags.
    // We cannot know which of those flags take arguments and which do not,
    // so we cannot detect subcommands later.
    let subcommand = match args.next().as_deref() {
        Some("test") => MiriCommand::Test,
        Some("run") => MiriCommand::Run,
        Some("setup") => MiriCommand::Setup,
        // Invalid command.
        _ => show_error(format!("`cargo miri` supports the following subcommands: `run`, `test`, and `setup`.")),
    };
    let verbose = has_arg_flag("-v");

    // We always setup.
    setup(subcommand);

    // Invoke actual cargo for the job, but with different flags.
    // We re-use `cargo test` and `cargo run`, which makes target and binary handling very easy but
    // requires some extra work to make the build check-only (see all the `--emit` hacks below).
    // <https://github.com/rust-lang/miri/pull/1540#issuecomment-693553191> describes an alternative
    // approach that uses `cargo check`, making that part easier but target and binary handling
    // harder.
    let cargo_miri_path = std::env::current_exe().expect("current executable path invalid");
    let cargo_cmd = match subcommand {
        MiriCommand::Test => "test",
        MiriCommand::Run => "run",
        MiriCommand::Setup => return, // `cargo miri setup` stops here.
    };
    let mut cmd = cargo();
    cmd.arg(cargo_cmd);

    // Make sure we know the build target, and cargo does, too.
    // This is needed to make the `CARGO_TARGET_*_RUNNER` env var do something,
    // and it later helps us detect which crates are proc-macro/build-script
    // (host crates) and which crates are needed for the program itself.
    let host = version_info().host;
    let target = get_arg_flag_value("--target");
    let target = if let Some(ref target) = target {
        target
    } else {
        // No target given. Pick default and tell cargo about it.
        cmd.arg("--target");
        cmd.arg(&host);
        &host
    };

    // Forward all further arguments. We do some processing here because we want to
    // detect people still using the old way of passing flags to Miri
    // (`cargo miri -- -Zmiri-foo`).
    while let Some(arg) = args.next() {
        cmd.arg(&arg);
        if arg == "--" {
            // Check if the next argument starts with `-Zmiri`. If yes, we assume
            // this is an old-style invocation.
            if let Some(next_arg) = args.next() {
                if next_arg.starts_with("-Zmiri") || next_arg == "--" {
                    eprintln!(
                        "WARNING: it seems like you are setting Miri's flags in `cargo miri` the old way,\n\
                        i.e., by passing them after the first `--`. This style is deprecated; please set\n\
                        the MIRIFLAGS environment variable instead. `cargo miri run/test` now interprets\n\
                        arguments the exact same way as `cargo run/test`."
                    );
                    // Old-style invocation. Turn these into MIRIFLAGS, if there are any.
                    if next_arg != "--" {
                        let mut miriflags = env::var("MIRIFLAGS").unwrap_or_default();
                        miriflags.push(' ');
                        miriflags.push_str(&next_arg);
                        while let Some(further_arg) = args.next() {
                            if further_arg == "--" {
                                // End of the Miri flags!
                                break;
                            }
                            miriflags.push(' ');
                            miriflags.push_str(&further_arg);
                        }
                        env::set_var("MIRIFLAGS", miriflags);
                    }
                    // Pass the remaining flags to cargo.
                    cmd.args(args);
                    break;
                }
                // Not a Miri argument after all, make sure we pass it to cargo.
                cmd.arg(next_arg);
            }
        }
    }

    // Set `RUSTC_WRAPPER` to ourselves.  Cargo will prepend that binary to its usual invocation,
    // i.e., the first argument is `rustc` -- which is what we use in `main` to distinguish
    // the two codepaths. (That extra argument is why we prefer this over setting `RUSTC`.)
    if env::var_os("RUSTC_WRAPPER").is_some() {
        println!("WARNING: Ignoring `RUSTC_WRAPPER` environment variable, Miri does not support wrapping.");
    }
    cmd.env("RUSTC_WRAPPER", &cargo_miri_path);

    let runner_env_name = |triple: &str| {
        format!("CARGO_TARGET_{}_RUNNER", triple.to_uppercase().replace('-', "_"))
    };
    let host_runner_env_name = runner_env_name(&host);
    let target_runner_env_name = runner_env_name(target);
    // Set the target runner to us, so we can interpret the binaries.
    cmd.env(&target_runner_env_name, &cargo_miri_path);
    // Unit tests of `proc-macro` crates are run on the host, so we set the host runner to
    // us in order to skip them.
    cmd.env(&host_runner_env_name, &cargo_miri_path);

    // Set rustdoc to us as well, so we can make it do nothing (see issue #584).
    cmd.env("RUSTDOC", &cargo_miri_path);

    // Run cargo.
    if verbose {
        eprintln!("[cargo-miri miri] RUSTC_WRAPPER={:?}", cargo_miri_path);
        eprintln!("[cargo-miri miri] {}={:?}", target_runner_env_name, cargo_miri_path);
        if *target != host {
            eprintln!("[cargo-miri miri] {}={:?}", host_runner_env_name, cargo_miri_path);
        }
        eprintln!("[cargo-miri miri] RUSTDOC={:?}", cargo_miri_path);
        eprintln!("[cargo-miri miri] {:?}", cmd);
        cmd.env("MIRI_VERBOSE", ""); // This makes the other phases verbose.
    }
    exec(cmd)
}

fn phase_cargo_rustc(args: env::Args) {
    /// Determines if we are being invoked (as rustc) to build a crate for
    /// the "target" architecture, in contrast to the "host" architecture.
    /// Host crates are for build scripts and proc macros and still need to
    /// be built like normal; target crates need to be built for or interpreted
    /// by Miri.
    ///
    /// Currently, we detect this by checking for "--target=", which is
    /// never set for host crates. This matches what rustc bootstrap does,
    /// which hopefully makes it "reliable enough". This relies on us always
    /// invoking cargo itself with `--target`, which `in_cargo_miri` ensures.
    fn is_target_crate() -> bool {
        get_arg_flag_value("--target").is_some()
    }

    /// Returns whether or not Cargo invoked the wrapper (this binary) to compile
    /// the final, binary crate (either a test for 'cargo test', or a binary for 'cargo run')
    /// Cargo does not give us this information directly, so we need to check
    /// various command-line flags.
    fn is_runnable_crate() -> bool {
        let is_bin = get_arg_flag_value("--crate-type").as_deref().unwrap_or("bin") == "bin";
        let is_test = has_arg_flag("--test");
        is_bin || is_test
    }

    fn out_filename(prefix: &str, suffix: &str) -> PathBuf {
        if let Some(out_dir) = get_arg_flag_value("--out-dir") {
            let mut path = PathBuf::from(out_dir);
            path.push(format!(
                "{}{}{}{}",
                prefix,
                get_arg_flag_value("--crate-name").unwrap(),
                // This is technically a `-C` flag but the prefix seems unique enough...
                // (and cargo passes this before the filename so it should be unique)
                get_arg_flag_value("extra-filename").unwrap_or(String::new()),
                suffix,
            ));
            path
        } else {
            let out_file = get_arg_flag_value("-o").unwrap();
            PathBuf::from(out_file)
        }
    }

    let verbose = std::env::var_os("MIRI_VERBOSE").is_some();
    let target_crate = is_target_crate();
    let print = get_arg_flag_value("--print").is_some(); // whether this is cargo passing `--print` to get some infos

    // rlib and cdylib are just skipped, we cannot interpret them and do not need them
    // for the rest of the build either.
    match get_arg_flag_value("--crate-type").as_deref() {
        Some("rlib") | Some("cdylib") => {
            if verbose {
                eprint!("[cargo-miri rustc] (rlib/cdylib skipped)");
            }
            return;
        }
        _ => {},
    }

    let store_json = |info: &CrateRunInfo| {
        let filename = out_filename("", "");
        if verbose {
            eprintln!("[cargo-miri rustc] writing run info to `{}`", filename.display());
        }
        info.store(&filename);
        // For Windows, do the same thing again with `.exe` appended to the filename.
        // (Need to do this here as cargo moves that "binary" to a different place before running it.)
        info.store(&out_filename("", ".exe"));
    };

    let runnable_crate = !print && is_runnable_crate();

    if runnable_crate && target_crate {
        // This is the binary or test crate that we want to interpret under Miri.
        // But we cannot run it here, as cargo invoked us as a compiler -- our stdin and stdout are not
        // like we want them.
        // Instead of compiling, we write JSON into the output file with all the relevant command-line flags
        // and environment variables; this is used when cargo calls us again in the CARGO_TARGET_RUNNER phase.
        let info = CrateRunInfo::collect(args);
        store_json(&info);

        // Rustdoc expects us to exit with an error code if the test is marked as `compile_fail`,
        // just creating the JSON file is not enough: we need to detect syntax errors,
        // so we need to run Miri with `MIRI_BE_RUSTC` for a check-only build.
        if std::env::var_os("MIRI_CALLED_FROM_RUSTDOC").is_some() {
            let mut cmd = miri();
            let env = if let CrateRunInfo::RunWith(env) = info {
                env
            } else {
                return;
            };

            // use our own sysroot
            if !has_arg_flag("--sysroot") {
                let sysroot = env::var_os("MIRI_SYSROOT")
                    .expect("the wrapper should have set MIRI_SYSROOT");
                cmd.arg("--sysroot").arg(sysroot);
            }
            
            // ensure --emit argument for a check-only build is present
            if let Some(i) = env.args.iter().position(|arg| arg.starts_with("--emit=")) {
                // We need to make sure we're not producing a binary that overwrites the JSON file.
                // rustdoc should only ever pass an --emit=metadata argument for tests marked as `no_run`:
                assert_eq!(env.args[i], "--emit=metadata");
            } else {
                cmd.arg("--emit=dep-info,metadata");
            }

            cmd.args(env.args);
            cmd.env("MIRI_BE_RUSTC", "1");

            if verbose {
                eprintln!("[cargo-miri rustc] captured input:\n{}", std::str::from_utf8(&env.stdin).unwrap());
                eprintln!("[cargo-miri rustc] {:?}", cmd);
            }
            
            exec_with_pipe(cmd, &env.stdin);
        }

        return;
    }

    if runnable_crate && ArgFlagValueIter::new("--extern").any(|krate| krate == "proc_macro") {
        // This is a "runnable" `proc-macro` crate (unit tests). We do not support
        // interpreting that under Miri now, so we write a JSON file to (display a
        // helpful message and) skip it in the runner phase.
        store_json(&CrateRunInfo::SkipProcMacroTest);
        return;
    }

    let mut cmd = miri();
    let mut emit_link_hack = false;
    // Arguments are treated very differently depending on whether this crate is
    // for interpretation by Miri, or for use by a build script / proc macro.
    if !print && target_crate {
        // Forward arguments, but remove "link" from "--emit" to make this a check-only build.
        let emit_flag = "--emit";
        for arg in args {
            if arg.starts_with(emit_flag) {
                // Patch this argument. First, extract its value.
                let val = &arg[emit_flag.len()..];
                assert!(val.starts_with("="), "`cargo` should pass `--emit=X` as one argument");
                let val = &val[1..];
                let mut val: Vec<_> = val.split(',').collect();
                // Now make sure "link" is not in there, but "metadata" is.
                if let Some(i) = val.iter().position(|&s| s == "link") {
                    emit_link_hack = true;
                    val.remove(i);
                    if !val.iter().any(|&s| s == "metadata") {
                        val.push("metadata");
                    }
                }
                cmd.arg(format!("{}={}", emit_flag, val.join(",")));
            } else {
                cmd.arg(arg);
            }
        }

        // Use our custom sysroot.
        let sysroot =
            env::var_os("MIRI_SYSROOT").expect("the wrapper should have set MIRI_SYSROOT");
        cmd.arg("--sysroot");
        cmd.arg(sysroot);
    } else {
        // For host crates or when we are printing, just forward everything.
        cmd.args(args);
    }

    // We want to compile, not interpret. We still use Miri to make sure the compiler version etc
    // are the exact same as what is used for interpretation.
    cmd.env("MIRI_BE_RUSTC", "1");

    // Run it.
    if verbose {
        eprintln!("[cargo-miri rustc] {:?}", cmd);
    }
    exec(cmd);

    // Create a stub .rlib file if "link" was requested by cargo.
    if emit_link_hack {
        // Some platforms prepend "lib", some do not... let's just create both files.
        let filename = out_filename("lib", ".rlib");
        File::create(filename).expect("failed to create rlib file");
        let filename = out_filename("", ".rlib");
        File::create(filename).expect("failed to create rlib file");
    }
}

fn forward_patched_extern_arg(args: &mut impl Iterator<Item = String>, cmd: &mut Command) {
    cmd.arg("--extern"); // always forward flag, but adjust filename:
    let path = args.next().expect("`--extern` should be followed by a filename");
    if let Some(lib) = path.strip_suffix(".rlib") {
        // If this is an rlib, make it an rmeta.
        cmd.arg(format!("{}.rmeta", lib));
    } else {
        // Some other extern file (e.g. a `.so`). Forward unchanged.
        cmd.arg(path);
    }
}

fn phase_cargo_runner(binary: &Path, binary_args: env::Args) {
    let verbose = std::env::var_os("MIRI_VERBOSE").is_some();

    let file = File::open(&binary)
        .unwrap_or_else(|_| show_error(format!("file {:?} not found or `cargo-miri` invoked incorrectly; please only invoke this binary through `cargo miri`", binary)));
    let file = BufReader::new(file);

    let info = serde_json::from_reader(file)
        .unwrap_or_else(|_| show_error(format!("file {:?} contains outdated or invalid JSON; try `cargo clean`", binary)));
    let info = match info {
        CrateRunInfo::RunWith(info) => info,
        CrateRunInfo::SkipProcMacroTest => {
            eprintln!("Running unit tests of `proc-macro` crates is not currently supported by Miri.");
            return;
        }
    };

    let mut cmd = miri();

    // Set missing env vars. We prefer build-time env vars over run-time ones; see
    // <https://github.com/rust-lang/miri/issues/1661> for the kind of issue that fixes.
    for (name, val) in info.env {
        if verbose {
            if let Some(old_val) = env::var_os(&name) {
                if old_val != val {
                    eprintln!("[cargo-miri runner] Overwriting run-time env var {:?}={:?} with build-time value {:?}", name, old_val, val);
                }
            }
        }
        cmd.env(name, val);
    }

    // Forward rustc arguments.
    // We need to patch "--extern" filenames because we forced a check-only
    // build without cargo knowing about that: replace `.rlib` suffix by
    // `.rmeta`.
    // We also need to remove `--error-format` as cargo specifies that to be JSON,
    // but when we run here, cargo does not interpret the JSON any more. `--json`
    // then also nees to be dropped.
    let mut args = info.args.into_iter();
    let extern_flag = "--extern";
    let error_format_flag = "--error-format";
    let json_flag = "--json";
    while let Some(arg) = args.next() {
        if arg == extern_flag {
            forward_patched_extern_arg(&mut args, &mut cmd);
        } else if arg.starts_with(error_format_flag) {
            let suffix = &arg[error_format_flag.len()..];
            assert!(suffix.starts_with('='));
            // Drop this argument.
        } else if arg.starts_with(json_flag) {
            let suffix = &arg[json_flag.len()..];
            assert!(suffix.starts_with('='));
            // Drop this argument.
        } else {
            cmd.arg(arg);
        }
    }
    // Set sysroot.
    let sysroot =
        env::var_os("MIRI_SYSROOT").expect("the wrapper should have set MIRI_SYSROOT");
    cmd.arg("--sysroot");
    cmd.arg(sysroot);
    // Respect `MIRIFLAGS`.
    if let Ok(a) = env::var("MIRIFLAGS") {
        // This code is taken from `RUSTFLAGS` handling in cargo.
        let args = a
            .split(' ')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        cmd.args(args);
    }

    // Then pass binary arguments.
    cmd.arg("--");
    cmd.args(binary_args);

    // Make sure we use the build-time working directory for interpreting Miri/rustc arguments.
    // But then we need to switch to the run-time one, which we instruct Miri do do by setting `MIRI_CWD`.
    cmd.current_dir(info.current_dir);
    cmd.env("MIRI_CWD", env::current_dir().unwrap());

    // Run it.
    if verbose {
        eprintln!("[cargo-miri runner] {:?}", cmd);
    }

    if std::env::var_os("MIRI_CALLED_FROM_RUSTDOC").is_some() {
        exec_with_pipe(cmd, &info.stdin)
    } else {
        exec(cmd)
    }
}

fn phase_cargo_rustdoc(fst_arg: &str, mut args: env::Args) {
    let verbose = std::env::var_os("MIRI_VERBOSE").is_some();

    // phase_cargo_miri sets the RUSTDOC env var to ourselves, so we can't use that here;
    // just default to a straight-forward invocation for now:
    let mut cmd = Command::new(OsString::from("rustdoc"));

    // Because of the way the main function is structured, we have to take the first argument spearately
    // from the rest; to simplify the following argument patching loop, we'll just skip that one.
    // This is fine for now, because cargo will never pass --extern arguments in the first position,
    // but we should defensively assert that this will work.
    let extern_flag = "--extern";
    assert!(fst_arg != extern_flag);
    cmd.arg(fst_arg);
    
    let runtool_flag = "--runtool";
    let mut crossmode = fst_arg == runtool_flag;
    while let Some(arg) = args.next() {
        if arg == extern_flag {
            // Patch --extern arguments to use *.rmeta files, since phase_cargo_rustc only creates stub *.rlib files.
            forward_patched_extern_arg(&mut args, &mut cmd);
        } else if arg == runtool_flag {
            // An existing --runtool flag indicates cargo is running in cross-target mode, which we don't support.
            // Note that this is only passed when cargo is run with the unstable -Zdoctest-xcompile flag;
            // otherwise, we won't be called as rustdoc at all.
            crossmode = true;
            break;
        } else {
            cmd.arg(arg);
        }
    }

    if crossmode {
        eprintln!("Cross-interpreting doc-tests is not currently supported by Miri.");
        return;
    }

    // For each doc-test, rustdoc starts two child processes: first the test is compiled,
    // then the produced executable is invoked. We want to reroute both of these to cargo-miri,
    // such that the first time we'll enter phase_cargo_rustc, and phase_cargo_runner second.
    // 
    // rustdoc invokes the test-builder by forwarding most of its own arguments, which makes
    // it difficult to determine when phase_cargo_rustc should run instead of phase_cargo_rustdoc.
    // Furthermore, the test code is passed via stdin, rather than a temporary file, so we need
    // to let phase_cargo_rustc know to expect that. We'll use this environment variable as a flag:
    cmd.env("MIRI_CALLED_FROM_RUSTDOC", "1");
    
    // The `--test-builder` and `--runtool` arguments are unstable rustdoc features,
    // which are disabled by default. We first need to enable them explicitly:
    cmd.arg("-Z").arg("unstable-options");
    
    let cargo_miri_path = std::env::current_exe().expect("current executable path invalid");
    cmd.arg("--test-builder").arg(&cargo_miri_path); // invoked by forwarding most arguments
    cmd.arg("--runtool").arg(&cargo_miri_path); // invoked with just a single path argument
    
    if verbose {
        eprintln!("[cargo-miri rustdoc] {:?}", cmd);
    }

    exec(cmd)
}

fn main() {
    // Rustc does not support non-UTF-8 arguments so we make no attempt either.
    // (We do support non-UTF-8 environment variables though.)
    let mut args = std::env::args();
    // Skip binary name.
    args.next().unwrap();

    // Dispatch running as part of sysroot compilation.
    if env::var_os("MIRI_BE_RUSTC").is_some() {
        phase_setup_rustc(args);
        return;
    }

    // The way rustdoc invokes rustc is indistuingishable from the way cargo invokes rustdoc
    // by the arguments alone, and we can't take from the args iterator in this case.
    // phase_cargo_rustdoc sets this environment variable to let us disambiguate here
    let invoked_by_rustdoc = env::var_os("MIRI_CALLED_FROM_RUSTDOC").is_some();
    if invoked_by_rustdoc {
        // ...however, we then also see this variable when rustdoc invokes us as the testrunner!
        // The runner is invoked as `$runtool ($runtool-arg)* output_file`;
        // since we don't specify any runtool-args, and rustdoc supplies multiple arguments to
        // the test-builder unconditionally, we can just check the number of remaining arguments:
        if args.len() == 1 {
            let arg = args.next().unwrap();
            let binary = Path::new(&arg);
            if binary.exists() {
                phase_cargo_runner(binary, args);
            } else {
                show_error(format!("`cargo-miri` called with non-existing path argument `{}` in rustdoc mode; please invoke this binary through `cargo miri`", arg));
            }
        } else {
            phase_cargo_rustc(args);
        }

        return;
    }

    // Dispatch to `cargo-miri` phase. There are three phases:
    // - When we are called via `cargo miri`, we run as the frontend and invoke the underlying
    //   cargo. We set RUSTC_WRAPPER and CARGO_TARGET_RUNNER to ourselves.
    // - When we are executed due to RUSTC_WRAPPER, we build crates or store the flags of
    //   binary crates for later interpretation.
    // - When we are executed due to CARGO_TARGET_RUNNER, we start interpretation based on the
    //   flags that were stored earlier.
    // On top of that, we are also called as RUSTDOC, but that is just a stub currently.
    match args.next().as_deref() {
        Some("miri") => phase_cargo_miri(args),
        Some("rustc") => phase_cargo_rustc(args),
        Some(arg) => {
            // We have to distinguish the "runner" and "rustdoc" cases.
            // As runner, the first argument is the binary (a file that should exist, with an absolute path);
            // as rustdoc, the first argument is a flag (`--something`).
            let binary = Path::new(arg);
            if binary.exists() {
                assert!(!arg.starts_with("--")); // not a flag
                phase_cargo_runner(binary, args);
            } else if arg.starts_with("--") {
                phase_cargo_rustdoc(arg, args);
            } else {
                show_error(format!("`cargo-miri` called with unexpected first argument `{}`; please only invoke this binary through `cargo miri`", arg));
            }
        }
        _ => show_error(format!("`cargo-miri` called without first argument; please only invoke this binary through `cargo miri`")),
    }
}
