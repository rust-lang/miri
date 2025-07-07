use std::path::{Path, PathBuf};
use std::str::FromStr;

const GENMC_LOCAL_PATH_STR: &str = "./genmc/";

/// Name of the library of the GenMC model checker.
const GENMC_MODEL_CHECKER: &str = "model_checker";

const RUST_CXX_BRIDGE_FILE_PATH: &str = "src/lib.rs";

// FIXME(GenMC, build): decide whether to keep debug enabled or not (without this: calling BUG() ==> UB)
const ENABLE_GENMC_DEBUG: bool = true;

#[cfg(feature = "vendor_genmc")]
mod vendoring {
    use std::path::PathBuf;
    use std::str::FromStr;

    use git2::{Oid, Repository};

    use super::GENMC_LOCAL_PATH_STR;

    pub(crate) const GENMC_GITHUB_URL: &str = "https://github.com/Patrick-6/genmc.git";
    pub(crate) const GENMC_COMMIT: &str = "e362c6f73f3567f972cbefb1323973e7120c9cf6";
    pub(crate) const GENMC_VENDORED_PATH_STR: &str = "./vendored/genmc/";

    pub(crate) fn vendor_genmc() -> PathBuf {
        let Ok(genmc_vendored_path) = PathBuf::from_str(GENMC_VENDORED_PATH_STR);

        let repo = Repository::open(&genmc_vendored_path).unwrap_or_else(|open_err| {
            match Repository::clone(GENMC_GITHUB_URL, &genmc_vendored_path) {
                Ok(repo) => {
                    repo
                }
                Err(clone_err) => {
                    println!("cargo::error=Cannot open GenMC repo at path '{GENMC_LOCAL_PATH_STR}': {open_err:?}");
                    println!("cargo::error=Cannot clone GenMC repo from '{GENMC_GITHUB_URL}': {clone_err:?}");
                    std::process::exit(1);
                }
            }
        });

        // Check if there are any updates:
        let commit = if let Ok(oid) = Oid::from_str(GENMC_COMMIT)
            && let Ok(commit) = repo.find_commit(oid)
        {
            commit
        } else {
            match repo.find_remote("origin") {
                Ok(mut remote) =>
                    match remote.fetch(&[GENMC_COMMIT], None, None) {
                        Ok(_) =>
                            println!(
                                "cargo::warning=Successfully fetched commit '{GENMC_COMMIT:?}'"
                            ),
                        Err(e) => panic!("Failed to fetch from remote: {e}"),
                    },
                Err(e) => println!("cargo::warning=Could not find remote 'origin': {e}"),
            }
            let oid = Oid::from_str(GENMC_COMMIT).unwrap();
            repo.find_commit(oid).unwrap()
        };

        // Set the repo to the correct branch:
        checkout_commit(&repo, GENMC_COMMIT);

        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head_commit.id(), commit.id());
        println!("cargo::warning=Successfully set checked out commit {head_commit:?}");

        genmc_vendored_path
    }

    fn checkout_commit(repo: &Repository, refname: &str) {
        let (object, reference) = repo.revparse_ext(refname).expect("Object not found");

        repo.checkout_tree(&object, None).expect("Failed to checkout");

        match reference {
            // `gref` is an actual reference like branches or tags.
            Some(gref) => repo.set_head(gref.name().unwrap()),
            // This is a commit, not a reference.
            None => repo.set_head_detached(object.id()),
        }
        .expect("Failed to set HEAD");
    }
}

/// Build the Rust-C++ interop library with cxx.rs
fn build_cxx_bridge(genmc_path: &Path) {
    // Paths for include directories:
    let model_checker_include_path = genmc_path.join(GENMC_MODEL_CHECKER).join("include");
    let genmc_common_include_path = genmc_path.join("common").join("include");

    let mut bridge = cxx_build::bridge("src/lib.rs");

    // FIXME(GenMC, build): make sure GenMC uses the same compiler / settings as the cxx_bridge
    // FIXME(GenMC, build): can we use c++23? Does CXX support that? Does rustc CI support that?
    bridge
        .opt_level(2)
        .debug(true) // Same settings that GenMC uses ("-O2 -g")
        .warnings(false) // NOTE: enabling this produces a lot of warnings.
        .std("c++20")
        .include(genmc_common_include_path)
        .include(model_checker_include_path)
        .include("./src_cpp")
        .define("PACKAGE_BUGREPORT", "\"FIXME(GenMC) determine what to do with this!!\"") // FIXME(GenMC): HACK to get stuff to compile (this is normally defined by cmake)
        .file("./src_cpp/MiriInterface.hpp")
        .file("./src_cpp/MiriInterface.cpp");

    // NOTE: It is very important to ensure that this and similar flags are set/unset both here and
    // for the cmake build below, otherwise, certain structs/classes can have different
    // sizes and field offsets in the cxx bridge library compared to the model_checker library.
    // This will lead to data corruption in these fields, which can be hard to debug (fields change values randomly).
    if ENABLE_GENMC_DEBUG {
        bridge.define("ENABLE_GENMC_DEBUG", "1");
    }

    bridge.compile("genmc_interop");

    // Link the Rust-C++ interface library generated by cxx_build:
    println!("cargo::rustc-link-lib=static=genmc_interop");
}

/// Build the GenMC model checker library.
/// Returns the path
fn build_genmc_model_checker(genmc_path: &Path) {
    let cmakelists_path = genmc_path.join("CMakeLists.txt");

    let mut config = cmake::Config::new(cmakelists_path);
    config.profile("RelWithDebInfo"); // FIXME(GenMC,cmake): decide on profile to use
    if ENABLE_GENMC_DEBUG {
        config.define("GENMC_DEBUG", "ON");
    }

    // FIXME(GenMC,HACK): Required for unknown reasons on older cmake (version 3.22.1, works without this with version 3.31.6)
    //              Without this, the files are written into the source directory by the cmake configure step, and then
    //              the build step cannot find these files, because it correctly tries using the `target` directory.
    let out_dir = std::env::var("OUT_DIR").unwrap();
    config.configure_arg(format!("-B {out_dir}/build"));

    // Enable only the components of GenMC that we need:
    config.define("BUILD_LLI", "OFF");
    config.define("BUILD_INSTRUMENTATION", "OFF");
    config.define("BUILD_MODEL_CHECKER", "ON");

    config.build_target(GENMC_MODEL_CHECKER);
    let dst = config.build();

    println!("cargo::rustc-link-search=native={}/build/{GENMC_MODEL_CHECKER}", dst.display());
    println!("cargo::rustc-link-lib=static={GENMC_MODEL_CHECKER}");
}

fn main() {
    // Select between local GenMC repo, or vendoring GenMC from a specific commit.
    let Ok(genmc_local_path) = PathBuf::from_str(GENMC_LOCAL_PATH_STR);
    let genmc_path = if genmc_local_path.exists() {
        genmc_local_path
    } else {
        #[cfg(not(feature = "vendor_genmc"))]
        panic!(
            "GenMC not found in path '{}', and vendoring GenMC is disabled.",
            genmc_local_path.to_string_lossy()
        );

        #[cfg(feature = "vendor_genmc")]
        vendoring::vendor_genmc()
    };

    // FIXME(GenMC, performance): these *should* be able to build in parallel:
    // Build all required components:
    build_cxx_bridge(&genmc_path);
    build_genmc_model_checker(&genmc_path);

    // FIXME(GenMC, build): Cloning the GenMC repo triggers a rebuild on the next build (since the directory changed during the first build)

    // Only rebuild if anything changes:
    println!("cargo::rerun-if-changed={RUST_CXX_BRIDGE_FILE_PATH}");
    println!("cargo::rerun-if-changed=./src_cpp");
    let genmc_src_paths = [genmc_path.join("model_checker"), genmc_path.join("common")];
    for genmc_src_path in genmc_src_paths {
        println!("cargo::rerun-if-changed={}", genmc_src_path.display());
    }
}
