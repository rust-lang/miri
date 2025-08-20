use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::Duration;

use genmc_sys::cxx_extra::NonNullUniquePtr;
use genmc_sys::{
    GENMC_GLOBAL_ADDRESSES_MASK, GenmcScalar, MemOrdering, MiriGenMCShim, RMWBinOp,
    createGenmcHandle,
};
use rustc_abi::{Align, Size};
use rustc_const_eval::interpret::{AllocId, InterpCx, InterpResult, interp_ok};
use rustc_middle::{mir, throw_machine_stop, throw_ub_format, throw_unsup_format};
use tracing::info;

use self::global_allocations::{EvalContextExt as _, GlobalAllocationHandler};
use self::helper::{genmc_scalar_to_scalar, scalar_to_genmc_scalar};
use self::mapping::{min_max_to_genmc_rmw_op, to_genmc_rmw_op};
use self::thread_id_map::ThreadIdMap;
use crate::concurrency::genmc::helper::split_access;
use crate::concurrency::genmc::warnings::WarningsCache;
use crate::concurrency::thread::EvalContextExt as _;
use crate::{
    AtomicFenceOrd, AtomicReadOrd, AtomicRwOrd, AtomicWriteOrd, BlockReason, MachineCallback,
    MemoryKind, MiriConfig, MiriInterpCx, MiriMachine, MiriMemoryKind, OpTy, Scalar,
    TerminationInfo, ThreadId, ThreadManager, UnblockKind, VisitProvenance, VisitWith, callback,
};

mod config;
mod global_allocations;
mod helper;
mod mapping;
pub mod miri_genmc;
pub(crate) mod scheduling;
mod thread_id_map;
mod warnings;

pub use genmc_sys::GenmcParams;

pub use self::config::GenmcConfig;

const UNSUPPORTED_ATOMICS_SIZE_MSG: &str =
    "GenMC mode currently does not support atomics larger than 8 bytes.";

#[derive(Clone, Copy, Debug)]
enum ExitType {
    MainThreadFinish,
    ExitCalled,
}

/// The exit status of a program.
/// GenMC must store this if a thread exits while any others can still run.
/// The other thread must also be explored before the program is terminated.
#[derive(Clone, Copy, Debug)]
struct ExitStatus {
    exit_code: i32,
    exit_type: ExitType,
}

impl ExitStatus {
    fn do_leak_check(&self) -> bool {
        matches!(self.exit_type, ExitType::MainThreadFinish)
    }
}

#[derive(Debug, Default)]
/// State that is reset at the start of every execution.
struct PerExecutionState {
    /// Thread id management, such as mapping between Miri `ThreadId` and GenMC's thread ids, or selecting GenMC thread ids.
    thread_id_manager: RefCell<ThreadIdMap>,

    /// A flag to indicate that we should not forward non-atomic accesses to genmc, e.g. because we
    /// are executing an atomic operation.
    allow_data_races: Cell<bool>,

    /// The exit status of the program.
    /// `None` if no thread has called `exit` and the main thread isn't finished yet.
    exit_status: Cell<Option<ExitStatus>>,
}

impl PerExecutionState {
    fn reset(&self) {
        self.allow_data_races.replace(false);
        self.thread_id_manager.borrow_mut().reset();
        self.exit_status.set(None);
    }
}

/// The main interface with GenMC.
/// Each `GenmcCtx` owns one `MiriGenmcShim`, which owns one `GenMCDriver` (the GenMC model checker).
/// For each GenMC run (estimation or verification), a new `GenmcCtx` is created.
///
/// In multithreading, each worker thread has its own `GenmcCtx`, which will have their results combined in the end.
/// FIXME(genmc): implement multithreading.
///
/// Some data is shared across all `GenmcCtx` in the same run, namely data for global allocation handling.
/// Globals must be allocated in a consistent manner, i.e., each global allocation must have the same address in each execution.
///
/// Some state is reset between each execution in the same run.
pub struct GenmcCtx {
    /// Handle to the GenMC model checker.
    handle: RefCell<NonNullUniquePtr<MiriGenMCShim>>,

    /// Keep track of global allocations, to ensure they keep the same address across different executions, even if the order of allocations changes.
    /// The `AllocId` for globals is stable across executions, so we can use it as an identifier.
    global_allocations: Arc<GlobalAllocationHandler>,

    /// Cache for which warnings have already been shown to the user.
    /// FIXME(genmc): like `GlobalAllocationHandler`, there should only be one of these per entire execution, maybe even across estimation and verification.
    warnings_cache: RefCell<WarningsCache>,

    /// State that is reset at the start of every execution.
    exec_state: PerExecutionState,
}

/// GenMC Context creation and administrative / query actions
impl GenmcCtx {
    /// Create a new `GenmcCtx` from a given config.
    pub fn new(miri_config: &MiriConfig, target_usize_max: u64, mode: miri_genmc::Mode) -> Self {
        let genmc_config = miri_config.genmc_config.as_ref().unwrap();
        info!("GenMC: Creating new GenMC Context");

        let handle = createGenmcHandle(&genmc_config.params, mode == miri_genmc::Mode::Estimation);
        let non_null_handle = NonNullUniquePtr::new(handle).expect("GenMC should not return null");
        let non_null_handle = RefCell::new(non_null_handle);
        let global_allocations = Arc::new(GlobalAllocationHandler::new(target_usize_max));
        Self {
            handle: non_null_handle,
            global_allocations,
            warnings_cache: Default::default(),
            exec_state: Default::default(),
        }
    }

    /// Given the time taken for the estimation mode run,
    /// print an estimation for how many executions the entire verification will require and give a total time estimate.
    pub fn print_estimation_result(&self, elapsed_time: Duration) {
        let elapsed_time_sec = elapsed_time.as_secs_f64();
        let mc = self.handle.borrow();
        mc.as_ref().printEstimationResults(elapsed_time_sec);
    }

    /// Get the number of blocked executions encountered by GenMC.
    pub fn get_blocked_execution_count(&self) -> u64 {
        let mc = self.handle.borrow();
        mc.as_ref().getBlockedExecutionCount()
    }

    /// Get the number of explored executions encountered by GenMC.
    pub fn get_explored_execution_count(&self) -> u64 {
        let mc = self.handle.borrow();
        mc.as_ref().getExploredExecutionCount()
    }

    /// Check if GenMC encountered an error that wasn't immediately returned during execution.
    /// Returns a string representation of the error if one occurred.
    pub fn try_get_error(&self) -> Option<String> {
        let mc = self.handle.borrow();
        mc.as_ref().getErrorString().as_ref().map(|error| error.to_string_lossy().to_string())
    }

    /// Check if GenMC encountered an error that wasn't immediately returned during execution.
    /// Returns a string representation of the error if one occurred.
    pub fn get_result_message(&self) -> String {
        let mc = self.handle.borrow();
        mc.as_ref()
            .getResultMessage()
            .as_ref()
            .map(|error| error.to_string_lossy().to_string())
            .expect("there should always be a message")
    }

    /// This function determines if we should continue exploring executions or if we are done.
    ///
    /// In GenMC mode, the input program should be repeatedly executed until this function returns `true` or an error is found.
    pub fn is_exploration_done(&self) -> bool {
        let mut mc = self.handle.borrow_mut();
        mc.as_mut().isExplorationDone()
    }

    /// Select whether data race free actions should be allowed. This function should be used carefully!
    ///
    /// If `true` is passed, allow for data races to happen without triggering an error, until this function is called again with argument `false`.
    /// This allows for racy non-atomic memory accesses to be ignored (GenMC is not informed about them at all).
    ///
    /// Certain operations are not permitted in GenMC mode with data races disabled and will cause a panic, e.g., atomic accesses or asking for scheduling decisions.
    ///
    /// # Panics
    /// If data race free is attempted to be set more than once (i.e., no nesting allowed).
    pub(super) fn set_ongoing_action_data_race_free(&self, enable: bool) {
        info!("GenMC: set_ongoing_action_data_race_free ({enable})");
        let old = self.exec_state.allow_data_races.replace(enable);
        assert_ne!(old, enable, "cannot nest allow_data_races");
    }

    /// Check whether data races are currently allowed (e.g., for loading values for validation which are not actually loaded by the program).
    fn get_alloc_data_races(&self) -> bool {
        self.exec_state.allow_data_races.get()
    }
}

/// GenMC event handling. These methods are used to inform GenMC about events happening in the program, and to handle scheduling decisions.
impl GenmcCtx {
    /// Inform GenMC that a new program execution has started.
    /// This function should be called at the start of every execution.
    pub(crate) fn handle_execution_start(&self) {
        self.exec_state.reset();

        let mut mc = self.handle.borrow_mut();
        mc.as_mut().handleExecutionStart();
    }

    /// Inform GenMC that the program's execution has ended.
    ///
    /// This function must be called even when the execution is blocked
    /// (i.e., it returned a `InterpErrorKind::MachineStop` with error kind `TerminationInfo::GenmcBlockedExecution`).
    pub(crate) fn handle_execution_end<'tcx>(
        &self,
        _ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
    ) -> Result<(), String> {
        let mut mc = self.handle.borrow_mut();
        let result = mc.as_mut().handleExecutionEnd();
        if let Some(msg) = result.as_ref() {
            Err(msg.to_string_lossy().to_string())
        } else {
            Ok(())
        }
    }

    /**** Memory access handling ****/

    //* might fails if there's a race, load might also not read anything (returns None) */
    pub(crate) fn atomic_load<'tcx>(
        &self,
        ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
        address: Size,
        size: Size,
        ordering: AtomicReadOrd,
        // The value that we would get, if we were to do a non-atomic load here.
        old_val: Option<Scalar>,
    ) -> InterpResult<'tcx, Scalar> {
        assert!(!self.get_alloc_data_races(), "atomic load with data race checking disabled.");
        let ordering = ordering.convert();
        let genmc_old_value = if let Some(scalar) = old_val {
            scalar_to_genmc_scalar(ecx, scalar)?
        } else {
            GenmcScalar::UNINIT
        };
        let read_value =
            self.handle_load(&ecx.machine, address, size, ordering, genmc_old_value)?;
        genmc_scalar_to_scalar(ecx, read_value, size)
    }

    pub(crate) fn atomic_store<'tcx>(
        &self,
        ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
        address: Size,
        size: Size,
        value: Scalar,
        // The value that we would get, if we were to do a non-atomic load here.
        old_value: Option<Scalar>,
        ordering: AtomicWriteOrd,
    ) -> InterpResult<'tcx, bool> {
        assert!(!self.get_alloc_data_races(), "atomic store with data race checking disabled.");
        let ordering = ordering.convert();
        let genmc_value = scalar_to_genmc_scalar(ecx, value)?;
        let genmc_old_value = if let Some(scalar) = old_value {
            scalar_to_genmc_scalar(ecx, scalar)?
        } else {
            GenmcScalar::UNINIT
        };
        self.handle_store(&ecx.machine, address, size, genmc_value, genmc_old_value, ordering)
    }

    pub(crate) fn atomic_fence<'tcx>(
        &self,
        machine: &MiriMachine<'tcx>,
        ordering: AtomicFenceOrd,
    ) -> InterpResult<'tcx> {
        assert!(!self.get_alloc_data_races(), "atomic fence with data race checking disabled.");

        let ordering = ordering.convert();

        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let curr_thread = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_genmc_tid(curr_thread);

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        pinned_mc.handleFence(genmc_tid, ordering);

        // TODO GENMC: can this operation ever fail?
        interp_ok(())
    }

    /// Inform GenMC about an atomic read-modify-write operation.
    ///
    /// Returns `(old_val, Option<new_val>)`. `new_val` might not be the latest write in coherence order, which is indicated by `None`.
    pub(crate) fn atomic_rmw_op<'tcx>(
        &self,
        ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
        address: Size,
        size: Size,
        ordering: AtomicRwOrd,
        (rmw_op, not): (mir::BinOp, bool),
        rhs_scalar: Scalar,
        // The value that we would get, if we were to do a non-atomic load here.
        old_value: Scalar,
    ) -> InterpResult<'tcx, (Scalar, Option<Scalar>)> {
        assert!(
            !self.get_alloc_data_races(),
            "atomic read-modify-write operation with data race checking disabled."
        );
        let (load_ordering, store_ordering) = ordering.to_genmc_memory_orderings();
        let genmc_rmw_op = to_genmc_rmw_op(rmw_op, not);
        tracing::info!(
            "GenMC: atomic_rmw_op (op: {rmw_op:?}, not: {not}, genmc_rmw_op: {genmc_rmw_op:?}): rhs value: {rhs_scalar:?}, orderings ({load_ordering:?}, {store_ordering:?})"
        );

        if matches!(rhs_scalar, Scalar::Ptr(..)) {
            throw_unsup_format!(
                "Right hand side of atomic read-modify-write operation cannot be a pointer"
            );
        }
        let genmc_rhs_scalar = scalar_to_genmc_scalar(ecx, rhs_scalar)?;
        let genmc_old_value = scalar_to_genmc_scalar(ecx, old_value)?;
        self.handle_atomic_rmw_op(
            ecx,
            address,
            size,
            load_ordering,
            store_ordering,
            genmc_rmw_op,
            genmc_rhs_scalar,
            genmc_old_value,
        )
    }

    /// Inform GenMC about an atomic `min` or `max` operation.
    ///
    /// Returns `(old_val, Option<new_val>)`. `new_val` might not be the latest write in coherence order, which is indicated by `None`.
    pub(crate) fn atomic_min_max_op<'tcx>(
        &self,
        ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
        address: Size,
        size: Size,
        ordering: AtomicRwOrd,
        min: bool,
        is_signed: bool,
        rhs_scalar: Scalar,
        // The value that we would get, if we were to do a non-atomic load here.
        old_value: Scalar,
    ) -> InterpResult<'tcx, (Scalar, Option<Scalar>)> {
        assert!(
            !self.get_alloc_data_races(),
            "atomic min/max operation with data race checking disabled."
        );
        let (load_ordering, store_ordering) = ordering.to_genmc_memory_orderings();
        let genmc_rmw_op = min_max_to_genmc_rmw_op(min, is_signed);
        tracing::info!(
            "GenMC: atomic_min_max_op (min: {min}, signed: {is_signed}, genmc_rmw_op: {genmc_rmw_op:?}): rhs value: {rhs_scalar:?}, orderings ({load_ordering:?}, {store_ordering:?})"
        );

        // FIXME(genmc): can `rhs_scalar` be a pointer? Should this be allowed?
        let genmc_rhs_scalar = scalar_to_genmc_scalar(ecx, rhs_scalar)?;
        let genmc_old_value = scalar_to_genmc_scalar(ecx, old_value)?;
        self.handle_atomic_rmw_op(
            ecx,
            address,
            size,
            load_ordering,
            store_ordering,
            genmc_rmw_op,
            genmc_rhs_scalar,
            genmc_old_value,
        )
    }

    /// Returns `(old_val, Option<new_val>)`. `new_val` might not be the latest write in coherence order, which is indicated by `None`.
    pub(crate) fn atomic_exchange<'tcx>(
        &self,
        ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
        address: Size,
        size: Size,
        rhs_scalar: Scalar,
        ordering: AtomicRwOrd,
        // The value that we would get, if we were to do a non-atomic load here.
        old_value: Scalar,
    ) -> InterpResult<'tcx, (Scalar, Option<Scalar>)> {
        assert!(
            !self.get_alloc_data_races(),
            "atomic swap operation with data race checking disabled."
        );

        let (load_ordering, store_ordering) = ordering.to_genmc_memory_orderings();
        let genmc_rmw_op = RMWBinOp::Xchg;
        tracing::info!(
            "GenMC: atomic_exchange (op: {genmc_rmw_op:?}): new value: {rhs_scalar:?}, orderings ({load_ordering:?}, {store_ordering:?})"
        );
        let genmc_rhs_scalar = scalar_to_genmc_scalar(ecx, rhs_scalar)?;
        let genmc_old_value = scalar_to_genmc_scalar(ecx, old_value)?;
        self.handle_atomic_rmw_op(
            ecx,
            address,
            size,
            load_ordering,
            store_ordering,
            genmc_rmw_op,
            genmc_rhs_scalar,
            genmc_old_value,
        )
    }

    pub(crate) fn atomic_compare_exchange<'tcx>(
        &self,
        ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
        address: Size,
        size: Size,
        expected_old_value: Scalar,
        new_value: Scalar,
        success: AtomicRwOrd,
        fail: AtomicReadOrd,
        can_fail_spuriously: bool,
        // The value that we would get, if we were to do a non-atomic load here.
        old_value: Scalar,
    ) -> InterpResult<'tcx, (Scalar, bool, bool)> {
        assert!(
            !self.get_alloc_data_races(),
            "atomic compare-exchange with data race checking disabled."
        );

        // FIXME(genmc): remove once GenMC supports failure memory ordering in `compare_exchange`.
        self.warnings_cache.borrow_mut().warn_once_rmw_failure_ordering(&ecx.tcx, success, fail);
        // FIXME(genmc): remove once GenMC implements spurious failures for `compare_exchange_weak`.
        if can_fail_spuriously {
            self.warnings_cache.borrow_mut().warn_once_compare_exchange_weak(&ecx.tcx);
        }

        let machine = &ecx.machine;
        let (success_load_ordering, success_store_ordering) = success.to_genmc_memory_orderings();
        let fail_load_ordering = fail.convert();

        info!(
            "GenMC: atomic_compare_exchange, address: {address:?}, size: {size:?} (expect: {expected_old_value:?}, new: {new_value:?}, old_value: {old_value:?}, {success:?}, {fail:?}), can fail spuriously: {can_fail_spuriously}"
        );
        info!(
            "GenMC: atomic_compare_exchange orderings: success: ({success_load_ordering:?}, {success_store_ordering:?}), failure load ordering: {fail_load_ordering:?}"
        );

        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let curr_thread = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_genmc_tid(curr_thread);

        let genmc_address = address.bytes();
        let genmc_size = size.bytes();

        let genmc_expected_value = scalar_to_genmc_scalar(ecx, expected_old_value)?;
        let genmc_new_value = scalar_to_genmc_scalar(ecx, new_value)?;
        let genmc_old_value = scalar_to_genmc_scalar(ecx, old_value)?;

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let cas_result = pinned_mc.handleCompareExchange(
            genmc_tid,
            genmc_address,
            genmc_size,
            genmc_expected_value,
            genmc_new_value,
            genmc_old_value,
            success_load_ordering,
            success_store_ordering,
            fail_load_ordering,
            can_fail_spuriously,
        );

        if let Some(error) = cas_result.error.as_ref() {
            throw_ub_format!("{}", error.to_string_lossy()); // TODO GENMC: proper error handling: find correct error here
        }

        let return_scalar = genmc_scalar_to_scalar(ecx, cas_result.old_value, size)?;
        info!(
            "GenMC: atomic_compare_exchange: result: {cas_result:?}, returning scalar: {return_scalar:?}"
        );
        // The write can only be a co-maximal write if the CAS succeeded.
        assert!(cas_result.is_success || !cas_result.isCoMaxWrite);
        interp_ok((return_scalar, cas_result.isCoMaxWrite, cas_result.is_success))
    }

    /// Inform GenMC about a non-atomic memory load
    ///
    /// NOTE: Unlike for *atomic* loads, we don't return a value here. Non-atomic values are still handled by Miri.
    pub(crate) fn memory_load<'tcx>(
        &self,
        machine: &MiriMachine<'tcx>,
        address: Size,
        size: Size,
    ) -> InterpResult<'tcx> {
        info!(
            "GenMC: received memory_load (non-atomic): address: {:#x}, size: {}",
            address.bytes(),
            size.bytes()
        );
        if self.get_alloc_data_races() {
            info!("GenMC: data race checking disabled, ignoring non-atomic load.");
            return interp_ok(());
        }
        // GenMC doesn't like ZSTs, and they can't have any data races, so we skip them
        if size.bytes() == 0 {
            return interp_ok(());
        }

        if size.bytes() <= 8 {
            // NOTE: Values loaded non-atomically are still handled by Miri, so we discard whatever we get from GenMC
            let _read_value = self.handle_load(
                machine,
                address,
                size,
                MemOrdering::NotAtomic,
                // Don't use DUMMY here, since that might have it stored as the initial value of the chunk.
                GenmcScalar::UNINIT,
            )?;
            return interp_ok(());
        }

        for (address, size) in split_access(address, size) {
            let chunk_addr = Size::from_bytes(address);
            let chunk_size = Size::from_bytes(size);
            let _read_value = self.handle_load(
                machine,
                chunk_addr,
                chunk_size,
                MemOrdering::NotAtomic,
                // Don't use DUMMY here, since that might have it stored as the initial value of the chunk.
                GenmcScalar::UNINIT,
            )?;
        }
        interp_ok(())
    }

    pub(crate) fn memory_store<'tcx>(
        &self,
        machine: &MiriMachine<'tcx>,
        address: Size,
        size: Size,
    ) -> InterpResult<'tcx> {
        info!(
            "GenMC: received memory_store (non-atomic): address: {:#x}, size: {}",
            address.bytes(),
            size.bytes()
        );
        if self.get_alloc_data_races() {
            info!("GenMC: data race checking disabled, ignoring non-atomic store.");
            return interp_ok(());
        }
        // GenMC doesn't like ZSTs, and they can't have any data races, so we skip them
        if size.bytes() == 0 {
            return interp_ok(());
        }

        if size.bytes() <= 8 {
            // TODO GENMC(mixed atomic-non-atomics): anything to do here?
            let _is_co_max_write = self.handle_store(
                machine,
                address,
                size,
                // We use DUMMY, since we don't know the actual value, but GenMC expects something.
                GenmcScalar::DUMMY,
                // Don't use DUMMY here, since that might have it stored as the initial value of the chunk.
                GenmcScalar::UNINIT,
                MemOrdering::NotAtomic,
            )?;
            return interp_ok(());
        }

        for (address, size) in split_access(address, size) {
            let chunk_addr = Size::from_bytes(address);
            let chunk_size = Size::from_bytes(size);
            let _is_co_max_write = self.handle_store(
                machine,
                chunk_addr,
                chunk_size,
                // We use DUMMY, since we don't know the actual value, but GenMC expects something.
                GenmcScalar::DUMMY,
                // Don't use DUMMY here, since that might have it stored as the initial value of the chunk.
                GenmcScalar::UNINIT,
                MemOrdering::NotAtomic,
            )?;
        }
        interp_ok(())
    }

    /**** Memory (de)allocation ****/

    pub(crate) fn handle_alloc<'tcx>(
        &self,
        ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
        alloc_id: AllocId,
        size: Size,
        alignment: Align,
        memory_kind: MemoryKind,
    ) -> InterpResult<'tcx, u64> {
        assert!(
            !self.get_alloc_data_races(),
            "memory allocation with data race checking disabled."
        );
        let machine = &ecx.machine;
        if memory_kind == MiriMemoryKind::Global.into() {
            info!("GenMC: global memory allocation: {alloc_id:?}");
            return ecx.get_global_allocation_address(&self.global_allocations, alloc_id);
        }
        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let curr_thread = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_genmc_tid(curr_thread);
        // GenMC doesn't support ZSTs, so we set the minimum size to 1 byte
        let genmc_size = size.bytes().max(1);
        info!(
            "GenMC: handle_alloc (thread: {curr_thread:?} ({genmc_tid:?}), size: {}, alignment: {alignment:?}, memory_kind: {memory_kind:?})",
            size.bytes()
        );

        let alignment = alignment.bytes();

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let chosen_address = pinned_mc.handleMalloc(genmc_tid, genmc_size, alignment);

        // Non-global addresses should not be in the global address space or null.
        assert_ne!(0, chosen_address, "GenMC malloc returned nullptr.");
        assert_eq!(0, chosen_address & GENMC_GLOBAL_ADDRESSES_MASK);
        // Sanity check the address alignment:
        assert!(
            chosen_address.is_multiple_of(alignment),
            "GenMC returned address {chosen_address:#x} with lower alignment than requested ({alignment}).",
        );

        interp_ok(chosen_address)
    }

    pub(crate) fn handle_dealloc<'tcx>(
        &self,
        machine: &MiriMachine<'tcx>,
        alloc_id: AllocId,
        address: Size,
        kind: MemoryKind,
    ) -> InterpResult<'tcx> {
        assert_ne!(
            kind,
            MiriMemoryKind::Global.into(),
            "we probably shouldn't try to deallocate global allocations (alloc_id: {alloc_id:?})"
        );
        assert!(
            !self.get_alloc_data_races(),
            "memory deallocation with data race checking disabled."
        );
        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let curr_thread = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_genmc_tid(curr_thread);

        let genmc_address = address.bytes();

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        pinned_mc.handleFree(genmc_tid, genmc_address);

        // TODO GENMC (ERROR HANDLING): can this ever fail?
        interp_ok(())
    }

    /**** Thread management ****/

    pub(crate) fn handle_thread_create<'tcx>(
        &self,
        threads: &ThreadManager<'tcx>,
        // FIXME(genmc,symmetry reduction): pass info to GenMC
        _start_routine: crate::Pointer,
        _func_arg: &crate::ImmTy<'tcx>,
        new_thread_id: ThreadId,
    ) -> InterpResult<'tcx> {
        assert!(!self.get_alloc_data_races(), "thread creation with data race checking disabled.");
        let mut thread_infos = self.exec_state.thread_id_manager.borrow_mut();

        let curr_thread_id = threads.active_thread();
        let genmc_parent_tid = thread_infos.get_genmc_tid(curr_thread_id);
        let genmc_new_tid = thread_infos.add_thread(new_thread_id);

        info!(
            "GenMC: handling thread creation (thread {curr_thread_id:?} ({genmc_parent_tid:?}) spawned thread {new_thread_id:?} ({genmc_new_tid:?}))"
        );

        let mut mc = self.handle.borrow_mut();
        mc.as_mut().handleThreadCreate(genmc_new_tid, genmc_parent_tid);

        // TODO GENMC (ERROR HANDLING): can this ever fail?
        interp_ok(())
    }

    pub(crate) fn handle_thread_join<'tcx>(
        &self,
        active_thread_id: ThreadId,
        child_thread_id: ThreadId,
    ) -> InterpResult<'tcx> {
        assert!(!self.get_alloc_data_races(), "thread join with data race checking disabled.");
        let thread_infos = self.exec_state.thread_id_manager.borrow();

        let genmc_curr_tid = thread_infos.get_genmc_tid(active_thread_id);
        let genmc_child_tid = thread_infos.get_genmc_tid(child_thread_id);

        let mut mc = self.handle.borrow_mut();
        mc.as_mut().handleThreadJoin(genmc_curr_tid, genmc_child_tid);

        interp_ok(())
    }

    pub(crate) fn handle_thread_finish<'tcx>(&self, threads: &ThreadManager<'tcx>) {
        assert!(!self.get_alloc_data_races(), "thread finish with data race checking disabled.");
        let curr_thread_id = threads.active_thread();

        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let genmc_tid = thread_infos.get_genmc_tid(curr_thread_id);

        // NOTE: Miri doesn't support return values for threads, but GenMC expects one, so we return 0
        let ret_val = 0;

        info!(
            "GenMC: handling thread finish (thread {curr_thread_id:?} ({genmc_tid:?}) returns with dummy value 0)"
        );

        let mut mc = self.handle.borrow_mut();
        mc.as_mut().handleThreadFinish(genmc_tid, ret_val);
    }

    /// Handle a call to `libc::exit` or the exit of the main thread.
    /// Unless an error is returned, the program should continue executing (in a different thread, chosen by the next scheduling call).
    pub(crate) fn handle_exit<'tcx>(
        &self,
        thread: ThreadId,
        exit_code: i32,
        is_exit_call: bool,
    ) -> InterpResult<'tcx> {
        // Calling `libc::exit` doesn't do cleanup, so we skip the leak check in that case.
        let exit_status = ExitStatus {
            exit_code,
            exit_type: if is_exit_call { ExitType::ExitCalled } else { ExitType::MainThreadFinish },
        };

        if let Some(old_exit_status) = self.exec_state.exit_status.get() {
            throw_ub_format!(
                "Exit called twice, first with status {old_exit_status:?}, now with status {exit_status:?}",
            );
        }

        // FIXME(genmc): Add a flag to continue exploration even when the program exits with a non-zero exit code.
        if exit_code != 0 {
            info!("GenMC: 'exit' called with non-zero argument, aborting execution.");
            let leak_check = exit_status.do_leak_check();
            throw_machine_stop!(TerminationInfo::Exit { code: exit_code, leak_check });
        }

        if is_exit_call {
            let thread_infos = self.exec_state.thread_id_manager.borrow();
            let genmc_tid = thread_infos.get_genmc_tid(thread);

            let mut mc = self.handle.borrow_mut();
            mc.as_mut().handleThreadKill(genmc_tid);
        }
        // We continue executing now, so we store the exit status.
        self.exec_state.exit_status.set(Some(exit_status));
        interp_ok(())
    }
}

impl GenmcCtx {
    /// Inform GenMC about a load (atomic or non-atomic).
    /// Returns the value that GenMC wants this load to read.
    fn handle_load<'tcx>(
        &self,
        machine: &MiriMachine<'tcx>,
        address: Size,
        size: Size,
        memory_ordering: MemOrdering,
        genmc_old_value: GenmcScalar,
    ) -> InterpResult<'tcx, GenmcScalar> {
        assert!(
            size.bytes() != 0
                && (memory_ordering == MemOrdering::NotAtomic || size.bytes().is_power_of_two())
        );
        if size.bytes() > 8 {
            throw_unsup_format!("{UNSUPPORTED_ATOMICS_SIZE_MSG}");
        }
        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let curr_thread_id = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_genmc_tid(curr_thread_id);

        info!(
            "GenMC: load, thread: {curr_thread_id:?} ({genmc_tid:?}), address: {addr} == {addr:#x}, size: {size:?}, ordering: {memory_ordering:?}, old_value: {genmc_old_value:x?}",
            addr = address.bytes()
        );
        let genmc_address = address.bytes();
        let genmc_size = size.bytes();

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let load_result = pinned_mc.handleLoad(
            genmc_tid,
            genmc_address,
            genmc_size,
            memory_ordering,
            genmc_old_value,
        );

        if let Some(error) = load_result.error.as_ref() {
            throw_ub_format!("{}", error.to_string_lossy()); // TODO GENMC: proper error handling: find correct error here
        }

        if !load_result.has_value {
            // FIXME(GenMC): Implementing certain GenMC optimizations will lead to this.
            unimplemented!("GenMC: load returned no value.");
        }

        info!("GenMC: load returned value: {:?}", load_result.read_value);

        interp_ok(load_result.read_value)
    }

    /// Inform GenMC about a store (atomic or non-atomic).
    /// Returns true if the store is co-maximal, i.e., it should be written to Miri's memory too.
    fn handle_store<'tcx>(
        &self,
        machine: &MiriMachine<'tcx>,
        address: Size,
        size: Size,
        genmc_value: GenmcScalar,
        genmc_old_value: GenmcScalar,
        memory_ordering: MemOrdering,
    ) -> InterpResult<'tcx, bool> {
        assert!(
            size.bytes() != 0
                && (memory_ordering == MemOrdering::NotAtomic || size.bytes().is_power_of_two())
        );
        if size.bytes() > 8 {
            throw_unsup_format!("{UNSUPPORTED_ATOMICS_SIZE_MSG}");
        }
        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let curr_thread_id = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_genmc_tid(curr_thread_id);

        let genmc_address = address.bytes();
        let genmc_size = size.bytes();

        info!(
            "GenMC: store, thread: {curr_thread_id:?} ({genmc_tid:?}), address: {addr} = {addr:#x}, size: {size:?}, ordering {memory_ordering:?}, value: {genmc_value:?}",
            addr = address.bytes()
        );

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let store_result = pinned_mc.handleStore(
            genmc_tid,
            genmc_address,
            genmc_size,
            genmc_value,
            genmc_old_value,
            memory_ordering,
        );

        if let Some(error) = store_result.error.as_ref() {
            throw_ub_format!("{}", error.to_string_lossy()); // TODO GENMC: proper error handling: find correct error here
        }

        interp_ok(store_result.isCoMaxWrite)
    }

    /// Inform GenMC about an atomic read-modify-write operation.
    /// For GenMC, compare-exchange and atomic-swap are also RMW (see `RMWBinOp` for full list of operations).
    /// Returns the previous value at that memory location, and optionally the value that should be written back to Miri's memory.
    fn handle_atomic_rmw_op<'tcx>(
        &self,
        ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
        address: Size,
        size: Size,
        load_ordering: MemOrdering,
        store_ordering: MemOrdering,
        genmc_rmw_op: RMWBinOp,
        genmc_rhs_scalar: GenmcScalar,
        genmc_old_value: GenmcScalar,
    ) -> InterpResult<'tcx, (Scalar, Option<Scalar>)> {
        assert!(
            size.bytes() <= 8,
            "TODO GENMC: no support for accesses larger than 8 bytes (got {} bytes)",
            size.bytes()
        );
        let machine = &ecx.machine;
        assert_ne!(0, size.bytes());
        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let curr_thread_id = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_genmc_tid(curr_thread_id);

        let genmc_address = address.bytes();
        let genmc_size = size.bytes();

        info!(
            "GenMC: atomic_rmw_op, thread: {curr_thread_id:?} ({genmc_tid:?}) (op: {genmc_rmw_op:?}, rhs value: {genmc_rhs_scalar:?}), address: {address:?}, size: {size:?}, orderings: ({load_ordering:?}, {store_ordering:?})",
        );

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let rmw_result = pinned_mc.handleReadModifyWrite(
            genmc_tid,
            genmc_address,
            genmc_size,
            load_ordering,
            store_ordering,
            genmc_rmw_op,
            genmc_rhs_scalar,
            genmc_old_value,
        );

        if let Some(error) = rmw_result.error.as_ref() {
            throw_ub_format!("{}", error.to_string_lossy()); // TODO GENMC: proper error handling: find correct error here
        }

        // Check that both RMW arguments have sane provenance.
        match (
            genmc_rmw_op,
            rmw_result.old_value.has_provenance(),
            genmc_rhs_scalar.has_provenance(),
        ) {
            // compare-exchange should not swap a pointer and an integer.
            // FIXME(GenMC): is this correct?
            (RMWBinOp::Xchg, left, right) =>
                if left != right {
                    throw_ub_format!(
                        "atomic compare-exchange arguments should either both have pointer provenance ({left} != {right}). Both arguments should be pointers, or both integers."
                    );
                },
            // All other read-modify-write operations should never have a right-side argument that's a pointer.
            (_, _, true) =>
                throw_ub_format!(
                    "atomic read-modify-write operation right argument has pointer provenance."
                ),
            _ => {}
        }

        let old_value_scalar = genmc_scalar_to_scalar(ecx, rmw_result.old_value, size)?;

        let new_value_scalar = if rmw_result.isCoMaxWrite {
            Some(genmc_scalar_to_scalar(ecx, rmw_result.new_value, size)?)
        } else {
            None
        };
        interp_ok((old_value_scalar, new_value_scalar))
    }

    /**** Blocking functionality ****/

    /// Handle a user thread getting blocked.
    /// This may happen due to an manual `assume` statement added by a user
    /// or added by some automated program transformation, e.g., for spinloops.
    fn handle_user_block<'tcx>(&self, machine: &MiriMachine<'tcx>) -> InterpResult<'tcx> {
        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let curr_thread = machine.threads.active_thread();
        let genmc_curr_thread = thread_infos.get_genmc_tid(curr_thread);
        info!("GenMC: handle_user_block, blocking thread {curr_thread:?} ({genmc_curr_thread:?})");

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        pinned_mc.handleUserBlock(genmc_curr_thread);

        interp_ok(())
    }
}

/// Other functionality not directly related to event handling
impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    /// Given a `ty::Instance<'tcx>`, do any required special handling. Returns true if this `instance` should be skipped (i.e., no Mir should be executed for it).
    fn check_genmc_intercept_function(
        &mut self,
        instance: rustc_middle::ty::Instance<'tcx>,
        args: &[rustc_const_eval::interpret::FnArg<'tcx, crate::Provenance>],
        dest: &crate::PlaceTy<'tcx>,
        ret: Option<mir::BasicBlock>,
    ) -> InterpResult<'tcx, bool> {
        let this = self.eval_context_mut();
        let genmc_ctx = this
            .machine
            .data_race
            .as_genmc_ref()
            .expect("This function should only be called in GenMC mode.");

        let get_mutex_call_infos = || {
            // assert!(!args.is_empty());
            assert_eq!(args.len(), 1);
            let arg = this.copy_fn_arg(&args[0]);
            let addr = this.read_target_usize(&arg)?;
            // FIXME(genmc): assert that we have at least 1 byte.
            // FIXME(genmc): maybe use actual size of mutex here?.

            let thread_infos = genmc_ctx.exec_state.thread_id_manager.borrow();
            let curr_thread = this.machine.threads.active_thread();
            let genmc_curr_thread = thread_infos.get_genmc_tid(curr_thread);
            interp_ok((genmc_curr_thread, addr, 1))
        };

        use rustc_span::sym;
        if this.tcx.is_diagnostic_item(sym::sys_mutex_lock, instance.def_id()) {
            info!("GenMC: handling Mutex::lock()");
            let (genmc_curr_thread, addr, size) = get_mutex_call_infos()?;

            let result = {
                let mut mc = genmc_ctx.handle.borrow_mut();
                let pinned_mc = mc.as_mut();
                pinned_mc.handleMutexLock(genmc_curr_thread, addr, size)
            };
            if let Some(error) = result.error.as_ref() {
                throw_ub_format!("{}", error.to_string_lossy());
            }
            // TODO GENMC(HACK): for determining if the Mutex lock blocks this thread.
            if !result.is_lock_acquired {
                fn create_callback<'tcx>(
                    genmc_curr_thread: i32,
                    addr: u64,
                    size: u64,
                ) -> crate::DynUnblockCallback<'tcx> {
                    crate::callback!(
                        @capture<'tcx> {
                            // mutex_ref: MutexRef,
                            // retval_dest: Option<(Scalar, MPlaceTy<'tcx>)>,
                            genmc_curr_thread: i32,
                            addr: u64,
                            size: u64,
                        }
                        |this, unblock: crate::UnblockKind| {
                            assert_eq!(unblock, crate::UnblockKind::Ready);
                            let genmc_ctx = this.machine.data_race.as_genmc_ref().unwrap();

                            info!("GenMC: handling Mutex::lock: unblocking callback called!");
                            let result = {
                                let mut mc = genmc_ctx.handle.borrow_mut();
                                let pinned_mc = mc.as_mut();
                                pinned_mc.handleMutexLock(genmc_curr_thread, addr, size)
                            };
                            if let Some(error) = result.error.as_ref() {
                                throw_ub_format!("{}", error.to_string_lossy());
                            }
                            // TODO GENMC(HACK): for determining if the Mutex lock blocks this thread.
                            if !result.is_lock_acquired {
                                // If this thread gets woken up without the mutex being made available, block the thread again.
                                this.block_thread( crate::BlockReason::Mutex, None, create_callback(genmc_curr_thread, addr, size));
                            }
                            interp_ok(())
                        }
                    )
                }

                info!("GenMC: handling Mutex::lock failed, blocking thread");
                // NOTE: We don't write anything back to Miri's memory, the Mutex state is handled only by GenMC.

                this.block_thread(
                    crate::BlockReason::Mutex,
                    None,
                    create_callback(genmc_curr_thread, addr, size),
                );
            } else {
                info!("GenMC: handling Mutex::lock: success: lock acquired.");
            }
        } else if this.tcx.is_diagnostic_item(sym::sys_mutex_try_lock, instance.def_id()) {
            info!("GenMC: handling Mutex::try_lock()");
            let (genmc_curr_thread, addr, size) = get_mutex_call_infos()?;

            let result = {
                let mut mc = genmc_ctx.handle.borrow_mut();
                let pinned_mc = mc.as_mut();
                pinned_mc.handleMutexTryLock(genmc_curr_thread, addr, size)
            };
            if let Some(error) = result.error.as_ref() {
                throw_ub_format!("{}", error.to_string_lossy());
            }
            info!(
                "GenMC: Mutex::try_lock(): writing resulting bool is_lock_acquired ({}) to place: {dest:?}",
                result.is_lock_acquired
            );

            this.write_scalar(Scalar::from_bool(result.is_lock_acquired), dest)?;
            // todo!("return whether lock was successful or not");
        } else if this.tcx.is_diagnostic_item(sym::sys_mutex_unlock, instance.def_id()) {
            info!("GenMC: handling Mutex::unlock()");
            let (genmc_curr_thread, addr, size) = get_mutex_call_infos()?;

            let mut mc = genmc_ctx.handle.borrow_mut();
            let pinned_mc = mc.as_mut();
            let result = pinned_mc.handleMutexUnlock(genmc_curr_thread, addr, size);
            if let Some(error) = result.error.as_ref() {
                throw_ub_format!("{}", error.to_string_lossy());
            }
            // NOTE: We don't write anything back to Miri's memory, the Mutex state is handled only by GenMC.

            // this.unblock_thread(, crate::BlockReason::Mutex)?;
        } else {
            return interp_ok(false);
        };

        this.return_to_block(ret)?;

        interp_ok(true)
    }

    /**** Blocking instructions ****/

    /// Handle an `assume` statement. This will tell GenMC to block the current thread if the `condition` is false.
    /// Returns `true` if the current thread should be blocked in Miri too.
    fn handle_genmc_verifier_assume(&mut self, condition: &OpTy<'tcx>) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let condition_bool = this.read_scalar(condition)?.to_bool()?;
        info!("GenMC: handle_genmc_verifier_assume, condition: {condition:?} = {condition_bool}");
        if condition_bool {
            return interp_ok(());
        }
        let genmc_ctx = this.machine.data_race.as_genmc_ref().unwrap();
        genmc_ctx.handle_user_block(&this.machine)?;
        let condition = condition.clone();
        this.block_thread(
            BlockReason::GenmcAssume,
            None,
            callback!(
                @capture<'tcx> {
                    condition: OpTy<'tcx>,
                }
                |this, unblock: UnblockKind| {
                    assert_eq!(unblock, UnblockKind::Ready);

                    let condition = this.run_for_validation_ref(|this| this.read_scalar(&condition))?.to_bool()?;
                    assert!(condition);

                    interp_ok(())
                }
            ),
        );
        interp_ok(())
    }
}

impl VisitProvenance for GenmcCtx {
    fn visit_provenance(&self, _visit: &mut VisitWith<'_>) {
        // We don't have any tags.
    }
}
