use std::path::{Path, PathBuf};
use std::str::FromStr;

// Build script for running Miri with GenMC.
// Check out doc/GenMC.md for more info.

/// Path used for development of Miri-GenMC.
/// A GenMC repository in this directory (relative to the `genmc-sys` directory) will take precedence over the downloaded GenMC repository.
/// If the `download` feature is disabled, this path must contain a GenMC repository.
const GENMC_LOCAL_PATH: &str = "./genmc/";

/// Path where the downloaded GenMC repository will be stored (relative to the `genmc-sys` directory).
/// Note that this directory is *not* cleaned up automatically by `cargo clean`.
#[cfg(feature = "download_genmc")]
const GENMC_DOWNLOAD_PATH: &str = "./downloaded/genmc/";

/// Name of the library of the GenMC model checker.
const GENMC_MODEL_CHECKER: &str = "genmc_lib";

/// Path where the `cxx_bridge!` macro is used to define the Rust-C++ interface.
const RUST_CXX_BRIDGE_FILE_PATH: &str = "src/lib.rs";

#[cfg(feature = "download_genmc")]
mod downloading {
    use std::path::{Path, PathBuf};
    use std::str::FromStr;

    use git2::{Commit, Oid, Repository, StatusOptions};

    use super::{GENMC_DOWNLOAD_PATH, GENMC_LOCAL_PATH};

    /// The GenMC repository the we get our commit from.
    pub(crate) const GENMC_GITHUB_URL: &str = "https://github.com/Patrick-6/genmc.git";
    /// The GenMC commit we depend on. It must be available on the specified GenMC repository.
    pub(crate) const GENMC_COMMIT: &str = "a3c6cbb3b0be78fbd1edbfe7e4ec76e5003b2e96";

    pub(crate) fn download_genmc() -> PathBuf {
        let Ok(genmc_download_path) = PathBuf::from_str(GENMC_DOWNLOAD_PATH);
        let commit_oid = Oid::from_str(GENMC_COMMIT).expect("Commit should be valid.");

        match Repository::open(&genmc_download_path) {
            Ok(repo) => {
                assert_repo_unmodified(&repo);
                let commit = update_local_repo(&repo, commit_oid);
                checkout_commit(&repo, &commit);
            }
            Err(_) => {
                let repo = clone_remote_repo(&genmc_download_path);
                let Ok(commit) = repo.find_commit(commit_oid) else {
                    panic!(
                        "Cloned GenMC repository does not contain required commit '{GENMC_COMMIT}'"
                    );
                };
                checkout_commit(&repo, &commit);
            }
        };

        genmc_download_path
    }

    // Check if the required commit exists already, otherwise try fetching it.
    fn update_local_repo(repo: &Repository, commit_oid: Oid) -> Commit<'_> {
        repo.find_commit(commit_oid).unwrap_or_else(|_find_error| {
            println!("GenMC repository at path '{GENMC_DOWNLOAD_PATH}' does not contain commit '{GENMC_COMMIT}'.");
            // The commit is not in the checkout. Try `git fetch` and hope that we find the commit then.
            match repo.find_remote("origin") {
                Ok(mut remote) =>
                    remote.fetch(&[GENMC_COMMIT], None, None).expect("Failed to fetch from remote."),
                Err(e) => {
                    panic!("Could not load commit ({GENMC_COMMIT}) from remote repository '{GENMC_GITHUB_URL}'. Error: {e}");
                }
            }
            repo.find_commit(commit_oid)
                .expect("Remote repository should contain expected commit")
        })
    }

    fn clone_remote_repo(genmc_download_path: &PathBuf) -> Repository {
        Repository::clone(GENMC_GITHUB_URL, &genmc_download_path).unwrap_or_else(|e| {
            panic!("Cannot clone GenMC repo from '{GENMC_GITHUB_URL}': {e:?}");
        })
    }

    /// Set the state of the repo to a specific commit
    fn checkout_commit(repo: &Repository, commit: &Commit<'_>) {
        repo.checkout_tree(commit.as_object(), None).expect("Failed to checkout");
        repo.set_head_detached(commit.id()).expect("Failed to set HEAD");
        println!("Successfully set checked out commit {commit:?}");
    }

    /// Check that the downloaded repository is unmodified.
    /// If it is modified, explain that it shouldn't be, and hint at how to do local development with GenMC.
    /// We don't overwrite any changes made to the directory, to prevent data loss.
    fn assert_repo_unmodified(repo: &Repository) {
        let statuses = repo
            .statuses(Some(
                StatusOptions::new()
                    .include_untracked(true)
                    .include_ignored(false)
                    .include_unmodified(false),
            ))
            .expect("should be able to get repository status");
        if statuses.is_empty() {
            return;
        }

        let local_path = Path::new(GENMC_LOCAL_PATH);
        println!(
            "HINT: For local development, place a GenMC repository in the path {:?}.",
            std::path::absolute(local_path).unwrap_or_else(|_| local_path.into())
        );
        panic!(
            "Downloaded GenMC repository at path '{GENMC_DOWNLOAD_PATH}' has been modified. Please undo any changes made, or delete the '{GENMC_DOWNLOAD_PATH}' directory to have it downloaded again."
        );
    }
}

// FIXME(genmc,llvm): Remove once the LLVM dependency of the GenMC model checker is removed.
/// The linked LLVM version is in the generated `config.h`` file, which we parse and use to link to LLVM.
/// Returns c++ flags required for building with/including LLVM.
fn link_to_llvm(config_file: &Path) -> String {
    let file_content = std::fs::read_to_string(&config_file).unwrap_or_else(|err| {
        panic!("GenMC config file ({}) should exist, but got errror {err:?}", config_file.display())
    });
    // Look for line '#define LLVM_VERSION "X.Y.Z"'
    let llvm_version = file_content
        .lines()
        .find_map(|line| {
            if let Some(suffix) = line.strip_prefix("#define LLVM_VERSION")
                && let Some(version_str) = suffix.split('"').nth(1)
                && let Some(major) = version_str.split('.').next()
            {
                // FIXME(genmc,debugging): remove warning print
                println!("cargo::warning=Found llvm version {version_str}");
                return Some(major);
            }
            None
        })
        .expect("Config file should contain LLVM version");

    println!("cargo::rustc-link-lib=dylib=LLVM-{llvm_version}");

    // Get required compile flags for LLVM.
    let llvm_config = format!("llvm-config-{}", llvm_version);
    let output = std::process::Command::new(&llvm_config)
        .arg("--cppflags")
        .output()
        .expect("Failed to run llvm-config");
    if !output.status.success() {
        panic!("llvm-config command failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let cpp_flags = String::from_utf8(output.stdout)
        .expect("llvm-config output should be valid UTF-8")
        .trim()
        .to_string();

    println!("cargo::warning=LLVM cpp_flags: '{cpp_flags}'");
    cpp_flags
}

/// Build the Rust-C++ interop library with cxx.rs
fn build_cxx_bridge(genmc_install_dir: &Path, llvm_cpp_flags: &str) {
    let genmc_include_dir = genmc_install_dir.join("include").join("genmc");

    // FIXME(genmc,debugging): remove this:
    println!("cargo::warning=cpp_flags: {:?}", llvm_cpp_flags.split(" ").collect::<Vec<_>>());

    // FIXME(GenMC, build): can we use c++23? Does CXX support that? Does rustc CI support that?
    cxx_build::bridge("src/lib.rs")
        .flags(llvm_cpp_flags.split(" ")) // FIXME(genmc,llvm): remove once LLVM dependency is removed.
        .opt_level(2)
        .debug(true) // Same settings that GenMC uses ("-O2 -g")
        .warnings(false) // NOTE: enabling this produces a lot of warnings.
        .std("c++20")
        .include(genmc_include_dir)
        .include("./src_cpp")
        .file("./src_cpp/MiriInterface.hpp")
        .file("./src_cpp/MiriInterface.cpp")
        .compile("genmc_interop");

    // Link the Rust-C++ interface library generated by cxx_build:
    println!("cargo::rustc-link-lib=static=genmc_interop");
}

/// Build the GenMC model checker library.
/// Returns the path where cmake installs the model checker library and the config.h file.
/// FIXME(genmc,llvm): Also returns the cpp_flags required to compile with LLVM (remove once LLVM dependency is removed).
fn build_genmc_model_checker(genmc_path: &Path) -> (PathBuf, String) {
    /// The profile with which to build GenMC.
    const GENMC_CMAKE_PROFILE: &str = "RelWithDebInfo";

    // Enable/disable additional debug checks, prints and options for GenMC, based on the Rust profile (debug/release)
    let enable_genmc_debug = matches!(std::env::var("PROFILE").as_deref().unwrap(), "debug");
    // FIXME(genmc,debugging): remove warning print
    println!("cargo::warning=GENMC_DEBUG = {enable_genmc_debug}");

    let cmakelists_path = genmc_path.join("CMakeLists.txt");

    let mut config = cmake::Config::new(cmakelists_path);
    config.profile(GENMC_CMAKE_PROFILE);
    config.define("GENMC_DEBUG", if enable_genmc_debug { "ON" } else { "OFF" });

    // Enable and install the components of GenMC that we need:
    config.define("BUILD_LLI", "OFF"); // No need to build the GenMC executable.
    config.define("BUILD_MODEL_CHECKER", "ON");
    config.define("INSTALL_MODEL_CHECKER", "ON");

    let genmc_install_dir = config.build();

    // Add the model checker library to be linked and tell GenMC where to find it:
    let cmake_lib_dir = genmc_install_dir.join("lib").join("genmc");
    // FIXME(genmc,debugging): remove warning print
    println!("cargo::warning=lib dir: {}", cmake_lib_dir.display());
    println!("cargo::rustc-link-search=native={}", cmake_lib_dir.display());
    println!("cargo::rustc-link-lib=static={GENMC_MODEL_CHECKER}");

    // FIXME(genmc,llvm): Remove once the LLVM dependency of the GenMC model checker is removed.
    let config_file = genmc_install_dir.join("include").join("genmc").join("config.h");
    let llvm_cpp_flags = link_to_llvm(&config_file);

    (genmc_install_dir, llvm_cpp_flags)
}

fn main() {
    // Select which path to use for the GenMC repo:
    let Ok(genmc_local_path) = PathBuf::from_str(GENMC_LOCAL_PATH);
    let genmc_path = if let Ok(genmc_src_path) = std::env::var("GENMC_SRC_PATH") {
        let genmc_src_path =
            PathBuf::from_str(&genmc_src_path).expect("GENMC_SRC_PATH should contain a valid path");
        assert!(
            genmc_src_path.exists(),
            "GENMC_SRC_PATH ({}) does not exist!",
            genmc_src_path.display()
        );
        genmc_src_path
    } else if genmc_local_path.exists() {
        // If the local repository exists, cargo should watch it for changes:
        // FIXME(genmc,cargo): We could always watch this path even if it doesn't (yet) exist, depending on how `https://github.com/rust-lang/cargo/issues/6003` is resolved.
        //                     Adding it here means we don't rebuild if a user creates `GENMC_LOCAL_PATH`, which isn't ideal.
        //                     Cargo currently always rebuilds if a watched directory doesn't exist, so we can only add it if it exists.
        println!("cargo::rerun-if-changed={GENMC_LOCAL_PATH}");
        genmc_local_path
    } else if cfg!(feature = "download_genmc") {
        downloading::download_genmc()
    } else {
        panic!("GenMC not found in path '{GENMC_LOCAL_PATH}', and downloading GenMC is disabled.");
    };

    // Build all required components:
    let (genmc_install_dir, llvm_cpp_flags) = build_genmc_model_checker(&genmc_path);
    build_cxx_bridge(&genmc_install_dir, &llvm_cpp_flags);

    // Only rebuild if anything changes:
    // Note that we don't add the downloaded GenMC repo, since that should never be modified manually.
    // Adding that path here would also trigger an unnecessary rebuild after the repo is cloned (since cargo detects that as a file modification).
    println!("cargo::rerun-if-changed={RUST_CXX_BRIDGE_FILE_PATH}");
    println!("cargo::rerun-if-changed=./src_cpp");
}
