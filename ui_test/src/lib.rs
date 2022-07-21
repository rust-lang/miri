#![allow(clippy::enum_variant_names, clippy::useless_format, clippy::too_many_arguments)]

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub use color_eyre;
use color_eyre::eyre::Result;
use colored::*;
use parser::{ErrorMatch, Pattern};
use regex::Regex;
use rustc_stderr::{Level, Message};

use crate::dependencies::build_dependencies;
use crate::parser::{Comments, Condition};

mod dependencies;
mod diff;
mod parser;
mod rustc_stderr;
#[cfg(test)]
mod tests;

#[derive(Debug)]
pub struct Config {
    /// Arguments passed to the binary that is executed.
    pub args: Vec<OsString>,
    /// `None` to run on the host, otherwise a target triple
    pub target: Option<String>,
    /// Filters applied to stderr output before processing it
    pub stderr_filters: Filter,
    /// Filters applied to stdout output before processing it
    pub stdout_filters: Filter,
    /// The folder in which to start searching for .rs files
    pub root_dir: PathBuf,
    pub mode: Mode,
    pub program: PathBuf,
    pub output_conflict_handling: OutputConflictHandling,
    /// Only run tests with one of these strings in their path/name
    pub path_filter: Vec<String>,
    /// Path to a `Cargo.toml` that describes which dependencies the tests can access.
    pub dependencies_crate_manifest_path: Option<PathBuf>,
    /// Can be used to override what command to run instead of `cargo` to build the
    /// dependencies in `manifest_path`
    pub dependency_builder: Option<DependencyBuilder>,
    /// Print one character per test instead of one line
    pub quiet: bool,
}

#[derive(Debug)]
pub struct DependencyBuilder {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
}

#[derive(Debug)]
pub enum OutputConflictHandling {
    /// The default: emit a diff of the expected/actual output.
    Error,
    /// Ignore mismatches in the stderr/stdout files.
    Ignore,
    /// Instead of erroring if the stderr/stdout differs from the expected
    /// automatically replace it with the found output (after applying filters).
    Bless,
}

pub type Filter = Vec<(Regex, &'static str)>;

pub fn run_tests(mut config: Config) -> Result<()> {
    eprintln!("   Compiler flags: {:?}", config.args);

    // Get the triple with which to run the tests
    let target = config.target.clone().unwrap_or_else(|| config.get_host());

    let dependencies = build_dependencies(&config)?;
    for (name, dependency) in dependencies.dependencies {
        config.args.push("--extern".into());
        let mut dep = OsString::from(name);
        dep.push("=");
        dep.push(dependency);
        config.args.push(dep);
    }
    for import_path in dependencies.import_paths {
        config.args.push("-L".into());
        config.args.push(import_path.into());
    }
    let config = config;

    // A channel for files to process
    let (submit, receive) = crossbeam::channel::unbounded();

    // Some statistics and failure reports.
    let failures = Mutex::new(vec![]);
    let succeeded = AtomicUsize::default();
    let ignored = AtomicUsize::default();
    let filtered = AtomicUsize::default();

    crossbeam::scope(|s| -> Result<()> {
        // Create a thread that is in charge of walking the directory and submitting jobs.
        // It closes the channel when it is done.
        s.spawn(|_| {
            let mut todo = VecDeque::new();
            todo.push_back(config.root_dir.clone());
            while let Some(path) = todo.pop_front() {
                if path.is_dir() {
                    // Enqueue everything inside this directory.
                    // We want it sorted, to have some control over scheduling of slow tests.
                    let mut entries =
                        std::fs::read_dir(path).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
                    entries.sort_by_key(|e| e.file_name());
                    for entry in entries {
                        todo.push_back(entry.path());
                    }
                } else if path.extension().map(|ext| ext == "rs").unwrap_or(false) {
                    // Forward .rs files to the test workers.
                    submit.send(path).unwrap();
                }
            }
            // There will be no more jobs. This signals the workers to quit.
            // (This also ensures `submit` is moved into this closure.)
            drop(submit);
        });

        // A channel for the messages emitted by the individual test threads.
        let (finished_files_sender, finished_files_recv) = crossbeam::channel::unbounded();
        enum TestResult {
            Ok,
            Failed,
            Ignored,
        }

        s.spawn(|_| {
            if config.quiet {
                for (i, (_, result)) in finished_files_recv.into_iter().enumerate() {
                    // Humans start counting at 1
                    let i = i + 1;
                    match result {
                        TestResult::Ok => eprint!("{}", ".".green()),
                        TestResult::Failed => eprint!("{}", "F".red().bold()),
                        TestResult::Ignored => eprint!("{}", "i".yellow()),
                    }
                    if i % 100 == 0 {
                        eprintln!(" {i}");
                    }
                }
            } else {
                for (msg, result) in finished_files_recv {
                    eprint!("{msg} ... ");
                    eprintln!(
                        "{}",
                        match result {
                            TestResult::Ok => "ok".green(),
                            TestResult::Failed => "FAILED".red().bold(),
                            TestResult::Ignored => "ignored (in-test comment)".yellow(),
                        }
                    );
                }
            }
        });

        let mut threads = vec![];

        // Create N worker threads that receive files to test.
        for _ in 0..std::thread::available_parallelism().unwrap().get() {
            let finished_files_sender = finished_files_sender.clone();
            threads.push(s.spawn(|_| -> Result<()> {
                let finished_files_sender = finished_files_sender;
                for path in &receive {
                    if !config.path_filter.is_empty() {
                        let path_display = path.display().to_string();
                        if !config.path_filter.iter().any(|filter| path_display.contains(filter)) {
                            filtered.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    }
                    let comments = Comments::parse_file(&path)?;
                    // Ignore file if only/ignore rules do (not) apply
                    if !test_file_conditions(&comments, &target, &config) {
                        ignored.fetch_add(1, Ordering::Relaxed);
                        finished_files_sender
                            .send((path.display().to_string(), TestResult::Ignored))?;
                        continue;
                    }
                    // Run the test for all revisions
                    for revision in
                        comments.revisions.clone().unwrap_or_else(|| vec![String::new()])
                    {
                        let (m, errors, stderr) =
                            run_test(&path, &config, &target, &revision, &comments);

                        // Using a single `eprintln!` to prevent messages from threads from getting intermingled.
                        let mut msg = format!("{} ", path.display());
                        if !revision.is_empty() {
                            write!(msg, "(revision `{revision}`) ").unwrap();
                        }
                        if errors.is_empty() {
                            finished_files_sender.send((msg, TestResult::Ok))?;
                            succeeded.fetch_add(1, Ordering::Relaxed);
                        } else {
                            finished_files_sender.send((msg, TestResult::Failed))?;
                            failures.lock().unwrap().push((
                                path.clone(),
                                m,
                                revision,
                                errors,
                                stderr,
                            ));
                        }
                    }
                }
                Ok(())
            }));
        }

        for thread in threads {
            thread.join().unwrap()?;
        }
        Ok(())
    })
    .unwrap()?;

    // Print all errors in a single thread to show reliable output
    let failures = failures.into_inner().unwrap();
    let succeeded = succeeded.load(Ordering::Relaxed);
    let ignored = ignored.load(Ordering::Relaxed);
    let filtered = filtered.load(Ordering::Relaxed);
    if !failures.is_empty() {
        for (path, miri, revision, errors, stderr) in &failures {
            eprintln!();
            eprint!("{}", path.display().to_string().underline().bold());
            if !revision.is_empty() {
                eprint!(" (revision `{}`)", revision);
            }
            eprint!(" {}", "FAILED:".red().bold());
            eprintln!();
            eprintln!("command: {:?}", miri);
            eprintln!();
            for error in errors {
                match error {
                    Error::ExitStatus(mode, exit_status) => eprintln!("{mode:?} got {exit_status}"),
                    Error::PatternNotFound { pattern, definition_line } => {
                        match pattern {
                            Pattern::SubString(s) =>
                                eprintln!("substring `{s}` {} in stderr output", "not found".red()),
                            Pattern::Regex(r) =>
                                eprintln!("`/{r}/` does {} stderr output", "not match".red()),
                        }
                        eprintln!(
                            "expected because of pattern here: {}:{definition_line}",
                            path.display().to_string().bold()
                        );
                    }
                    Error::NoPatternsFound => {
                        eprintln!("{}", "no error patterns found in failure test".red());
                    }
                    Error::PatternFoundInPassTest =>
                        eprintln!("{}", "error pattern found in success test".red()),
                    Error::OutputDiffers { path, actual, expected } => {
                        eprintln!("actual output differed from expected {}", path.display());
                        diff::print_diff(expected, actual);
                    }
                    Error::ErrorsWithoutPattern { path: None, msgs } => {
                        eprintln!(
                            "There were {} unmatched diagnostics that occurred outside the testfile and had not pattern",
                            msgs.len(),
                        );
                        for Message { level, message } in msgs {
                            eprintln!("    {level:?}: {message}")
                        }
                    }
                    Error::ErrorsWithoutPattern { path: Some((path, line)), msgs } => {
                        eprintln!(
                            "There were {} unmatched diagnostics at {}:{line}",
                            msgs.len(),
                            path.display()
                        );
                        for Message { level, message } in msgs {
                            eprintln!("    {level:?}: {message}")
                        }
                    }
                }
                eprintln!();
            }
            eprintln!("full stderr:");
            eprintln!("{}", stderr);
            eprintln!();
        }
        eprintln!("{}", "FAILURES:".red().underline().bold());
        for (path, _miri, _revision, _errors, _stderr) in &failures {
            eprintln!("    {}", path.display());
        }
        eprintln!();
        eprintln!(
            "test result: {}. {} tests failed, {} tests passed, {} ignored, {} filtered out",
            "FAIL".red(),
            failures.len().to_string().red().bold(),
            succeeded.to_string().green(),
            ignored.to_string().yellow(),
            filtered.to_string().yellow(),
        );
        std::process::exit(1);
    }
    eprintln!();
    eprintln!(
        "test result: {}. {} tests passed, {} ignored, {} filtered out",
        "ok".green(),
        succeeded.to_string().green(),
        ignored.to_string().yellow(),
        filtered.to_string().yellow(),
    );
    eprintln!();
    Ok(())
}

#[derive(Debug)]
enum Error {
    /// Got an invalid exit status for the given mode.
    ExitStatus(Mode, ExitStatus),
    PatternNotFound {
        pattern: Pattern,
        definition_line: usize,
    },
    /// A ui test checking for failure does not have any failure patterns
    NoPatternsFound,
    /// A ui test checking for success has failure patterns
    PatternFoundInPassTest,
    /// Stderr/Stdout differed from the `.stderr`/`.stdout` file present.
    OutputDiffers {
        path: PathBuf,
        actual: String,
        expected: String,
    },
    ErrorsWithoutPattern {
        msgs: Vec<Message>,
        path: Option<(PathBuf, usize)>,
    },
}

type Errors = Vec<Error>;

fn run_test(
    path: &Path,
    config: &Config,
    target: &str,
    revision: &str,
    comments: &Comments,
) -> (Command, Errors, String) {
    // Run miri
    let mut miri = Command::new(&config.program);
    miri.args(config.args.iter());
    miri.arg(path);
    if !revision.is_empty() {
        miri.arg(format!("--cfg={revision}"));
    }
    miri.arg("--error-format=json");
    for arg in &comments.compile_flags {
        miri.arg(arg);
    }
    miri.envs(comments.env_vars.iter().map(|(k, v)| (k, v)));
    let output = miri.output().expect("could not execute miri");
    let mut errors = config.mode.ok(output.status);
    let stderr = check_test_result(
        path,
        config,
        target,
        revision,
        comments,
        &mut errors,
        &output.stdout,
        &output.stderr,
    );
    (miri, errors, stderr)
}

fn check_test_result(
    path: &Path,
    config: &Config,
    target: &str,
    revision: &str,
    comments: &Comments,
    errors: &mut Errors,
    stdout: &[u8],
    stderr: &[u8],
) -> String {
    // Always remove annotation comments from stderr.
    let diagnostics = rustc_stderr::process(path, stderr);
    let stdout = std::str::from_utf8(stdout).unwrap();
    // Check output files (if any)
    let revised = |extension: &str| {
        if revision.is_empty() {
            extension.to_string()
        } else {
            format!("{}.{}", revision, extension)
        }
    };
    // Check output files against actual output
    check_output(
        &diagnostics.rendered,
        path,
        errors,
        revised("stderr"),
        target,
        &config.stderr_filters,
        config,
        comments,
    );
    check_output(
        stdout,
        path,
        errors,
        revised("stdout"),
        target,
        &config.stdout_filters,
        config,
        comments,
    );
    // Check error annotations in the source against output
    check_annotations(
        diagnostics.messages,
        diagnostics.messages_from_unknown_file_or_line,
        path,
        errors,
        config,
        revision,
        comments,
    );
    diagnostics.rendered
}

fn check_annotations(
    mut messages: Vec<Vec<Message>>,
    mut messages_from_unknown_file_or_line: Vec<Message>,
    path: &Path,
    errors: &mut Errors,
    config: &Config,
    revision: &str,
    comments: &Comments,
) {
    if let Some((ref error_pattern, definition_line)) = comments.error_pattern {
        // first check the diagnostics messages outside of our file. We check this first, so that
        // you can mix in-file annotations with //@error-pattern annotations, even if there is overlap
        // in the messages.
        if let Some(i) = messages_from_unknown_file_or_line
            .iter()
            .position(|msg| error_pattern.matches(&msg.message))
        {
            messages_from_unknown_file_or_line.remove(i);
        } else {
            errors.push(Error::PatternNotFound { pattern: error_pattern.clone(), definition_line });
        }
    }

    // The order on `Level` is such that `Error` is the highest level.
    // We will ensure that *all* diagnostics of level at least `lowest_annotation_level`
    // are matched.
    let mut lowest_annotation_level = Level::Error;
    for &ErrorMatch { ref pattern, revision: ref rev, definition_line, line, level } in
        &comments.error_matches
    {
        if let Some(rev) = rev {
            if rev != revision {
                continue;
            }
        }

        // If we found a diagnostic with a level annotation, make sure that all
        // diagnostics of that level have annotations, even if we don't end up finding a matching diagnostic
        // for this pattern.
        lowest_annotation_level = std::cmp::min(lowest_annotation_level, level);

        if let Some(msgs) = messages.get_mut(line) {
            let found =
                msgs.iter().position(|msg| pattern.matches(&msg.message) && msg.level == level);
            if let Some(found) = found {
                msgs.remove(found);
                continue;
            }
        }

        errors.push(Error::PatternNotFound { pattern: pattern.clone(), definition_line });
    }

    let filter = |msgs: Vec<Message>| -> Vec<_> {
        msgs.into_iter()
            .filter(|msg| {
                msg.level
                    >= comments.require_annotations_for_level.unwrap_or(lowest_annotation_level)
            })
            .collect()
    };

    let messages_from_unknown_file_or_line = filter(messages_from_unknown_file_or_line);
    if !messages_from_unknown_file_or_line.is_empty() {
        errors.push(Error::ErrorsWithoutPattern {
            path: None,
            msgs: messages_from_unknown_file_or_line,
        });
    }

    for (line, msgs) in messages.into_iter().enumerate() {
        let msgs = filter(msgs);
        if !msgs.is_empty() {
            errors
                .push(Error::ErrorsWithoutPattern { path: Some((path.to_path_buf(), line)), msgs });
        }
    }

    match (config.mode, comments.error_pattern.is_some() || !comments.error_matches.is_empty()) {
        (Mode::Pass, true) | (Mode::Panic, true) => errors.push(Error::PatternFoundInPassTest),
        (Mode::Fail, false) => errors.push(Error::NoPatternsFound),
        _ => {}
    }
}

fn check_output(
    output: &str,
    path: &Path,
    errors: &mut Errors,
    kind: String,
    target: &str,
    filters: &Filter,
    config: &Config,
    comments: &Comments,
) {
    let output = normalize(path, output, filters, comments);
    let path = output_path(path, comments, kind, target);
    match config.output_conflict_handling {
        OutputConflictHandling::Bless =>
            if output.is_empty() {
                let _ = std::fs::remove_file(path);
            } else {
                std::fs::write(path, &output).unwrap();
            },
        OutputConflictHandling::Error => {
            let expected_output = std::fs::read_to_string(&path).unwrap_or_default();
            if output != expected_output {
                errors.push(Error::OutputDiffers {
                    path,
                    actual: output,
                    expected: expected_output,
                });
            }
        }
        OutputConflictHandling::Ignore => {}
    }
}

fn output_path(path: &Path, comments: &Comments, kind: String, target: &str) -> PathBuf {
    if comments.stderr_per_bitwidth {
        return path.with_extension(format!("{}bit.{kind}", get_pointer_width(target)));
    }
    path.with_extension(kind)
}

fn test_condition(condition: &Condition, target: &str, config: &Config) -> bool {
    match condition {
        Condition::Bitwidth(bits) => get_pointer_width(target) == *bits,
        Condition::Target(t) => target.contains(t),
        Condition::OnHost => config.target.is_none(),
    }
}

/// Returns whether according to the in-file conditions, this file should be run.
fn test_file_conditions(comments: &Comments, target: &str, config: &Config) -> bool {
    if comments.ignore.iter().any(|c| test_condition(c, target, config)) {
        return false;
    }
    comments.only.iter().all(|c| test_condition(c, target, config))
}

// Taken 1:1 from compiletest-rs
fn get_pointer_width(triple: &str) -> u8 {
    if (triple.contains("64") && !triple.ends_with("gnux32") && !triple.ends_with("gnu_ilp32"))
        || triple.starts_with("s390x")
    {
        64
    } else if triple.starts_with("avr") {
        16
    } else {
        32
    }
}

fn normalize(path: &Path, text: &str, filters: &Filter, comments: &Comments) -> String {
    // Useless paths
    let mut text = text.replace(&path.parent().unwrap().display().to_string(), "$DIR");
    if let Some(lib_path) = option_env!("RUSTC_LIB_PATH") {
        text = text.replace(lib_path, "RUSTLIB");
    }

    for (regex, replacement) in filters.iter() {
        text = regex.replace_all(&text, *replacement).to_string();
    }

    for (from, to) in &comments.normalize_stderr {
        text = from.replace_all(&text, to).to_string();
    }
    text
}

impl Config {
    fn get_host(&self) -> String {
        rustc_version::VersionMeta::for_command(std::process::Command::new(&self.program))
            .expect("failed to parse rustc version info")
            .host
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Mode {
    // The test passes a full execution of the rustc driver
    Pass,
    // The rustc driver panicked
    Panic,
    // The rustc driver emitted an error
    Fail,
}

impl Mode {
    fn ok(self, status: ExitStatus) -> Errors {
        match (status.code().unwrap(), self) {
            (1, Mode::Fail) | (101, Mode::Panic) | (0, Mode::Pass) => vec![],
            _ => vec![Error::ExitStatus(self, status)],
        }
    }
}
