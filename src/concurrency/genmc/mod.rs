use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::Duration;

use genmc_sys::cxx_extra::NonNullUniquePtr;
use genmc_sys::{
    ActionKind, GENMC_GLOBAL_ADDRESSES_MASK, GenmcScalar, GenmcThreadId, MemOrdering,
    MiriGenMCShim, RMWBinOp, StoreEventType, createGenmcHandle,
};
use rustc_abi::{Align, Size};
use rustc_const_eval::interpret::{AllocId, InterpCx, InterpResult, interp_ok};
use rustc_middle::{mir, throw_machine_stop, throw_ub_format, throw_unsup_format};
use tracing::info;

use self::global_allocations::{EvalContextExtPriv as _, GlobalAllocationHandler};
use self::helper::{
    genmc_scalar_to_scalar, option_scalar_to_genmc_scalar, rhs_scalar_to_genmc_scalar,
    scalar_to_genmc_scalar,
};
use self::mapping::{min_max_to_genmc_rmw_op, to_genmc_rmw_op};
use self::thread_info_manager::ThreadInfoManager;
use crate::concurrency::genmc::helper::{is_terminator_atomic, split_access};
use crate::concurrency::genmc::warnings::WarningsCache;
use crate::concurrency::thread::{EvalContextExt as _, ThreadState};
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
mod thread_info_manager;
mod warnings;

pub use genmc_sys::GenmcParams;

pub use self::config::GenmcConfig;

const UNSUPPORTED_ATOMICS_SIZE_MSG: &str =
    "GenMC mode currently does not support atomics larger than 8 bytes.";

#[derive(Clone, Copy)]
struct ExitStatus {
    exit_code: i32,
    leak_check: bool,
}

pub struct GenmcCtx {
    handle: RefCell<NonNullUniquePtr<MiriGenMCShim>>,

    // TODO GENMC (PERFORMANCE): could use one RefCell for all internals instead of multiple
    thread_infos: RefCell<ThreadInfoManager>,

    /// Some actions Miri does are allowed to cause data races.
    /// GenMC will not be informed about certain actions (e.g. non-atomic loads) when this flag is set.
    allow_data_races: Cell<bool>,

    /// Keep track of global allocations, to ensure they keep the same address across different executions, even if the order of allocations changes.
    /// The `AllocId` for globals is stable across executions, so we can use it as an identifier.
    global_allocations: Arc<GlobalAllocationHandler>,
    // TODO GENMC: maybe make this a (base, size), maybe BTreeMap/sorted vector for reverse lookups
    //          GenMC needs to have access to that
    // TODO: look at code of "pub struct GlobalStateInner"
    warnings_cache: RefCell<WarningsCache>,
    // terminator_cache: RefCell<FxHashMap<rustc_middle::ty::Ty<'tcx>, bool>>,
    exit_status: Cell<Option<ExitStatus>>,
}

/// GenMC Context creation and administrative / query actions
impl GenmcCtx {
    /// Create a new `GenmcCtx` from a given config.
    pub fn new(miri_config: &MiriConfig, mode: miri_genmc::Mode) -> Self {
        let genmc_config = miri_config.genmc_config.as_ref().unwrap();
        info!("GenMC: Creating new GenMC Context");

        let handle = createGenmcHandle(&genmc_config.params, mode == miri_genmc::Mode::Estimation);
        let non_null_handle = NonNullUniquePtr::new(handle).expect("GenMC should not return null");
        let non_null_handle = RefCell::new(non_null_handle);

        Self {
            handle: non_null_handle,
            thread_infos: Default::default(),
            allow_data_races: Cell::new(false),
            global_allocations: Default::default(),
            warnings_cache: Default::default(),
            exit_status: Cell::new(None),
        }
    }

    pub fn print_estimation_result(&self, elapsed_time: Duration) {
        let elapsed_time_sec = elapsed_time.as_secs_f64();
        let mc = self.handle.borrow();
        mc.as_ref().printEstimationResults(elapsed_time_sec);
    }

    pub fn get_blocked_execution_count(&self) -> u64 {
        let mc = self.handle.borrow();
        mc.as_ref().getBlockedExecutionCount()
    }

    pub fn get_explored_execution_count(&self) -> u64 {
        let mc = self.handle.borrow();
        mc.as_ref().getExploredExecutionCount()
    }

    pub fn print_genmc_graph(&self) {
        info!("GenMC: print the Execution graph");
        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        pinned_mc.printGraph();
    }

    /// This function determines if we should continue exploring executions or if we are done.
    ///
    /// In GenMC mode, the input program should be repeatedly executed until this function returns `true` or an error is found.
    pub fn is_exploration_done(&self) -> bool {
        let mut mc = self.handle.borrow_mut();
        mc.as_mut().isExplorationDone()
    }

    pub fn get_exit_status(&self) -> Option<(i32, bool)> {
        let ExitStatus { exit_code, leak_check } = self.exit_status.get()?;
        Some((exit_code, leak_check))
    }

    fn set_exit_status(&self, exit_code: i32, leak_check: bool) {
        self.exit_status.set(Some(ExitStatus { exit_code, leak_check }));
    }
}

/// GenMC event handling. These methods are used to inform GenMC about events happening in the program, and to handle scheduling decisions.
impl GenmcCtx {
    /**** Memory access handling ****/

    /// Inform GenMC that a new program execution has started.
    /// This function should be called at the start of every execution.
    pub(crate) fn handle_execution_start(&self) {
        self.allow_data_races.replace(false);
        self.thread_infos.borrow_mut().reset();
        self.exit_status.set(None);

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
            let msg = msg.to_string_lossy().to_string();
            info!("GenMC: execution ended with error \"{msg}\"");
            Err(msg) // TODO GENMC: add more error info here, and possibly handle this without requiring to clone the CxxString
        } else {
            Ok(())
        }
    }

    /**** Memory access handling ****/

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
        let old = self.allow_data_races.replace(enable);
        assert_ne!(old, enable, "cannot nest allow_data_races");
    }

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
        info!("GenMC: atomic_load: old_val: {old_val:?}");
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        let ordering = ordering.convert();
        let genmc_old_value = option_scalar_to_genmc_scalar(ecx, old_val)?;
        let read_value =
            self.atomic_load_impl(&ecx.machine, address, size, ordering, genmc_old_value)?;
        info!("GenMC: atomic_load: received value from GenMC: {read_value:?}");
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
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        let ordering = ordering.convert();
        let genmc_value = scalar_to_genmc_scalar(ecx, value)?;
        let genmc_old_value = option_scalar_to_genmc_scalar(ecx, old_value)?;
        self.atomic_store_impl(&ecx.machine, address, size, genmc_value, genmc_old_value, ordering)
    }

    pub(crate) fn atomic_fence<'tcx>(
        &self,
        machine: &MiriMachine<'tcx>,
        ordering: AtomicFenceOrd,
    ) -> InterpResult<'tcx> {
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        info!("GenMC: atomic_fence with ordering: {ordering:?}");

        let ordering = ordering.convert();

        let thread_infos = self.thread_infos.borrow();
        let curr_thread = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_info(curr_thread).genmc_tid;

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        pinned_mc.handleFence(genmc_tid.0, ordering);

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
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        let (load_ordering, store_ordering) = ordering.to_genmc_memory_orderings();
        let genmc_rmw_op = to_genmc_rmw_op(rmw_op, not);
        tracing::info!(
            "GenMC: atomic_rmw_op (op: {rmw_op:?}, not: {not}, genmc_rmw_op: {genmc_rmw_op:?}): rhs value: {rhs_scalar:?}, orderings ({load_ordering:?}, {store_ordering:?})"
        );
        let genmc_rhs_scalar = rhs_scalar_to_genmc_scalar(ecx, rhs_scalar)?;
        let genmc_old_value = scalar_to_genmc_scalar(ecx, old_value)?;
        self.atomic_rmw_op_impl(
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
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        let (load_ordering, store_ordering) = ordering.to_genmc_memory_orderings();
        let genmc_rmw_op = min_max_to_genmc_rmw_op(min, is_signed);
        tracing::info!(
            "GenMC: atomic_min_max_op (min: {min}, signed: {is_signed}, genmc_rmw_op: {genmc_rmw_op:?}): rhs value: {rhs_scalar:?}, orderings ({load_ordering:?}, {store_ordering:?})"
        );
        let genmc_rhs_scalar = rhs_scalar_to_genmc_scalar(ecx, rhs_scalar)?;
        let genmc_old_value = scalar_to_genmc_scalar(ecx, old_value)?;
        self.atomic_rmw_op_impl(
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
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        // TODO GENMC: could maybe merge this with atomic_rmw?

        let (load_ordering, store_ordering) = ordering.to_genmc_memory_orderings();
        let genmc_rmw_op = RMWBinOp::Xchg;
        tracing::info!(
            "GenMC: atomic_exchange (op: {genmc_rmw_op:?}): new value: {rhs_scalar:?}, orderings ({load_ordering:?}, {store_ordering:?})"
        );
        let genmc_rhs_scalar = scalar_to_genmc_scalar(ecx, rhs_scalar)?;
        let genmc_old_value = scalar_to_genmc_scalar(ecx, old_value)?;
        self.atomic_rmw_op_impl(
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
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly

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

        let thread_infos = self.thread_infos.borrow();
        let curr_thread = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_info(curr_thread).genmc_tid;

        let genmc_address = address.bytes();
        let genmc_size = size.bytes();

        let genmc_expected_value = scalar_to_genmc_scalar(ecx, expected_old_value)?;
        let genmc_new_value = scalar_to_genmc_scalar(ecx, new_value)?;
        let genmc_old_value = scalar_to_genmc_scalar(ecx, old_value)?;

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let cas_result = pinned_mc.handleCompareExchange(
            genmc_tid.0,
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
            let msg = error.to_string_lossy().to_string();
            info!("GenMC: RMW operation returned an error: \"{msg}\"");
            throw_ub_format!("{}", msg); // TODO GENMC: proper error handling: find correct error here
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
        if self.allow_data_races.get() {
            info!("GenMC: skipping `memory_load` on address");
            return interp_ok(());
        }
        // GenMC doesn't like ZSTs, and they can't have any data races, so we skip them
        if size.bytes() == 0 {
            return interp_ok(());
        }

        if size.bytes() <= 8 {
            // NOTE: Values loaded non-atomically are still handled by Miri, so we discard whatever we get from GenMC
            let _read_value = self.atomic_load_impl(
                machine,
                address,
                size,
                MemOrdering::NotAtomic,
                GenmcScalar::UNINIT, // Don't use DUMMY here, since that might have it stored as the initial value
            )?;
            return interp_ok(());
        }

        for (address, size) in split_access(address, size) {
            let chunk_addr = Size::from_bytes(address);
            let chunk_size = Size::from_bytes(size);
            let _read_value = self.atomic_load_impl(
                machine,
                chunk_addr,
                chunk_size,
                MemOrdering::NotAtomic,
                GenmcScalar::UNINIT, // Don't use DUMMY here, since that might have it stored as the initial value
            )?;
        }
        interp_ok(())
    }

    pub(crate) fn memory_store<'tcx>(
        &self,
        machine: &MiriMachine<'tcx>,
        address: Size,
        size: Size,
        // old_value: Option<Scalar>, // TODO GENMC(mixed atomic-non-atomic): is this needed?
    ) -> InterpResult<'tcx> {
        info!(
            "GenMC: received memory_store (non-atomic): address: {:#x}, size: {}",
            address.bytes(),
            size.bytes()
        );
        if self.allow_data_races.get() {
            info!("GenMC: skipping `memory_store`");
            return interp_ok(());
        }
        // GenMC doesn't like ZSTs, and they can't have any data races, so we skip them
        if size.bytes() == 0 {
            return interp_ok(());
        }

        if size.bytes() <= 8 {
            // TODO GENMC(mixed atomic-non-atomics): anything to do here?
            let _is_co_max_write = self.atomic_store_impl(
                machine,
                address,
                size,
                GenmcScalar::DUMMY,
                GenmcScalar::UNINIT, // Don't use DUMMY here, since that might have it stored as the initial value
                MemOrdering::NotAtomic,
            )?;
            return interp_ok(());
        }

        for (address, size) in split_access(address, size) {
            let chunk_addr = Size::from_bytes(address);
            let chunk_size = Size::from_bytes(size);
            let _is_co_max_write = self.atomic_store_impl(
                machine,
                chunk_addr,
                chunk_size,
                GenmcScalar::DUMMY,
                GenmcScalar::UNINIT, // Don't use DUMMY here, since that might have it stored as the initial value
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
        let machine = &ecx.machine;
        let chosen_address = if memory_kind == MiriMemoryKind::Global.into() {
            info!("GenMC: global memory allocation: {alloc_id:?}");
            ecx.get_global_allocation_address(&self.global_allocations, alloc_id)?
        } else {
            // TODO GENMC: Does GenMC need to know about the kind of Memory?

            // eprintln!(
            //     "handle_alloc ({memory_kind:?}): Custom backtrace: {}",
            //     std::backtrace::Backtrace::force_capture()
            // );
            // TODO GENMC: should we put this before the special handling for globals?
            if self.allow_data_races.get() {
                unreachable!(); // FIXME(genmc): can this happen and if yes, how should this be handled?
            }
            let thread_infos = self.thread_infos.borrow();
            let curr_thread = machine.threads.active_thread();
            let genmc_tid = thread_infos.get_info(curr_thread).genmc_tid;
            // GenMC doesn't support ZSTs, so we set the minimum size to 1 byte
            let genmc_size = size.bytes().max(1);
            info!(
                "GenMC: handle_alloc (thread: {curr_thread:?} ({genmc_tid:?}), size: {}, alignment: {alignment:?}, memory_kind: {memory_kind:?})",
                size.bytes()
            );

            let alignment = alignment.bytes();

            let mut mc = self.handle.borrow_mut();
            let pinned_mc = mc.as_mut();
            let chosen_address = pinned_mc.handleMalloc(genmc_tid.0, genmc_size, alignment);
            info!("GenMC: handle_alloc: got address '{chosen_address}' ({chosen_address:#x})");

            // TODO GENMC:
            if chosen_address == 0 {
                throw_unsup_format!("TODO GENMC: we got address '0' from malloc");
            }
            assert_eq!(0, chosen_address & GENMC_GLOBAL_ADDRESSES_MASK);
            chosen_address
        };
        // Sanity check the address alignment:
        assert_eq!(
            0,
            chosen_address % alignment.bytes(),
            "GenMC returned address {chosen_address} == {chosen_address:#x} with lower alignment than requested ({:})!",
            alignment.bytes()
        );

        interp_ok(chosen_address)
    }

    pub(crate) fn handle_dealloc<'tcx>(
        &self,
        machine: &MiriMachine<'tcx>,
        alloc_id: AllocId,
        address: Size,
        size: Size,
        align: Align,
        kind: MemoryKind,
    ) -> InterpResult<'tcx> {
        assert_ne!(
            kind,
            MiriMemoryKind::Global.into(),
            "we probably shouldn't try to deallocate global allocations (alloc_id: {alloc_id:?})"
        );
        if self.allow_data_races.get() {
            unreachable!(); // FIXME(genmc): can this happen and if yes, how should this be handled?
        }
        let thread_infos = self.thread_infos.borrow();
        let curr_thread = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_info(curr_thread).genmc_tid;
        info!(
            "GenMC: memory deallocation, thread: {curr_thread:?} ({genmc_tid:?}), address: {addr} == {addr:#x}, size: {size:?}, align: {align:?}, memory_kind: {kind:?}",
            addr = address.bytes()
        );

        let genmc_address = address.bytes();
        // GenMC doesn't support ZSTs, so we set the minimum size to 1 byte
        let genmc_size = size.bytes().max(1);

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        pinned_mc.handleFree(genmc_tid.0, genmc_address, genmc_size);

        // TODO GENMC (ERROR HANDLING): can this ever fail?
        interp_ok(())
    }

    /**** Thread management ****/

    pub(crate) fn handle_thread_create<'tcx>(
        &self,
        threads: &ThreadManager<'tcx>,
        _start_routine: crate::Pointer, // TODO GENMC: pass info to GenMC
        _func_arg: &crate::ImmTy<'tcx>,
        new_thread_id: ThreadId,
    ) -> InterpResult<'tcx> {
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        let mut thread_infos = self.thread_infos.borrow_mut();

        let curr_thread_id = threads.active_thread();
        let genmc_parent_tid = thread_infos.get_info(curr_thread_id).genmc_tid;
        let genmc_new_tid = thread_infos.add_thread(new_thread_id);

        info!(
            "GenMC: handling thread creation (thread {curr_thread_id:?} ({genmc_parent_tid:?}) spawned thread {new_thread_id:?} ({genmc_new_tid:?}))"
        );

        let mut mc = self.handle.borrow_mut();
        mc.as_mut().handleThreadCreate(genmc_new_tid.0, genmc_parent_tid.0);

        // TODO GENMC (ERROR HANDLING): can this ever fail?
        interp_ok(())
    }

    pub(crate) fn handle_thread_join<'tcx>(
        &self,
        active_thread_id: ThreadId,
        child_thread_id: ThreadId,
    ) -> InterpResult<'tcx> {
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        let thread_infos = self.thread_infos.borrow();

        let genmc_curr_tid = thread_infos.get_info(active_thread_id).genmc_tid;
        let genmc_child_tid = thread_infos.get_info(child_thread_id).genmc_tid;

        info!(
            "GenMC: handling thread joining (thread {active_thread_id:?} ({genmc_curr_tid:?}) joining thread {child_thread_id:?} ({genmc_child_tid:?}))"
        );

        let mut mc = self.handle.borrow_mut();
        mc.as_mut().handleThreadJoin(genmc_curr_tid.0, genmc_child_tid.0);

        interp_ok(())
    }

    pub(crate) fn handle_thread_finish<'tcx>(&self, threads: &ThreadManager<'tcx>) {
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        let curr_thread_id = threads.active_thread();

        let thread_infos = self.thread_infos.borrow();
        let genmc_tid = thread_infos.get_info(curr_thread_id).genmc_tid;

        // NOTE: Miri doesn't support return values for threads, but GenMC expects one, so we return 0
        let ret_val = 0;

        info!(
            "GenMC: handling thread finish (thread {curr_thread_id:?} ({genmc_tid:?}) returns with dummy value 0)"
        );

        let mut mc = self.handle.borrow_mut();
        mc.as_mut().handleThreadFinish(genmc_tid.0, ret_val);
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
        let leak_check = !is_exit_call;
        self.set_exit_status(exit_code, leak_check);

        // FIXME(genmc): Add a flag to continue exploration even when the program exits with a non-zero exit code.
        if exit_code != 0 {
            info!("GenMC: 'exit' called with non-zero argument, aborting execution.");
            throw_machine_stop!(TerminationInfo::GenmcFinishedExecution);
        }

        if is_exit_call {
            let thread_infos = self.thread_infos.borrow();
            let genmc_tid = thread_infos.get_info(thread).genmc_tid;

            let mut mc = self.handle.borrow_mut();
            mc.as_mut().handleThreadKill(genmc_tid.0);
        }
        interp_ok(())
    }
}

impl GenmcCtx {
    //* might fails if there's a race, load might also not read anything (returns None) */
    fn atomic_load_impl<'tcx>(
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
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        let thread_infos = self.thread_infos.borrow();
        let curr_thread_id = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_info(curr_thread_id).genmc_tid;

        info!(
            "GenMC: load, thread: {curr_thread_id:?} ({genmc_tid:?}), address: {addr} == {addr:#x}, size: {size:?}, ordering: {memory_ordering:?}, old_value: {genmc_old_value:x?}",
            addr = address.bytes()
        );
        let genmc_address = address.bytes();
        let genmc_size = size.bytes();

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let load_result = pinned_mc.handleLoad(
            genmc_tid.0,
            genmc_address,
            genmc_size,
            memory_ordering,
            genmc_old_value,
        );

        if load_result.is_read_opt {
            todo!();
        }

        if let Some(error) = load_result.error.as_ref() {
            let msg = error.to_string_lossy().to_string();
            info!("GenMC: load operation returned an error: \"{msg}\"");
            throw_ub_format!("{}", msg); // TODO GENMC: proper error handling: find correct error here
        }

        info!("GenMC: load returned value: {:?}", load_result.read_value);

        interp_ok(load_result.read_value)
    }

    fn atomic_store_impl<'tcx>(
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
        let thread_infos = self.thread_infos.borrow();
        let curr_thread_id = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_info(curr_thread_id).genmc_tid;

        let genmc_address = address.bytes();
        let genmc_size = size.bytes();

        info!(
            "GenMC: store, thread: {curr_thread_id:?} ({genmc_tid:?}), address: {addr} = {addr:#x}, size: {size:?}, ordering {memory_ordering:?}, value: {genmc_value:?}",
            addr = address.bytes()
        );

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let store_result = pinned_mc.handleStore(
            genmc_tid.0,
            genmc_address,
            genmc_size,
            genmc_value,
            genmc_old_value,
            memory_ordering,
            StoreEventType::Normal,
        );

        if let Some(error) = store_result.error.as_ref() {
            let msg = error.to_string_lossy().to_string();
            info!("GenMC: store operation returned an error: \"{msg}\"");
            throw_ub_format!("{}", msg); // TODO GENMC: proper error handling: find correct error here
        }

        interp_ok(store_result.isCoMaxWrite)
    }

    fn atomic_rmw_op_impl<'tcx>(
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
        let thread_infos = self.thread_infos.borrow();
        let curr_thread_id = machine.threads.active_thread();
        let genmc_tid = thread_infos.get_info(curr_thread_id).genmc_tid;

        let genmc_address = address.bytes();
        let genmc_size = size.bytes();

        info!(
            "GenMC: atomic_rmw_op, thread: {curr_thread_id:?} ({genmc_tid:?}) (op: {genmc_rmw_op:?}, rhs value: {genmc_rhs_scalar:?}), address: {address:?}, size: {size:?}, orderings: ({load_ordering:?}, {store_ordering:?})",
        );

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let rmw_result = pinned_mc.handleReadModifyWrite(
            genmc_tid.0,
            genmc_address,
            genmc_size,
            load_ordering,
            store_ordering,
            genmc_rmw_op,
            genmc_rhs_scalar,
            genmc_old_value,
        );

        if let Some(error) = rmw_result.error.as_ref() {
            let msg = error.to_string_lossy().to_string();
            info!("GenMC: RMW operation returned an error: \"{msg}\"");
            throw_ub_format!("{}", msg); // TODO GENMC: proper error handling: find correct error here
        }

        let old_value_scalar = genmc_scalar_to_scalar(ecx, rmw_result.old_value, size)?;

        let new_value_scalar = if rmw_result.isCoMaxWrite {
            Some(genmc_scalar_to_scalar(ecx, rmw_result.new_value, size)?)
        } else {
            None
        };
        interp_ok((old_value_scalar, new_value_scalar))
    }

    /**** Scheduling functionality ****/

    fn schedule_thread<'tcx>(
        &self,
        ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
    ) -> InterpResult<'tcx, ThreadId> {
        assert!(!self.allow_data_races.get()); // TODO GENMC: handle this properly
        let thread_manager = &ecx.machine.threads;
        let active_thread_id = thread_manager.active_thread();

        let curr_thread_next_instr_kind =
            if !thread_manager.active_thread_ref().get_state().is_enabled() {
                // The current thread can get blocked (e.g., due to a thread join, assume statement, ...), then we need to ask GenMC for another thread to schedule.
                // `Load` is a safe default for the next instruction type, since we may not know what the next instruction is.
                ActionKind::Load
            } else {
                let Some(frame) = thread_manager.get_thread_stack(active_thread_id).last() else {
                    return interp_ok(active_thread_id);
                };
                let either::Either::Left(loc) = frame.current_loc() else {
                    // We are unwinding.
                    return interp_ok(active_thread_id);
                };
                let basic_block = &frame.body().basic_blocks[loc.block];
                if let Some(_statement) = basic_block.statements.get(loc.statement_index) {
                    return interp_ok(active_thread_id);
                }

                if is_terminator_atomic(ecx, basic_block.terminator(), active_thread_id)? {
                    ActionKind::Load
                } else {
                    ActionKind::NonLoad
                }
            };

        info!(
            "GenMC: schedule_thread, active thread: {active_thread_id:?}, next instr.: '{curr_thread_next_instr_kind:?}'"
        );

        // let curr_thread_user_block = self.curr_thread_user_block.replace(false);
        let thread_infos = self.thread_infos.borrow();
        let curr_thread_info = thread_infos.get_info(active_thread_id);

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let result =
            pinned_mc.scheduleNext(curr_thread_info.genmc_tid.0, curr_thread_next_instr_kind);
        if result >= 0 {
            // TODO GENMC: can we ensure this thread_id is valid?
            let genmc_next_thread_id = result.try_into().unwrap();
            let genmc_next_thread_id = GenmcThreadId(genmc_next_thread_id);
            let thread_infos = self.thread_infos.borrow();
            let next_thread_id = thread_infos.get_info_genmc(genmc_next_thread_id).miri_tid;

            return interp_ok(next_thread_id);
        }

        // Negative result means that GenMC has no next thread to schedule.
        info!("GenMC: scheduleNext returned no thread to schedule, execution is finished.");
        throw_machine_stop!(TerminationInfo::GenmcFinishedExecution);
    }

    /**** Blocking functionality ****/

    fn handle_user_block<'tcx>(&self, machine: &MiriMachine<'tcx>) -> InterpResult<'tcx> {
        let thread_infos = self.thread_infos.borrow();
        let curr_thread = machine.threads.active_thread();
        let genmc_curr_thread = thread_infos.get_info(curr_thread).genmc_tid;
        info!("GenMC: handle_user_block, blocking thread {curr_thread:?} ({genmc_curr_thread:?})");

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        pinned_mc.handleUserBlock(genmc_curr_thread.0);

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

            let thread_infos = genmc_ctx.thread_infos.borrow();
            let curr_thread = this.machine.threads.active_thread();
            let genmc_curr_thread = thread_infos.get_info(curr_thread).genmc_tid;
            interp_ok((genmc_curr_thread, addr, 1))
        };

        use rustc_span::sym;
        if this.tcx.is_diagnostic_item(sym::sys_mutex_lock, instance.def_id()) {
            info!("GenMC: handling Mutex::lock()");
            let (genmc_curr_thread, addr, size) = get_mutex_call_infos()?;

            let result = {
                let mut mc = genmc_ctx.handle.borrow_mut();
                let pinned_mc = mc.as_mut();
                pinned_mc.handleMutexLock(genmc_curr_thread.0, addr, size)
            };
            if let Some(error) = result.error.as_ref() {
                let msg = error.to_string_lossy().to_string();
                info!("GenMC: handling Mutex::lock: error: {msg:?}");
                throw_ub_format!("{msg}");
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
                                let msg = error.to_string_lossy().to_string();
                                info!("GenMC: handling Mutex::lock: error: {msg:?}");
                                throw_ub_format!("{msg}");
                            }
                            // TODO GENMC(HACK): for determining if the Mutex lock blocks this thread.
                            if !result.is_lock_acquired {
                                // If this thread gets woken up without the mutex being made available, block the thread again.
                                this.block_thread( crate::BlockReason::Mutex, None, create_callback(genmc_curr_thread, addr, size));
                                // panic!(
                                //     "Somehow, Mutex is still locked after waiting thread was unblocked?!"
                                // );
                            }
                            interp_ok(())
                        }
                    )
                }

                info!("GenMC: handling Mutex::lock failed, blocking thread");
                // NOTE: We don't write anything back to Miri's memory, the Mutex state is handled only by GenMC.

                info!("GenMC: blocking thread due to intercepted call.");
                let genmc_curr_thread = genmc_curr_thread.0;
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
                pinned_mc.handleMutexTryLock(genmc_curr_thread.0, addr, size)
            };
            if let Some(error) = result.error.as_ref() {
                let msg = error.to_string_lossy().to_string();
                info!("GenMC: handling Mutex::try_lock: error: {msg:?}");
                throw_ub_format!("{msg}");
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
            let result = pinned_mc.handleMutexUnlock(genmc_curr_thread.0, addr, size);
            if let Some(error) = result.error.as_ref() {
                let msg = error.to_string_lossy().to_string();
                throw_ub_format!("{msg}");
            }
            // NOTE: We don't write anything back to Miri's memory, the Mutex state is handled only by GenMC.

            // this.unblock_thread(, crate::BlockReason::Mutex)?;
        } else {
            return interp_ok(false);
        };

        this.return_to_block(ret)?;

        interp_ok(true)
    }

    /**** Scheduling functionality ****/

    /// Ask for a scheduling decision. This should be called before every MIR instruction.
    ///
    /// GenMC may realize that the execution is blocked, then this function will return a `InterpErrorKind::MachineStop` with error kind `TerminationInfo::GenmcBlockedExecution`).
    ///
    /// This is **not** an error by iself! Treat this as if the program ended normally: `handle_execution_end` should be called next, which will determine if were are any actual errors.
    fn genmc_schedule_thread(&mut self) -> InterpResult<'tcx, ThreadId> {
        let this = self.eval_context_mut();
        loop {
            let genmc_ctx = this.machine.data_race.as_genmc_ref().unwrap();
            let next_thread_id = genmc_ctx.schedule_thread(this)?;

            match this.machine.threads.threads_ref()[next_thread_id].get_state() {
                ThreadState::Blocked {
                    reason: block_reason @ (BlockReason::Mutex | BlockReason::GenmcAssume),
                    ..
                } => {
                    info!(
                        "GenMC: schedule returned thread {next_thread_id:?}, which is blocked, so we unblock it now."
                    );
                    this.unblock_thread(next_thread_id, *block_reason)?;

                    // In some cases, like waiting on a Mutex::lock, the thread might still be blocked here:
                    if this.machine.threads.threads_ref()[next_thread_id]
                        .get_state()
                        .is_blocked_on(crate::BlockReason::Mutex)
                    {
                        info!("GenMC: Unblocked thread is blocked on a Mutex again!");
                        continue;
                    }
                }
                _ => {}
            }

            return interp_ok(next_thread_id);
        }
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

impl std::fmt::Debug for GenmcCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenmcCtx")
            // .field("mc", &self.mc)
            .field("thread_infos", &self.thread_infos)
            .finish_non_exhaustive()
    }
}
