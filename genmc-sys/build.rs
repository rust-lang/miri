use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Path used for development of Miri-GenMC.
/// A GenMC repository in this directory will take precedence over the downloaded GenMC repository.
/// If the `download` feature is disabled, this path must contain a GenMC repository.
const GENMC_LOCAL_PATH: &str = "./genmc/";

/// Name of the library of the GenMC model checker.
const GENMC_MODEL_CHECKER: &str = "model_checker";

/// Path where the `cxx_bridge!` macro is used to define the Rust-C++ interface.
const RUST_CXX_BRIDGE_FILE_PATH: &str = "src/lib.rs";

// FIXME(GenMC, build): decide whether to keep debug enabled or not (without this: calling BUG() ==> UB).
// FIXME(GenMC, build): decide on which profile to use.

/// Enable/disable additional debug checks, prints and options for GenMC.
const ENABLE_GENMC_DEBUG: bool = true;
/// The profile with which to build GenMC.
const GENMC_CMAKE_PROFILE: &str = "RelWithDebInfo";

fn fatal_error() -> ! {
    println!("cargo::error=");
    println!("cargo::error=HINT: For more information on GenMC, check out 'doc/GenMC.md'");
    std::process::exit(1)
}

#[cfg(feature = "download_genmc")]
mod downloading {
    use std::path::{Path, PathBuf};
    use std::str::FromStr;

    use git2::{Commit, Oid, Repository, StatusOptions};

    use super::{GENMC_LOCAL_PATH, fatal_error};

    pub(crate) const GENMC_GITHUB_URL: &str = "https://github.com/Patrick-6/genmc.git";
    pub(crate) const GENMC_COMMIT: &str = "2f503036ae14dc91746bfc292d142f332f31727e";
    pub(crate) const GENMC_DOWNLOAD_PATH: &str = "./downloaded/genmc/";

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
                    println!(
                        "cargo::error=Cloned GenMC repository does not contain required commit '{GENMC_COMMIT}'"
                    );
                    fatal_error();
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
            match repo.find_remote("origin") {
                Ok(mut remote) =>
                    remote.fetch(&[GENMC_COMMIT], None, None).unwrap_or_else(|e| {
                        println!("cargo::error=Failed to fetch from remote: {e}");
                        fatal_error();
                    }),
                Err(e) => {
                    println!("cargo::error=could not load commit from remote repository '{GENMC_GITHUB_URL}'. Error: {e}");
                    fatal_error();
                }
            }
            repo.find_commit(commit_oid)
                .expect("Remote repository should contain expected commit")
        })
    }

    fn clone_remote_repo(genmc_download_path: &PathBuf) -> Repository {
        Repository::clone(GENMC_GITHUB_URL, &genmc_download_path).unwrap_or_else(|e| {
            println!("cargo::error=Cannot clone GenMC repo from '{GENMC_GITHUB_URL}': {e:?}");
            fatal_error();
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

        /// Printing too many files makes reading the error message difficult, so we limit the number.
        const PRINT_LIMIT: usize = 8;

        println!();
        println!(
            "cargo::error=Downloaded GenMC repository at path '{GENMC_DOWNLOAD_PATH}' has been modified:"
        );
        for entry in statuses.iter().take(PRINT_LIMIT) {
            println!(
                "cargo::error=  {} is {:?}",
                entry.path().unwrap_or("unknown"),
                entry.status()
            );
        }
        if statuses.len() > PRINT_LIMIT {
            println!("cargo::error=  ...");
            println!("cargo::error=  [ Total {} modified files ]", statuses.len());
        }

        println!("cargo::error=");
        println!(
            "cargo::error=This repository should only be modified by the 'genmc-sys' build script."
        );
        println!(
            "cargo::error=Please undo any changes made, or delete the '{GENMC_DOWNLOAD_PATH}' directory to have it downloaded again."
        );

        println!("cargo::error=");
        let local_path = Path::new(GENMC_LOCAL_PATH);
        println!(
            "cargo::error=HINT: For local development, place a GenMC repository in the path {:?}.",
            std::path::absolute(local_path).unwrap_or_else(|_| local_path.into())
        );
        fatal_error();
    }
}

/// Build the Rust-C++ interop library with cxx.rs
fn build_cxx_bridge(genmc_path: &Path, genmc_install_dir: &Path) {
    // Paths for include directories:
    let model_checker_include_path = genmc_path.join(GENMC_MODEL_CHECKER).join("include");
    let genmc_common_include_path = genmc_path.join("common").join("include");

    // FIXME(GenMC, build): can we use c++23? Does CXX support that? Does rustc CI support that?
    cxx_build::bridge("src/lib.rs")
        .opt_level(2)
        .debug(true) // Same settings that GenMC uses ("-O2 -g")
        .warnings(false) // NOTE: enabling this produces a lot of warnings.
        .std("c++20")
        .include(genmc_common_include_path) // Required for including GenMC helper files.
        .include(model_checker_include_path) // Required for including GenMC model checker files.
        .include(genmc_install_dir) // Required for including `config.h`.
        .include("./src_cpp")
        .file("./src_cpp/MiriInterface.hpp")
        .file("./src_cpp/MiriInterface.cpp")
        .compile("genmc_interop");

    // Link the Rust-C++ interface library generated by cxx_build:
    println!("cargo::rustc-link-lib=static=genmc_interop");
}

/// Build the GenMC model checker library.
/// Returns the path where cmake installs the model checker library and the config.h file.
fn build_genmc_model_checker(genmc_path: &Path) -> PathBuf {
    let cmakelists_path = genmc_path.join("CMakeLists.txt");

    let mut config = cmake::Config::new(cmakelists_path);
    config.profile(GENMC_CMAKE_PROFILE);
    if ENABLE_GENMC_DEBUG {
        config.define("GENMC_DEBUG", "ON");
    }

    // FIXME(GenMC,HACK): Required for unknown reasons on older cmake (version 3.22.1, works without this with version 3.31.6)
    //              Without this, the files are written into the source directory by the cmake configure step, and then
    //              the build step cannot find these files, because it correctly tries using the `target` directory.
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let genmc_build_path: PathBuf = [&out_dir, "build"].into_iter().collect();
    config.configure_arg(format!("-B {}", genmc_build_path.display()));

    // Enable only the components of GenMC that we need:
    config.define("BUILD_LLI", "OFF");
    config.define("BUILD_INSTRUMENTATION", "OFF");
    config.define("BUILD_MODEL_CHECKER", "ON");

    let cmake_install_dir = config.build();

    // Add the model checker library to be linked and the install directory where it is located:
    println!("cargo::rustc-link-search=native={}", cmake_install_dir.display());
    println!("cargo::rustc-link-lib=static={GENMC_MODEL_CHECKER}");

    cmake_install_dir
}

fn main() {
    // Select between local GenMC repo, or downloading GenMC from a specific commit.
    let Ok(genmc_local_path) = PathBuf::from_str(GENMC_LOCAL_PATH);
    let genmc_path = if genmc_local_path.exists() {
        genmc_local_path
    } else if cfg!(feature = "download_genmc") {
        downloading::download_genmc()
    } else {
        println!(
            "cargo::error=GenMC not found in path '{GENMC_LOCAL_PATH}', and downloading GenMC is disabled."
        );
        fatal_error();
    };

    // Build all required components:
    let genmc_install_dir = build_genmc_model_checker(&genmc_path);
    build_cxx_bridge(&genmc_path, &genmc_install_dir);

    // Only rebuild if anything changes:
    // Note that we don't add the downloaded GenMC repo, since that should never be modified manually.
    // Adding that path here would also trigger an unnecessary rebuild after the repo is cloned (since cargo detects that as a file modification).
    println!("cargo::rerun-if-changed={RUST_CXX_BRIDGE_FILE_PATH}");
    println!("cargo::rerun-if-changed=./src_cpp");
    println!("cargo::rerun-if-changed={GENMC_LOCAL_PATH}");
}
