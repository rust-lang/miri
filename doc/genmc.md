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

The actual code for GenMC is not contained in the Miri repo itself, but in a [separate GenMC repo](https://github.com/MPI-SWS/genmc) (with different maintainers).
Note that this repo is just a mirror repo.
<!-- FIXME(genmc): define how submitting code to GenMC should be handled. -->

<!-- FIXME(genmc): explain development. -->
