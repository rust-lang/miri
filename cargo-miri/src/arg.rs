//! Utilities for dealing with argument flags

use std::borrow::Cow;
use std::error::Error;
use std::{env, fmt, fs, io};

#[derive(Debug)]
pub enum ArgfileError {
    IO(String, io::Error),
    Utf8(String),
}

impl fmt::Display for ArgfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArgfileError::IO(path, err) =>
                write!(f, "failed to read argument file `{path}`: {err}"),
            ArgfileError::Utf8(path) =>
                write!(f, "failed to read argument file `{path}`: not valid UTF-8"),
        }
    }
}

impl Error for ArgfileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ArgfileError::IO(_, err) => Some(err),
            _ => None,
        }
    }
}

/// Owned command-line arguments providing flag inspection queries.
#[derive(Debug)]
pub struct Args {
    args: Vec<String>,
}

impl Args {
    /// Reads arguments from `std::env::args()`, skipping the binary name.
    pub fn from_env() -> Self {
        let mut args = env::args();
        args.next();
        Self { args: args.collect() }
    }

    pub fn as_slice(&self) -> &[String] {
        &self.args
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.args.iter().map(String::as_str)
    }

    pub fn first(&self) -> Option<&str> {
        self.args.first().map(String::as_str)
    }

    pub fn consume_first(&mut self) -> Option<String> {
        if self.args.is_empty() { None } else { Some(self.args.remove(0)) }
    }

    /// Determines whether a `--flag` is present before `--`.
    pub fn has_arg_flag(&self, name: &str) -> bool {
        self.num_arg_flag(name) > 0
    }

    /// Determines how many times a `--flag` is present before `--`.
    pub fn num_arg_flag(&self, name: &str) -> usize {
        self.args.iter().take_while(|val| *val != "--").filter(|val| *val == name).count()
    }

    /// Yields all values of command line flag `name` before `--`.
    pub fn get_arg_flag_values<'x, 'a>(
        &'x self,
        name: &'a str,
    ) -> impl Iterator<Item = &'x str> + 'a
    where
        'x: 'a,
    {
        ArgFlagValueIter::from_str_iter(self.iter(), name)
    }

    /// Gets the first value of a `--flag` before `--`.
    pub fn get_arg_flag_value<'x>(&'x self, name: &str) -> Option<&'x str> {
        self.get_arg_flag_values(name).next()
    }

    /// Replaces any `@file` arguments with the lines of `file`.
    ///
    /// Matches rustc's default line-based argfile expansion behavior:
    /// - Each line of the file becomes an argument.
    /// - `\n` and `\r\n` line endings are supported.
    /// - Lines are not recursively expanded or split on shell whitespace.
    ///
    /// Note: while this expansion is non-recursive within `Args`, forwarding literal `@...`
    /// arguments verbatim across child process boundaries without downstream re-expansion
    /// requires outgoing argfiles (step 3).
    pub fn expand_argfiles(self) -> Result<Self, ArgfileError> {
        let mut expanded = Vec::with_capacity(self.args.len());
        for arg in self.args {
            if let Some(argfile) = arg.strip_prefix('@') {
                let contents = fs::read_to_string(argfile).map_err(|e| {
                    if e.kind() == io::ErrorKind::InvalidData {
                        ArgfileError::Utf8(argfile.to_string())
                    } else {
                        ArgfileError::IO(argfile.to_string(), e)
                    }
                })?;
                for line in contents.lines() {
                    expanded.push(line.to_string());
                }
            } else {
                expanded.push(arg);
            }
        }
        Ok(Self { args: expanded })
    }
}

impl IntoIterator for Args {
    type Item = String;
    type IntoIter = std::vec::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        self.args.into_iter()
    }
}

/// Yields all values of command line flag `name` as `Ok(arg)`, and all other arguments except
/// the flag as `Err(arg)`. (The flag `name` itself is not yielded at all, only its values are.)
pub struct ArgSplitFlagValue<'a, I> {
    args: Option<I>,
    name: &'a str,
}

impl<'a, I: Iterator> ArgSplitFlagValue<'a, I> {
    fn new(args: I, name: &'a str) -> Self {
        Self { args: Some(args), name }
    }
}

impl<'s, I: Iterator<Item = Cow<'s, str>>> Iterator for ArgSplitFlagValue<'_, I> {
    // If the original iterator was all `Owned`, then we will only ever yield `Owned`
    // (so `into_owned()` is cheap).
    type Item = Result<Cow<'s, str>, Cow<'s, str>>;

    fn next(&mut self) -> Option<Self::Item> {
        let Some(args) = self.args.as_mut() else {
            // We already canceled this iterator.
            return None;
        };
        let arg = args.next()?;
        if arg == "--" {
            // Stop searching at `--`.
            self.args = None;
            // But yield the `--` so that it does not get lost!
            return Some(Err(Cow::Borrowed("--")));
        }
        // These branches cannot be merged if we want to avoid the allocation in the `Borrowed` branch.
        match &arg {
            Cow::Borrowed(arg) =>
                if let Some(suffix) = arg.strip_prefix(self.name) {
                    // Strip leading `name`.
                    if suffix.is_empty() {
                        // This argument is exactly `name`; the next one is the value.
                        return args.next().map(Ok);
                    } else if let Some(suffix) = suffix.strip_prefix('=') {
                        // This argument is `name=value`; get the value.
                        return Some(Ok(Cow::Borrowed(suffix)));
                    }
                },
            Cow::Owned(arg) =>
                if let Some(suffix) = arg.strip_prefix(self.name) {
                    // Strip leading `name`.
                    if suffix.is_empty() {
                        // This argument is exactly `name`; the next one is the value.
                        return args.next().map(Ok);
                    } else if let Some(suffix) = suffix.strip_prefix('=') {
                        // This argument is `name=value`; get the value. We need to do an allocation
                        // here as a `String` cannot be subsliced (what would the lifetime be?).
                        return Some(Ok(Cow::Owned(suffix.to_owned())));
                    }
                },
        }
        Some(Err(arg))
    }
}

impl<'a, I: Iterator<Item = String> + 'a> ArgSplitFlagValue<'a, I> {
    pub fn from_string_iter(
        args: I,
        name: &'a str,
    ) -> impl Iterator<Item = Result<String, String>> + 'a {
        ArgSplitFlagValue::new(args.map(Cow::Owned), name).map(|x| {
            match x {
                Ok(s) => Ok(s.into_owned()),
                Err(s) => Err(s.into_owned()),
            }
        })
    }
}

impl<'x: 'a, 'a, I: Iterator<Item = &'x str> + 'a> ArgSplitFlagValue<'a, I> {
    pub fn from_str_iter(
        args: I,
        name: &'a str,
    ) -> impl Iterator<Item = Result<&'x str, &'x str>> + 'a {
        ArgSplitFlagValue::new(args.map(Cow::Borrowed), name).map(|x| {
            match x {
                Ok(Cow::Borrowed(s)) => Ok(s),
                Err(Cow::Borrowed(s)) => Err(s),
                _ => panic!("iterator converted borrowed to owned"),
            }
        })
    }
}

/// Yields all values of command line flag `name`.
pub struct ArgFlagValueIter;

impl ArgFlagValueIter {
    pub fn from_str_iter<'x: 'a, 'a, I: Iterator<Item = &'x str> + 'a>(
        args: I,
        name: &'a str,
    ) -> impl Iterator<Item = &'x str> + 'a {
        ArgSplitFlagValue::from_str_iter(args, name).filter_map(Result::ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl<'a> FromIterator<&'a str> for Args {
        fn from_iter<T: IntoIterator<Item = &'a str>>(iter: T) -> Self {
            Self { args: iter.into_iter().map(String::from).collect() }
        }
    }

    impl FromIterator<String> for Args {
        fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
            Self { args: iter.into_iter().collect() }
        }
    }

    #[test]
    fn test_flag_queries() {
        let args = Args::from_iter([
            "run",
            "-v",
            "--verbose",
            "-v",
            "--target=x86_64",
            "--target",
            "i686",
            "--",
            "-v",
            "--target",
            "arm",
            "-q",
        ]);

        assert!(args.has_arg_flag("-v"));
        assert!(args.has_arg_flag("--verbose"));
        assert!(!args.has_arg_flag("-q"));
        assert_eq!(args.num_arg_flag("-v"), 2);
        assert_eq!(args.num_arg_flag("--verbose"), 1);
        assert_eq!(args.num_arg_flag("-q"), 0);

        let targets: Vec<_> = args.get_arg_flag_values("--target").collect();
        assert_eq!(targets, vec!["x86_64", "i686"]);
        assert_eq!(args.get_arg_flag_value("--target"), Some("x86_64"));
        assert_eq!(args.get_arg_flag_value("--missing"), None);
        // `iter()` yields all arguments including those after `--`.
        assert!(args.iter().any(|arg| arg == "-q"));
    }

    #[test]
    fn test_extra_filename_prefix() {
        let args = Args::from_iter(["-C", "extra-filename=-suffix", "-o", "output"]);
        assert_eq!(args.get_arg_flag_value("extra-filename"), Some("-suffix"));
        assert_eq!(args.get_arg_flag_value("-o"), Some("output"));
    }

    #[test]
    fn test_consume_first() {
        let mut args = Args::from_iter(["miri", "run", "--flag"]);
        assert_eq!(args.as_slice(), &["miri", "run", "--flag"]);
        assert_eq!(args.first(), Some("miri"));
        assert_eq!(args.consume_first(), Some("miri".into()));
        assert_eq!(args.first(), Some("run"));
        assert_eq!(args.consume_first(), Some("run".into()));
        assert_eq!(args.first(), Some("--flag"));
        assert!(args.has_arg_flag("--flag"));
        assert_eq!(args.consume_first(), Some("--flag".into()));
        assert_eq!(args.first(), None);
        assert_eq!(args.consume_first(), None);
        assert!(args.into_iter().next().is_none());
    }

    #[test]
    fn test_split_flag_value_preserves_trailing_args() {
        let args = Args::from_iter([
            "--target-dir",
            "foo",
            "cmd",
            "--target-dir=bar",
            "--",
            "--target-dir",
            "keep",
        ]);
        let mut iter = args.into_iter();
        let forwarded: Vec<_> = ArgSplitFlagValue::from_string_iter(&mut iter, "--target-dir")
            .filter_map(Result::err)
            .collect();
        assert_eq!(forwarded, vec!["cmd", "--"]);
        let remaining: Vec<_> = iter.collect();
        assert_eq!(remaining, vec!["--target-dir", "keep"]);
    }

    use std::path::{Path, PathBuf};

    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, content: &[u8]) -> Self {
            let mut path = env::temp_dir();
            path.push(format!("miri_argfile_test_{}_{name}", std::process::id()));
            fs::write(&path, content).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn test_expand_argfiles_mixed_and_order() {
        let file1 =
            TempFile::new("file1", b"--target\nx86_64-unknown-linux-gnu\narg with spaces\r\n");
        let empty_file = TempFile::new("empty", b"");
        let file2 = TempFile::new("file2", b"--crate-type=lib\r\n--verbose\n");

        let arg1 = format!("@{}", file1.path().display());
        let arg_empty = format!("@{}", empty_file.path().display());
        let arg2 = format!("@{}", file2.path().display());

        let args = Args::from_iter(["--before", &arg1, "--middle", &arg_empty, &arg2, "--after"]);

        let expanded = args.expand_argfiles().unwrap();

        // Drop the temporary files so they are removed from disk.
        drop(file1);
        drop(empty_file);
        drop(file2);

        // Verify exact contents and order are retained after file removal.
        assert_eq!(
            expanded.as_slice(),
            &[
                "--before",
                "--target",
                "x86_64-unknown-linux-gnu",
                "arg with spaces",
                "--middle",
                "--crate-type=lib",
                "--verbose",
                "--after",
            ]
        );

        // Verify flag queries work on the expanded arguments.
        assert_eq!(expanded.get_arg_flag_value("--target"), Some("x86_64-unknown-linux-gnu"));
        assert_eq!(expanded.get_arg_flag_value("--crate-type"), Some("lib"));
        assert!(expanded.has_arg_flag("--verbose"));
        assert!(expanded.has_arg_flag("--before"));
        assert!(expanded.has_arg_flag("--after"));
    }

    #[test]
    fn test_expand_argfiles_missing_file() {
        let nonexistent = env::temp_dir().join(format!("miri_nonexistent_{}", std::process::id()));
        let args = Args::from_iter([format!("@{}", nonexistent.display())]);
        let err = args.expand_argfiles().unwrap_err();
        match &err {
            ArgfileError::IO(path, _) => {
                assert_eq!(path, &nonexistent.to_string_lossy());
            }
            other => panic!("expected IO error, got: {other:?}"),
        }
        let msg = format!("{err}");
        assert!(msg.contains("failed to read argument file"));
        assert!(msg.contains(&nonexistent.to_string_lossy().to_string()));
    }

    #[test]
    fn test_expand_argfiles_invalid_utf8() {
        let file = TempFile::new("invalid_utf8", b"valid\n\xff\xfe\ninvalid");
        let args = Args::from_iter([format!("@{}", file.path().display())]);
        let err = args.expand_argfiles().unwrap_err();
        match &err {
            ArgfileError::Utf8(path) => {
                assert_eq!(path, &file.path().to_string_lossy());
            }
            other => panic!("expected Utf8 error, got: {other:?}"),
        }
        let msg = format!("{err}");
        assert!(msg.contains("not valid UTF-8"));
    }

    #[test]
    fn test_expand_argfiles_persists_after_file_removed() {
        let expanded = {
            let file =
                TempFile::new("to_delete", b"--target\nx86_64-unknown-linux-gnu\n--verbose\n");
            let args =
                Args::from_iter(["--before", &format!("@{}", file.path().display()), "--after"]);
            args.expand_argfiles().unwrap()
        };
        // The temporary argfile has been deleted here.
        assert_eq!(
            expanded.as_slice(),
            &["--before", "--target", "x86_64-unknown-linux-gnu", "--verbose", "--after"]
        );
        assert_eq!(expanded.get_arg_flag_value("--target"), Some("x86_64-unknown-linux-gnu"));
        assert!(expanded.has_arg_flag("--verbose"));
        assert!(expanded.has_arg_flag("--before"));
        assert!(expanded.has_arg_flag("--after"));
    }
}
