pub use self::ffi::*;

pub mod cxx_extra;

/// Defined in "genmc/src/Support/SAddr.hpp".
/// The first bit of all global addresses must be set to `1`, the rest are the actual address.
/// This means the mask, interpreted as an address, is the lower bound of where the global address space starts.
///
/// FIXME(genmc): rework this if non-64bit support is added to GenMC (the current allocation scheme only allows for 64bit addresses).
/// FIXME(genmc): currently we use `getGlobalAllocStaticMask()` to ensure the constant is consistent between Miri and GenMC,
///   but if https://github.com/dtolnay/cxx/issues/1051 is fixed we could share the constant directly.
pub const GENMC_GLOBAL_ADDRESSES_MASK: u64 = 1 << 63;

/// GenMC thread ids are C++ type `int`, which is equivalent to Rust's `i32` on most platforms.
/// The main thread always has thread id 0.
pub const GENMC_MAIN_THREAD_ID: i32 = 0;

impl GenmcScalar {
    pub const UNINIT: Self = Self { value: 0, extra: 0, is_init: false };
    /// GenMC expects a value for all stores, but we cannot always provide one (e.g., non-atomic writes).
    /// FIXME(genmc): remove this if a permanent fix is ever found.
    pub const DUMMY: Self = Self::from_u64(0xDEADBEEF);

    pub const MUTEX_LOCKED_STATE: Self = Self::from_u64(1);
    pub const MUTEX_UNLOCKED_STATE: Self = Self::from_u64(0);

    pub const fn from_u64(value: u64) -> Self {
        Self { value, extra: 0, is_init: true }
    }
}

impl Default for GenmcParams {
    fn default() -> Self {
        Self {
            print_random_schedule_seed: false,
            quiet: true,
            log_level_trace: false,
            do_symmetry_reduction: false, // TODO GENMC (PERFORMANCE): maybe make this default `true`
            estimation_max: 1000,
        }
    }
}

#[cxx::bridge]
mod ffi {
    /// Parameters that will be given to GenMC for setting up the model checker.
    /// (The fields of this struct are visible to both Rust and C++)
    #[derive(Clone, Debug)]
    struct GenmcParams {
        // pub genmc_seed: u64; // OR: Option<u64>
        pub print_random_schedule_seed: bool,
        pub quiet: bool, // TODO GENMC: maybe make log-level more fine grained
        pub log_level_trace: bool,
        pub do_symmetry_reduction: bool,
        pub estimation_max: u32,
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

    #[derive(Debug)]
    enum RMWBinOp {
        Xchg = 0,
        Add = 1,
        Sub = 2,
        And = 3,
        Nand = 4,
        Or = 5,
        Xor = 6,
        Max = 7,
        Min = 8,
        UMax = 9,
        UMin = 10,
    }

    // TODO GENMC: do these have to be shared with the Rust side?
    #[derive(Debug)]
    enum StoreEventType {
        Normal,
        ReadModifyWrite,
        CompareExchange,
        MutexUnlockWrite,
    }

    #[derive(Debug, Clone, Copy)]
    struct GenmcScalar {
        value: u64,
        extra: u64,
        is_init: bool,
    }

    /**** \/ Result & Error types \/ ****/

    #[must_use]
    #[derive(Debug)]
    struct ReadModifyWriteResult {
        old_value: GenmcScalar,
        new_value: GenmcScalar,
        isCoMaxWrite: bool,
        error: UniquePtr<CxxString>, // TODO GENMC: pass more error info here
    }

    #[must_use]
    #[derive(Debug)]
    struct MutexLockResult {
        is_lock_acquired: bool,
        error: UniquePtr<CxxString>, // TODO GENMC: pass more error info here
    }

    #[must_use]
    #[derive(Debug)]
    struct CompareExchangeResult {
        old_value: GenmcScalar, // TODO GENMC: handle bigger values
        is_success: bool,
        isCoMaxWrite: bool,
        error: UniquePtr<CxxString>, // TODO GENMC: pass more error info here
    }

    #[must_use]
    #[derive(Debug)]
    struct LoadResult {
        is_read_opt: bool,
        read_value: GenmcScalar,     // TODO GENMC: handle bigger values
        error: UniquePtr<CxxString>, // TODO GENMC: pass more error info here
    }

    #[must_use]
    #[derive(Debug)]
    struct StoreResult {
        error: UniquePtr<CxxString>, // TODO GENMC: pass more error info here
        isCoMaxWrite: bool,
    }

    /**** /\ Result & Error types /\ ****/

    unsafe extern "C++" {
        include!("MiriInterface.hpp");

        type MemOrdering;
        type RMWBinOp;
        type StoreEventType;

        // Types for Scheduling queries:
        type ActionKind;

        // Result / Error types:
        type LoadResult;
        type StoreResult;
        type ReadModifyWriteResult;
        type CompareExchangeResult;
        type MutexLockResult;

        type GenmcScalar;

        // type OperatingMode; // Estimation(budget) or Verification

        type MiriGenMCShim;

        fn createGenmcHandle(config: &GenmcParams, do_estimation: bool)
        -> UniquePtr<MiriGenMCShim>;
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
        fn handleReadModifyWrite(
            self: Pin<&mut MiriGenMCShim>,
            thread_id: i32,
            address: u64,
            size: u64,
            load_ordering: MemOrdering,
            store_ordering: MemOrdering,
            rmw_op: RMWBinOp,
            rhs_value: GenmcScalar,
            old_value: GenmcScalar,
        ) -> ReadModifyWriteResult;
        fn handleCompareExchange(
            self: Pin<&mut MiriGenMCShim>,
            thread_id: i32,
            address: u64,
            size: u64,
            expected_value: GenmcScalar,
            new_value: GenmcScalar,
            old_value: GenmcScalar,
            success_load_ordering: MemOrdering,
            success_store_ordering: MemOrdering,
            fail_load_ordering: MemOrdering,
            can_fail_spuriously: bool,
        ) -> CompareExchangeResult;
        fn handleStore(
            self: Pin<&mut MiriGenMCShim>,
            thread_id: i32,
            address: u64,
            size: u64,
            value: GenmcScalar,
            old_value: GenmcScalar,
            memory_ordering: MemOrdering,
            store_event_type: StoreEventType,
        ) -> StoreResult;
        fn handleFence(self: Pin<&mut MiriGenMCShim>, thread_id: i32, memory_ordering: MemOrdering);

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

        /**** Blocking instructions ****/
        fn handleUserBlock(self: Pin<&mut MiriGenMCShim>, thread_id: i32);

        /**** Mutex handling ****/
        fn handleMutexLock(
            self: Pin<&mut MiriGenMCShim>,
            thread_id: i32,
            address: u64,
            size: u64,
        ) -> MutexLockResult;
        fn handleMutexTryLock(
            self: Pin<&mut MiriGenMCShim>,
            thread_id: i32,
            address: u64,
            size: u64,
        ) -> MutexLockResult;
        fn handleMutexUnlock(
            self: Pin<&mut MiriGenMCShim>,
            thread_id: i32,
            address: u64,
            size: u64,
        ) -> StoreResult;

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

        fn printGraph(self: Pin<&mut MiriGenMCShim>);
        fn printEstimationResults(self: &MiriGenMCShim, elapsed_time_sec: f64);
    }
}
