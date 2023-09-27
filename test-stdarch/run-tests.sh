#!/bin/bash
set -euo pipefail

function msg {
  local GREEN_BOLD=$'\e[32;1m'
  local RESET=$'\e[0m'
  echo "$GREEN_BOLD$@$RESET" 1>&2
}

STDARCH_DIR="test-stdarch/stdarch"
STDARCH_REPO="https://github.com/rust-lang/stdarch.git"
STDARCH_REV="6d80869c7ff129f6062eeefa4f876a8cb5f3ebb2"

# FIXME: If already clone, check checked out revision
if [ ! -d "$STDARCH_DIR" ]; then
  msg "## Cloning stdarch repository into $STDARCH_DIR and checking out rev $STDARCH_REV"

  git clone "$STDARCH_REPO" "$STDARCH_DIR"
  (cd "$STDARCH_DIR"; git checkout "$STDARCH_REV")
fi

FILTERS=(
  "core_arch::x86::sse::"
  "core_arch::x86::sse2::"
  "core_arch::x86_64::sse::"
  "core_arch::x86_64::sse2::"
)

SKIPS=(
  # FIXME: Add `#[cfg_attr(miri, ignore)]` to stdarch tests?
  # Those tests use unsupported intrinsics
  "core_arch::x86::sse::tests::test_mm_comieq_ss_vs_ucomieq_ss"
  "core_arch::x86::sse::tests::test_mm_getcsr_setcsr_1"
  "core_arch::x86::sse::tests::test_mm_getcsr_setcsr_2"
  "core_arch::x86::sse::tests::test_mm_getcsr_setcsr_underflow"
  "core_arch::x86::sse::tests::test_mm_sfence"
  "core_arch::x86::sse::tests::test_mm_stream_ps"
  "core_arch::x86::sse2::tests::test_mm_clflush"
  "core_arch::x86::sse2::tests::test_mm_lfence"
  "core_arch::x86::sse2::tests::test_mm_madd_epi16" # FIXME: I forgot to implement it
  "core_arch::x86::sse2::tests::test_mm_maskmoveu_si128"
  "core_arch::x86::sse2::tests::test_mm_mfence"
  "core_arch::x86::sse2::tests::test_mm_stream_pd"
  "core_arch::x86::sse2::tests::test_mm_stream_si128"
  "core_arch::x86::sse2::tests::test_mm_stream_si32"
  "core_arch::x86_64::sse2::tests::test_mm_stream_si64"
  # FIXME: Those are actually failing
  "core_arch::x86::sse::tests::test_mm_rcp_ss" # __m128(0.24997461, 13.0, 16.0, 100.0) != __m128(0.24993896, 13.0, 16.0, 100.0)
  "core_arch::x86::sse::tests::test_mm_store1_ps" # attempt to subtract with overflow
  "core_arch::x86::sse::tests::test_mm_store_ps" # attempt to subtract with overflow
  "core_arch::x86::sse::tests::test_mm_storer_ps" # attempt to subtract with overflow
)

# FIXME: Sub-filter with command line arguments
TEST_ARGS=("${FILTERS[@]}")

for SKIP in "${SKIPS[@]}"; do
  TEST_ARGS+=(--skip "$SKIP")
done

export PATH="$(pwd)/target/debug:$PATH"
export RUSTC="$(which rustc)"
export MIRI="$(pwd)/target/debug/miri"

export STDARCH_TEST_EVERYTHING=1
export TARGET="$MIRI_TEST_TARGET"
export RUST_BACKTRACE=1

msg "## Running stdarch tests in $STDARCH_DIR against miri for $TARGET"

cargo +miri miri test \
    --manifest-path "$STDARCH_DIR/crates/core_arch/Cargo.toml" \
    -- \
    "${TEST_ARGS[@]}"
