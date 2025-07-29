pub use cxx::UniquePtr;

pub use self::ffi::*;

/// Defined in "genmc/src/Support/SAddr.hpp"
/// FIXME(genmc): `getGlobalAllocStaticMask()` is used to ensure the constant is consistent between Miri and GenMC,
///   but if https://github.com/dtolnay/cxx/issues/1051 is fixed we could share the constant directly.
pub const GENMC_GLOBAL_ADDRESSES_MASK: u64 = 1 << 63;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenmcThreadId(pub i32);

pub const GENMC_MAIN_THREAD_ID: GenmcThreadId = GenmcThreadId(0);

impl GenmcScalar {
    pub const UNINIT: Self = Self { value: 0, is_init: false };
    /// GenMC expects a value for all stores, but we cannot always provide one (e.g., non-atomic writes).
    /// FIXME(genmc): remove this if a permanent fix is ever found.
    pub const DUMMY: Self = Self::from_u64(0xDEADBEEF);

    pub const fn from_u64(value: u64) -> Self {
        Self { value, is_init: true }
    }
}

impl Default for GenmcParams {
    fn default() -> Self {
        Self { print_random_schedule_seed: false, do_symmetry_reduction: false }
    }
}

#[cxx::bridge]
mod ffi {
    /// Parameters that will be given to GenMC for setting up the model checker.
    /// (The fields of this struct are visible to both Rust and C++)
    #[derive(Clone, Debug)]
    struct GenmcParams {
        pub print_random_schedule_seed: bool,
        pub do_symmetry_reduction: bool,
        // FIXME(GenMC): Add remaining parameters.
    }

    #[derive(Debug)]
    enum ActionKind {
        /// Any Mir terminator that's atomic and has load semantics.
        Load,
        /// Anything that's not a `Load`.
        NonLoad,
    }

    #[derive(Debug)]
    enum MemOrdering {
        NotAtomic = 0,
        Relaxed = 1,
        // We skip 2 in case we support consume.
        Acquire = 3,
        Release = 4,
        AcquireRelease = 5,
        SequentiallyConsistent = 6,
    }

    #[derive(Debug, Clone, Copy)]
    struct GenmcScalar {
        value: u64,
        is_init: bool,
    }

    /**** \/ Result & Error types \/ ****/

    // FIXME(genmc): Rework error handling (likely requires changes on the GenMC side).

    #[must_use]
    #[derive(Debug)]
    struct LoadResult {
        is_read_opt: bool,
        read_value: GenmcScalar,
        error: UniquePtr<CxxString>,
    }

    #[must_use]
    #[derive(Debug)]
    struct StoreResult {
        error: UniquePtr<CxxString>,
        isCoMaxWrite: bool,
    }

    /**** /\ Result & Error types /\ ****/

    unsafe extern "C++" {
        include!("MiriInterface.hpp");

        type MemOrdering;

        // Types for Scheduling queries:
        type ActionKind;

        // Result / Error types:
        type LoadResult;
        type StoreResult;

        type GenmcScalar;

        type MiriGenMCShim;

        fn createGenmcHandle(config: &GenmcParams) -> UniquePtr<MiriGenMCShim>;
        fn getGlobalAllocStaticMask() -> u64;

        fn handleExecutionStart(self: Pin<&mut MiriGenMCShim>);
        fn handleExecutionEnd(self: Pin<&mut MiriGenMCShim>) -> UniquePtr<CxxString>;

        fn handleLoad(
            self: Pin<&mut MiriGenMCShim>,
            thread_id: i32,
            address: u64,
            size: u64,
            memory_ordering: MemOrdering,
            old_value: GenmcScalar,
        ) -> LoadResult;
        fn handleStore(
            self: Pin<&mut MiriGenMCShim>,
            thread_id: i32,
            address: u64,
            size: u64,
            value: GenmcScalar,
            old_value: GenmcScalar,
            memory_ordering: MemOrdering,
        ) -> StoreResult;

        fn handleMalloc(
            self: Pin<&mut MiriGenMCShim>,
            thread_id: i32,
            size: u64,
            alignment: u64,
        ) -> u64;
        fn handleFree(self: Pin<&mut MiriGenMCShim>, thread_id: i32, address: u64, size: u64);

        fn handleThreadCreate(self: Pin<&mut MiriGenMCShim>, thread_id: i32, parent_id: i32);
        fn handleThreadJoin(self: Pin<&mut MiriGenMCShim>, thread_id: i32, child_id: i32);
        fn handleThreadFinish(self: Pin<&mut MiriGenMCShim>, thread_id: i32, ret_val: u64);
        fn handleThreadKill(self: Pin<&mut MiriGenMCShim>, thread_id: i32);

        /**** Scheduling ****/
        fn scheduleNext(
            self: Pin<&mut MiriGenMCShim>,
            curr_thread_id: i32,
            curr_thread_next_instr_kind: ActionKind,
        ) -> i64;

        fn getBlockedExecutionCount(self: &MiriGenMCShim) -> u64;
        fn getExploredExecutionCount(self: &MiriGenMCShim) -> u64;

        /// Check whether there are more executions to explore.
        /// If there are more executions, this method prepares for the next execution and returns `true`.
        fn isExplorationDone(self: Pin<&mut MiriGenMCShim>) -> bool;
    }
}
