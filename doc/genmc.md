# **(WIP)** Documentation for Miri-GenMC
[GenMC](https://github.com/MPI-SWS/genmc) is a stateless model checker for exploring concurrent executions of a program.

**NOTE: Currently, no actual GenMC functionality is part of Miri, this is still WIP.**

<!-- FIXME(genmc): add explanation. -->

## Usage
Basic usage:
```shell
MIRIFLAGS="-Zmiri-genmc" cargo miri run
```

<!-- FIXME(genmc): explain options. -->

<!-- FIXME(genmc): explain Miri-GenMC specific functions. -->

## Tips

<!-- FIXME(genmc): add tips for using Miri-GenMC more efficiently. -->

## Limitations

Some or all of these limitations might get removed in the future:

- Borrow tracking is currently incompatible (stacked/tree borrows).
- Only Linux is supported for now.
- No 32-bit platform support.
- No cross-platform interpretation.

<!-- FIXME(genmc): document remaining limitations -->

## Development

GenMC is written in C++, which complicates development a bit.
For Rust-C++ interop, Miri uses [CXX.rs](https://cxx.rs/), and all handling of C++ code is contained in the `genmc-sys` crate (located in the Miri repository root directory: `miri/genmc-sys/`).

Building GenMC requires a compiler with C++23 support.
<!-- FIXME(genmc,llvm): remove once LLVM dependency is no longer required. -->
Currently, building GenMC also requires linking to LLVM, which needs to be installed manually.

The actual code for GenMC is not contained in the Miri repo itself, but in a [separate GenMC repo](https://github.com/MPI-SWS/genmc) (with different maintainers).
Note that this repo is just a mirror repo.
<!-- FIXME(genmc): define how submitting code to GenMC should be handled. -->

<!-- FIXME(genmc): explain development. -->

### Building the GenMC Library
The build script in the `genmc-sys` crate handles locating, downloading, building and linking the GenMC library.

To determine which GenMC repo path will be used, the following steps are taken:
- If the env var `GENMC_SRC_PATH` contains a path to a directory with a GenMC repo, use that path.
- If the path `genmc-sys/genmc` exists, use that path.
- If the path `genmc-sys/downloaded/genmc` exists, try to set the GenMC repo there to the commit we need.
- If the downloaded repo doesn't exist or is missing the commit, the build script will fetch the commit over the network.
  - Note that the build script will *not* access the network if any of the steps previous steps succeeds.

Once we get the path to the repo, the compilation proceeds in two steps:
- Compile GenMC into a library (using cmake).
- Compile the cxx.rs bridge to connect the library to the Rust code.
The first step is where all build settings are made, the relevant ones are then stored in a `config.h` file that can be included in the second compilation step.
