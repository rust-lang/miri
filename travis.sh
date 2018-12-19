#!/bin/bash
set -e

# Determine configuration
if [ "$TRAVIS_OS_NAME" == osx ]; then
  export MIRI_SYSROOT_BASE=~/Library/Caches/miri.miri.miri/
  FOREIGN_TARGET=i686-apple-darwin
elif [ "$TRAVIS_OS_NAME" == windows ]; then
  export MIRI_SYSROOT_BASE=~/AppData/Local/miri/miri/cache/
  FOREIGN_TARGET=i686-pc-windows-msvc
else
  export MIRI_SYSROOT_BASE=~/.cache/miri/
  FOREIGN_TARGET=i686-unknown-linux-gnu
fi

echo "Build and install miri"
cargo build --release --all-features --all-targets
cargo install --all-features --force --path .
echo

echo "Get ourselves a MIR-full libstd for the host and a foreign architecture"
cargo miri setup
cargo miri setup --target "$FOREIGN_TARGET"
echo

echo "Test miri with full MIR, on the host and other architectures"
MIRI_SYSROOT="$MIRI_SYSROOT_BASE"/HOST cargo test --release --all-features
MIRI_SYSROOT="$MIRI_SYSROOT_BASE" MIRI_TARGET="$FOREIGN_TARGET" cargo test --release --all-features
echo

echo "Test cargo integration"
(cd test-cargo-miri && MIRI_SYSROOT="$MIRI_SYSROOT_BASE"/HOST ./run-test.py)
echo
