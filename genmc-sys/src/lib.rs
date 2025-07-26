use std::str::FromStr;

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
        Self {
            print_random_schedule_seed: false,
            log_level: Default::default(),
            do_symmetry_reduction: false,
        }
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        // FIXME(genmc): set `Warning` by default once changes to GenMC are upstreamed.
        // FIXME(genmc): set `Tip` by default once the GenMC tips are relevant to Miri.
        Self::Error
    }
}

impl FromStr for LogLevel {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "quiet" => LogLevel::Quiet,
            "error" => LogLevel::Error,
            "warning" => LogLevel::Warning,
            "tip" => LogLevel::Tip,
            "debug1" => LogLevel::Debug1Revisits,
            "debug2" => LogLevel::Debug2MemoryAccesses,
            "debug3" => LogLevel::Debug3ReadsFrom,
            _ => return Err("invalid log level"),
        })
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
        pub log_level: LogLevel,
        pub do_symmetry_reduction: bool,
    }

    /// This is mostly equivalent to GenMC `VerbosityLevel`, but the debug log levels are always present (not conditionally compiled based on `ENABLE_GENMC_DEBUG`).
    /// We add this intermediate type to prevent changes to the GenMC log-level from breaking the Miri
    /// build, and to have a stable type for the C++-Rust interface, independent of `ENABLE_GENMC_DEBUG`.
    #[derive(Debug)]
    enum LogLevel {
        /// Disable *all* logging (including error messages on a crash).
        Quiet,
        /// Log errors.
        Error,
        /// Log errors and warnings.
        Warning,
        /// Log errors, warnings and tips.
        Tip,
        /// Debug print considered revisits.
        /// Downgraded to `Tip` if `GENMC_DEBUG` is not enabled.
        Debug1Revisits,
        /// Print the execution graph after every memory access.
        /// Also includes the previous debug log level.
        /// Downgraded to `Tip` if `GENMC_DEBUG` is not enabled.
        Debug2MemoryAccesses,
        /// Print reads-from values considered by GenMC.
        /// Also includes the previous debug log level.
        /// Downgraded to `Tip` if `GENMC_DEBUG` is not enabled.
        Debug3ReadsFrom,
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

    #[derive(Debug, Clone, Copy)]
    struct GenmcScalar {
        value: u64,
        is_init: bool,
    }

    /**** \/ Result & Error types \/ ****/

    #[must_use]
    #[derive(Debug, Clone, Copy)]
    enum ExecutionState {
        Ok,
        Blocked,
        Finished,
    }

    #[must_use]
    #[derive(Debug)]
    struct SchedulingResult {
        exec_state: ExecutionState,
        next_thread: i32,
    }

    #[must_use]
    #[derive(Debug)]
    struct LoadResult {
        /// If there was an error, it will be stored in `error`, otherwise it is `None`.
        error: UniquePtr<CxxString>,
        /// Indicates whether a value was read or not.
        has_value: bool,
        /// The value that was read. Should not be used if `has_value` is `false`.
        read_value: GenmcScalar,
    }

    #[must_use]
    #[derive(Debug)]
    struct StoreResult {
        /// If there was an error, it will be stored in `error`, otherwise it is `None`.
        error: UniquePtr<CxxString>,
        /// `true` if the write should also be reflected in Miri's memory representation.
        isCoMaxWrite: bool,
    }

    #[must_use]
    #[derive(Debug)]
    struct ReadModifyWriteResult {
        /// If there was an error, it will be stored in `error`, otherwise it is `None`.
        error: UniquePtr<CxxString>,
        /// The value that was read by the RMW operation as the left operand.
        old_value: GenmcScalar,
        /// The value that was produced by the RMW operation.
        new_value: GenmcScalar,
        /// `true` if the write should also be reflected in Miri's memory representation.
        isCoMaxWrite: bool,
    }

    #[must_use]
    #[derive(Debug)]
    struct CompareExchangeResult {
        /// If there was an error, it will be stored in `error`, otherwise it is `None`.
        error: UniquePtr<CxxString>,
        /// The value that was read by the compare-exchange.
        old_value: GenmcScalar,
        /// `true` if compare_exchange op was successful.
        is_success: bool,
        /// `true` if the write should also be reflected in Miri's memory representation.
        isCoMaxWrite: bool,
    }

    /**** /\ Result & Error types /\ ****/

    unsafe extern "C++" {
        include!("MiriInterface.hpp");

        // Types for event handling:
        type GenmcScalar;
        type MemOrdering;
        type RMWBinOp;

        // Types for Scheduling queries:
        type ActionKind;

        // Result / Error types:
        type LoadResult;
        type StoreResult;
        type ReadModifyWriteResult;
        type CompareExchangeResult;

        /// Communication layer between Miri/Rust and GenMC/C++:
        type MiriGenMCShim;

        type ExecutionState;
        type SchedulingResult;

        /// Set up everything required for one run of GenMC, either in verification or estimation mode.
        fn createGenmcHandle(config: &GenmcParams) -> UniquePtr<MiriGenMCShim>;
        /// Get the bit mask that GenMC expects for global memory allocations.
        fn getGlobalAllocStaticMask() -> u64;

        /// This function must be called at the start of any execution, before any events are reported to GenMC.
        fn handleExecutionStart(self: Pin<&mut MiriGenMCShim>);
        /// This function must be called at the end of any execution, even if an error was found during the execution.
        fn handleExecutionEnd(self: Pin<&mut MiriGenMCShim>) -> UniquePtr<CxxString>;

        /***** Functions for handling events encountered during program execution. *****/

        /**** Memory access handling ****/
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
        ) -> StoreResult;

        /**** Memory (de)allocation ****/
        fn handleMalloc(
            self: Pin<&mut MiriGenMCShim>,
            thread_id: i32,
            size: u64,
            alignment: u64,
        ) -> u64;
        fn handleFree(self: Pin<&mut MiriGenMCShim>, thread_id: i32, address: u64);

        /**** Thread management ****/
        fn handleThreadCreate(self: Pin<&mut MiriGenMCShim>, thread_id: i32, parent_id: i32);
        fn handleThreadJoin(self: Pin<&mut MiriGenMCShim>, thread_id: i32, child_id: i32);
        fn handleThreadFinish(self: Pin<&mut MiriGenMCShim>, thread_id: i32, ret_val: u64);
        fn handleThreadKill(self: Pin<&mut MiriGenMCShim>, thread_id: i32);

        /***** Exploration related functionality *****/

        /// Ask GenMC which thread should be scheduled next.
        /// Returns -1 if no more threads can/should be scheduled in the current execution.
        /// Returns the id of the thread that should be scheduled next.
        /// NOTE: This is GenMC's thread id, which needs to be mapped back to a Miri `ThreadId` before it can be used.
        fn scheduleNext(
            self: Pin<&mut MiriGenMCShim>,
            curr_thread_id: i32,
            curr_thread_next_instr_kind: ActionKind,
        ) -> SchedulingResult;

        /// Check whether there are more executions to explore.
        /// If there are more executions, this method prepares for the next execution and returns `true`.
        fn isExplorationDone(self: Pin<&mut MiriGenMCShim>) -> bool;

        /**** Result querying functionality. ****/

        // NOTE: We don't want to share the `VerificationResult` type with the Rust side, since it
        // is very large, uses features that CXX.rs doesn't support and may change as GenMC changes.
        // Instead, we only use the result on the C++ side, and only expose these getter function to
        // the Rust side.

        /// Get the number of blocked executions encountered by GenMC (cast into a fixed with integer)
        fn getBlockedExecutionCount(self: &MiriGenMCShim) -> u64;
        /// Get the number of executions explored by GenMC (cast into a fixed with integer)
        fn getExploredExecutionCount(self: &MiriGenMCShim) -> u64;
        /// Get all messages that GenMC produced (errors, warnings).
        fn getResultMessage(self: &MiriGenMCShim) -> UniquePtr<CxxString>;
        /// If an error occurred, return a string describing the error, otherwise, return `nullptr`.
        fn getErrorString(self: &MiriGenMCShim) -> UniquePtr<CxxString>;
    }
}
