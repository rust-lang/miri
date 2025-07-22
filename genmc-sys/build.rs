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

/// The profile with which to build GenMC.
const GENMC_CMAKE_PROFILE: &str = "RelWithDebInfo";

#[cfg(feature = "download_genmc")]
mod downloading {
    use std::path::{Path, PathBuf};
    use std::str::FromStr;

    use git2::{Commit, Oid, Repository, StatusOptions};

    use super::{GENMC_DOWNLOAD_PATH, GENMC_LOCAL_PATH};

    /// The GenMC repository the we get our commit from.
    pub(crate) const GENMC_GITHUB_URL: &str = "https://github.com/Patrick-6/genmc.git";
    /// The GenMC commit we depend on. It must be available on the specified GenMC repository.
    pub(crate) const GENMC_COMMIT: &str = "f8d41c7d8c7d88e47f71ef6bd7914041a2691aab";

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
/// Returns c++ compiler definitions required for building with/including LLVM, and the include path for LLVM headers.
fn link_to_llvm(config_file: &Path) -> (String, String) {
    /// Search a string for a line matching `//@VARIABLE_NAME: VARIABLE CONTENT`
    fn extract_value<'a>(input: &'a str, name: &str) -> Option<&'a str> {
        input
            .lines()
            .find_map(|line| line.strip_prefix("//@")?.strip_prefix(name)?.strip_prefix(": "))
    }

    let file_content = std::fs::read_to_string(&config_file).unwrap_or_else(|err| {
        panic!("GenMC config file ({}) should exist, but got errror {err:?}", config_file.display())
    });

    let llvm_definitions = extract_value(&file_content, "LLVM_DEFINITIONS")
        .expect("Config file should contain LLVM_DEFINITIONS");
    let llvm_include_dirs = extract_value(&file_content, "LLVM_INCLUDE_DIRS")
        .expect("Config file should contain LLVM_INCLUDE_DIRS");
    let llvm_library_dir = extract_value(&file_content, "LLVM_LIBRARY_DIR")
        .expect("Config file should contain LLVM_LIBRARY_DIR");
    let llvm_config_path = extract_value(&file_content, "LLVM_CONFIG_PATH")
        .expect("Config file should contain LLVM_CONFIG_PATH");

    // Add linker search path.
    let lib_dir = PathBuf::from_str(llvm_library_dir).unwrap();
    println!("cargo::rustc-link-search=native={}", lib_dir.display());

    // Add libraries to link.
    let output = std::process::Command::new(llvm_config_path)
        .arg("--libs") // Print the libraries to link to (space-separated list)
        .output()
        .expect("failed to execute llvm-config");
    let llvm_link_libs =
        String::try_from(output.stdout).expect("llvm-config output should be a valid string");

    for link_lib in llvm_link_libs.trim().split(" ") {
        let link_lib =
            link_lib.strip_prefix("-l").expect("Linker parameter should start with \"-l\"");
        println!("cargo::rustc-link-lib=dylib={link_lib}");
    }

    (llvm_definitions.to_string(), llvm_include_dirs.to_string())
}

/// Build the Rust-C++ interop library with cxx.rs
fn build_cxx_bridge(genmc_install_dir: &Path, llvm_definitions: &str, llvm_include_dirs: &str) {
    let genmc_include_dir = genmc_install_dir.join("include").join("genmc");

    // FIXME(genmc,llvm): remove once LLVM dependency is removed.
    // HACK: We filter out _GNU_SOURCE, since it is already set and produces a warning if set again.
    let definitions =
        llvm_definitions.split(";").filter(|definition| definition != &"-D_GNU_SOURCE");

    // FIXME(GenMC, build): can we use c++23? Does CXX support that? Does rustc CI support that?
    cxx_build::bridge("src/lib.rs")
        .flags(definitions)
        .opt_level(2)
        .debug(true) // Same settings that GenMC uses (default for cmake `RelWithDebInfo`)
        .warnings(false) // NOTE: enabling this produces a lot of warnings.
        .std("c++20")
        .include(genmc_include_dir)
        .include(llvm_include_dirs)
        .include("./src_cpp")
        .file("./src_cpp/MiriInterface.hpp")
        .file("./src_cpp/MiriInterface.cpp")
        .compile("genmc_interop");

    // Link the Rust-C++ interface library generated by cxx_build:
    println!("cargo::rustc-link-lib=static=genmc_interop");
}

/// Build the GenMC model checker library.
/// Returns the path where cmake installs the model checker library and the config.h file.
/// FIXME(genmc,llvm): Also returns the c++ compiler definitions required for building with/including LLVM, and the include path for LLVM headers. (remove once LLVM dependency is removed).
fn build_genmc_model_checker(genmc_path: &Path) -> (PathBuf, String, String) {
    // FIXME(genmc,cargo): Switch to using `CARGO_CFG_DEBUG_ASSERTIONS` once https://github.com/rust-lang/cargo/issues/15760 is completed.
    // Enable/disable additional debug checks, prints and options for GenMC, based on the Rust profile (debug/release)
    let enable_genmc_debug = matches!(std::env::var("PROFILE").as_deref().unwrap(), "debug");

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
    println!("cargo::rustc-link-search=native={}", cmake_lib_dir.display());
    println!("cargo::rustc-link-lib=static={GENMC_MODEL_CHECKER}");

    // FIXME(genmc,llvm): Remove once the LLVM dependency of the GenMC model checker is removed.
    let config_file = genmc_install_dir.join("include").join("genmc").join("config.h");
    let (llvm_definitions, llvm_include_dirs) = link_to_llvm(&config_file);

    (genmc_install_dir, llvm_definitions, llvm_include_dirs)
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
    let (genmc_install_dir, llvm_definitions, llvm_include_dirs) =
        build_genmc_model_checker(&genmc_path);
    build_cxx_bridge(&genmc_install_dir, &llvm_definitions, &llvm_include_dirs);

    // Only rebuild if anything changes:
    // Note that we don't add the downloaded GenMC repo, since that should never be modified manually.
    // Adding that path here would also trigger an unnecessary rebuild after the repo is cloned (since cargo detects that as a file modification).
    println!("cargo::rerun-if-changed={RUST_CXX_BRIDGE_FILE_PATH}");
    println!("cargo::rerun-if-changed=./src_cpp");
}
