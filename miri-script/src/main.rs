#![allow(clippy::needless_question_mark)]

mod commands;
mod coverage;
mod util;

use std::ops::Range;

use anyhow::{Context, Result, anyhow};
use clap::{Arg, ArgAction, Command as ClapCommand};

#[derive(Clone, Debug)]
pub struct MiriScriptRange(Range<u32>);

fn parse_range(val: &str) -> anyhow::Result<MiriScriptRange> {
    let (from, to) = val
        .split_once("..")
        .ok_or_else(|| anyhow!("invalid format for `--many-seeds`: expected `from..to`"))?;
    let from: u32 = if from.is_empty() {
        0
    } else {
        from.parse().context("invalid `from` in `--many-seeds=from..to")?
    };
    let to: u32 = to.parse().context("invalid `to` in `--many-seeds=from..to")?;
    Ok(MiriScriptRange(from..to))
}

#[derive(Clone, Debug)]
pub enum Command {
    /// Installs the miri driver and cargo-miri.
    /// Sets up the rpath such that the installed binary should work in any
    /// working directory. Note that the binaries are placed in the `miri` toolchain
    /// sysroot, to prevent conflicts with other toolchains.
    Install {
        /// Flags that are passed through to `cargo install`.
        flags: Vec<String>,
    },
    /// Just build miri.
    Build {
        /// Flags that are passed through to `cargo build`.
        flags: Vec<String>,
    },
    /// Just check miri.
    Check {
        /// Flags that are passed through to `cargo check`.
        flags: Vec<String>,
    },
    /// Build miri, set up a sysroot and then run the test suite.
    Test {
        bless: bool,
        /// The cross-interpretation target.
        /// If none then the host is the target.
        target: Option<String>,
        /// Produce coverage report if set.
        coverage: bool,
        /// Flags that are passed through to the test harness.
        flags: Vec<String>,
    },
    /// Build miri, set up a sysroot and then run the driver with the given <flags>.
    /// (Also respects MIRIFLAGS environment variable.)
    Run {
        dep: bool,
        verbose: bool,
        many_seeds: Option<MiriScriptRange>,
        target: Option<String>,
        edition: Option<String>,
        /// Flags that are passed through to `miri`.
        flags: Vec<String>,
    },
    /// Build documentation
    Doc {
        /// Flags that are passed through to `cargo doc`.
        flags: Vec<String>,
    },
    /// Format all sources and tests.
    Fmt {
        /// Flags that are passed through to `rustfmt`.
        flags: Vec<String>,
    },
    /// Runs clippy on all sources.
    Clippy {
        /// Flags that are passed through to `cargo clippy`.
        flags: Vec<String>,
    },
    /// Runs the benchmarks from bench-cargo-miri in hyperfine. hyperfine needs to be installed.
    Bench {
        target: Option<String>,
        /// List of benchmarks to run. By default all benchmarks are run.
        benches: Vec<String>,
    },
    /// Update and activate the rustup toolchain 'miri' to the commit given in the
    /// `rust-version` file.
    /// `rustup-toolchain-install-master` must be installed for this to work. Any extra
    /// flags are passed to `rustup-toolchain-install-master`.
    Toolchain { flags: Vec<String> },
    /// Pull and merge Miri changes from the rustc repo. Defaults to fetching the latest
    /// rustc commit. The fetched commit is stored in the `rust-version` file, so the
    /// next `./miri toolchain` will install the rustc that just got pulled.
    RustcPull { commit: Option<String> },
    /// Push Miri changes back to the rustc repo. This will pull a copy of the rustc
    /// history into the Miri repo, unless you set the RUSTC_GIT env var to an existing
    /// clone of the rustc repo.
    RustcPush { github_user: String, branch: String },
}

impl Command {
    fn add_remainder(&mut self, remainder: Vec<String>) {
        match self {
            Self::Install { flags }
            | Self::Build { flags }
            | Self::Check { flags }
            | Self::Doc { flags }
            | Self::Fmt { flags }
            | Self::Clippy { flags }
            | Self::Run { flags, .. }
            | Self::Toolchain { flags, .. }
            | Self::Test { flags, .. } =>
                if !remainder.is_empty() {
                    flags.push("--".into());
                    flags.extend(remainder);
                },
            Self::Bench { .. } | Self::RustcPull { .. } | Self::RustcPush { .. } => (),
        }
    }
}

fn parse_many_seeds(arg: &str) -> Result<MiriScriptRange, String> {
    parse_range(arg).map_err(|e| e.to_string())
}

fn build_cli() -> ClapCommand {
    ClapCommand::new("miri")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            ClapCommand::new("install").about("Installs the miri driver and cargo-miri.").arg(
                Arg::new("flags")
                    .help("Flags that are passed through to `cargo install`.")
                    .action(ArgAction::Append)
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true),
            ),
        )
        .subcommand(
            ClapCommand::new("build").about("Just build miri.").arg(
                Arg::new("flags")
                    .help("Flags that are passed through to `cargo build`.")
                    .action(ArgAction::Append)
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true),
            ),
        )
        .subcommand(
            ClapCommand::new("check").about("Just check miri.").arg(
                Arg::new("flags")
                    .help("Flags that are passed through to `cargo check`.")
                    .action(ArgAction::Append)
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true),
            ),
        )
        .subcommand(
            ClapCommand::new("test")
                .about("Build miri, set up a sysroot and then run the test suite.")
                .arg(Arg::new("bless").long("bless").action(ArgAction::SetTrue))
                .arg(
                    Arg::new("target")
                        .long("target")
                        .help("The cross-interpretation target. If none, the host is the target."),
                )
                .arg(
                    Arg::new("coverage")
                        .long("coverage")
                        .help("Produce coverage report if set.")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("flags")
                        .help("Flags that are passed through to the test harness.")
                        .action(ArgAction::Append)
                        .trailing_var_arg(true)
                        .allow_hyphen_values(true),
                ),
        )
        .subcommand(
            ClapCommand::new("run")
                .about("Run the driver with the given flags.")
                .arg(Arg::new("dep").long("dep").action(ArgAction::SetTrue))
                .arg(Arg::new("verbose").long("verbose").short('v').action(ArgAction::SetTrue))
                .arg(
                    Arg::new("many_seeds")
                        .long("many-seeds")
                        .help("Specify a range for many seeds.")
                        .value_parser(parse_many_seeds),
                )
                .arg(Arg::new("target").long("target"))
                .arg(Arg::new("edition").long("edition"))
                .arg(
                    Arg::new("flags")
                        .help("Flags that are passed through to `miri`.")
                        .action(ArgAction::Append)
                        .trailing_var_arg(true)
                        .allow_hyphen_values(true),
                ),
        )
        .subcommand(
            ClapCommand::new("doc").about("Build documentation.").arg(
                Arg::new("flags")
                    .help("Flags that are passed through to `cargo doc`.")
                    .action(ArgAction::Append)
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true),
            ),
        )
        .subcommand(
            ClapCommand::new("fmt").about("Format all sources and tests.").arg(
                Arg::new("flags")
                    .help("Flags that are passed through to `rustfmt`.")
                    .action(ArgAction::Append)
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true),
            ),
        )
        .subcommand(
            ClapCommand::new("clippy").about("Runs clippy on all sources.").arg(
                Arg::new("flags")
                    .help("Flags that are passed through to `cargo clippy`.")
                    .action(ArgAction::Append)
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true),
            ),
        )
        .subcommand(
            ClapCommand::new("bench")
                .about("Runs benchmarks with hyperfine.")
                .arg(Arg::new("target").long("target"))
                .arg(
                    Arg::new("benches")
                        .help("List of benchmarks to run.")
                        .action(ArgAction::Append),
                ),
        )
        .subcommand(
            ClapCommand::new("toolchain")
                .about("Update and activate the rustup toolchain 'miri'.")
                .arg(Arg::new("flags").action(ArgAction::Append).trailing_var_arg(true))
                .allow_hyphen_values(true),
        )
        .subcommand(
            ClapCommand::new("rustc-pull")
                .about("Pull and merge Miri changes from the rustc repo.")
                .arg(Arg::new("commit").help("The commit hash to fetch.")),
        )
        .subcommand(
            ClapCommand::new("rustc-push")
                .about("Push Miri changes back to the rustc repo.")
                .arg(Arg::new("github_user").help("GitHub user for the push."))
                .arg(Arg::new("branch").help("Branch for the push.")),
        )
}

fn main() -> Result<()> {
    let miri_args: Vec<_> =
        std::env::args().take_while(|x| *x != "--").filter(|x| *x != "--").collect();
    let remainder: Vec<_> = std::env::args().skip_while(|x| *x != "--").skip(1).collect();

    let matches = build_cli().get_matches_from(miri_args);

    let mut command = match matches.subcommand() {
        Some(("install", sub_m)) =>
            Command::Install {
                flags: sub_m.get_many::<String>("flags").unwrap_or_default().cloned().collect(),
            },
        Some(("build", sub_m)) =>
            Command::Build {
                flags: sub_m.get_many::<String>("flags").unwrap_or_default().cloned().collect(),
            },
        Some(("check", sub_m)) =>
            Command::Check {
                flags: sub_m.get_many::<String>("flags").unwrap_or_default().cloned().collect(),
            },
        Some(("test", sub_m)) =>
            Command::Test {
                bless: sub_m.get_flag("bless"),
                target: sub_m.get_one::<String>("target").cloned(),
                coverage: sub_m.get_flag("coverage"),
                flags: sub_m.get_many::<String>("flags").unwrap_or_default().cloned().collect(),
            },
        Some(("run", sub_m)) =>
            Command::Run {
                dep: sub_m.get_flag("dep"),
                verbose: sub_m.get_flag("verbose"),
                many_seeds: sub_m.get_one::<MiriScriptRange>("many_seeds").cloned(),
                target: sub_m.get_one::<String>("target").cloned(),
                edition: sub_m.get_one::<String>("edition").cloned(),
                flags: sub_m.get_many::<String>("flags").unwrap_or_default().cloned().collect(),
            },
        Some(("doc", sub_m)) =>
            Command::Doc {
                flags: sub_m.get_many::<String>("flags").unwrap_or_default().cloned().collect(),
            },
        Some(("fmt", sub_m)) =>
            Command::Fmt {
                flags: sub_m.get_many::<String>("flags").unwrap_or_default().cloned().collect(),
            },
        Some(("clippy", sub_m)) =>
            Command::Clippy {
                flags: sub_m.get_many::<String>("flags").unwrap_or_default().cloned().collect(),
            },
        Some(("bench", sub_m)) =>
            Command::Bench {
                target: sub_m.get_one::<String>("target").cloned(),
                benches: sub_m.get_many::<String>("benches").unwrap_or_default().cloned().collect(),
            },
        Some(("toolchain", sub_m)) =>
            Command::Toolchain {
                flags: sub_m.get_many::<String>("flags").unwrap_or_default().cloned().collect(),
            },
        Some(("rustc-pull", sub_m)) =>
            Command::RustcPull { commit: sub_m.get_one::<String>("commit").cloned() },
        Some(("rustc-push", sub_m)) =>
            Command::RustcPush {
                github_user: sub_m
                    .get_one::<String>("github_user")
                    .context("missing GitHub user")?
                    .to_string(),
                branch: sub_m.get_one::<String>("branch").context("missing branch")?.to_string(),
            },
        _ => unreachable!("Unhandled subcommand"),
    };

    command.add_remainder(remainder);
    command.exec()?;
    Ok(())
}
