use rustc_middle::throw_unsup_format;
use tracing::debug;

use crate::concurrency::thread::EvalContextExt as _;
use crate::{
    BlockReason, InterpResult, MachineCallback, MiriInterpCx, OpTy, Scalar, UnblockKind,
    VisitProvenance, VisitWith, callback, interp_ok, throw_ub_format,
};

// Handling of code intercepted by Miri in GenMC mode, such as assume statement or `std::sync::Mutex`.

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    /// Given a `ty::Instance<'tcx>`, do any required special handling.
    /// Returns true if this `instance` should be skipped (i.e., no MIR should be executed for it).
    fn genmc_intercept_function(
        &mut self,
        instance: rustc_middle::ty::Instance<'tcx>,
        args: &[rustc_const_eval::interpret::FnArg<'tcx, crate::Provenance>],
        dest: &crate::PlaceTy<'tcx>,
    ) -> InterpResult<'tcx, bool> {
        let this = self.eval_context_mut();
        let genmc_ctx = this
            .machine
            .data_race
            .as_genmc_ref()
            .expect("This function should only be called in GenMC mode.");

        let get_mutex_call_infos = || {
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
        interp_ok(if this.tcx.is_diagnostic_item(sym::sys_mutex_lock, instance.def_id()) {
            debug!("GenMC: handling Mutex::lock()");
            let (genmc_curr_thread, addr, size) = get_mutex_call_infos()?;

            let result = genmc_ctx.handle.borrow_mut().pin_mut().handle_mutex_lock(
                genmc_curr_thread,
                addr,
                size,
            );
            if let Some(error) = result.error.as_ref() {
                // FIXME(genmc): improve error handling.
                throw_ub_format!("{}", error.to_string_lossy());
            }
            if result.is_lock_acquired {
                debug!("GenMC: handling Mutex::lock: success: lock acquired.");
                return interp_ok(true);
            }
            debug!("GenMC: handling Mutex::lock failed, blocking thread");
            // NOTE: We don't write anything back to Miri's memory, the Mutex state is handled only by GenMC.

            this.block_thread(
                crate::BlockReason::Genmc,
                None,
                crate::callback!(
                    @capture<'tcx> {
                        genmc_curr_thread: i32,
                        addr: u64,
                        size: u64,
                    }
                    |this, unblock: crate::UnblockKind| {
                        debug!("GenMC: handling Mutex::lock: unblocking callback called.");
                        assert_eq!(unblock, crate::UnblockKind::Ready);
                        let genmc_ctx = this.machine.data_race.as_genmc_ref().unwrap();
                        let result = genmc_ctx.handle
                            .borrow_mut()
                            .pin_mut()
                            .handle_mutex_lock(genmc_curr_thread, addr, size);
                        if let Some(error) = result.error.as_ref() {
                            // FIXME(genmc): improve error handling.
                            throw_ub_format!("{}", error.to_string_lossy());
                        }
                        // FIXME(genmc): The reported error message is bad, it does not point to the second lock call involved in the deadlock.
                        // FIXME(genmc): there can be cases where not acquiring a mutex after the second attempt is *not* a deadlock. Reliably detecting deadlocks requires extra analysis (in GenMC).
                        if !result.is_lock_acquired {
                            throw_unsup_format!("Could not lock Mutex, which may indicate a deadlock. (GenMC mode does not fully support deadlock detection yet).")
                        }
                        interp_ok(())
                    }
                ),
            );
            // NOTE: We don't write anything back to Miri's memory where the Mutex is located, that state is handled only by GenMC.
            true
        } else if this.tcx.is_diagnostic_item(sym::sys_mutex_try_lock, instance.def_id()) {
            debug!("GenMC: handling Mutex::try_lock()");
            let (genmc_curr_thread, addr, size) = get_mutex_call_infos()?;

            let result = genmc_ctx.handle.borrow_mut().pin_mut().handle_mutex_try_lock(
                genmc_curr_thread,
                addr,
                size,
            );
            if let Some(error) = result.error.as_ref() {
                // FIXME(genmc): improve error handling.
                throw_ub_format!("{}", error.to_string_lossy());
            }
            debug!("GenMC: Mutex::try_lock(): is_lock_acquired: {}", result.is_lock_acquired);
            // Write the return value of the intercepted function.
            // NOTE: We don't write anything back to Miri's memory where the Mutex is located, that state is handled only by GenMC.
            this.write_scalar(Scalar::from_bool(result.is_lock_acquired), dest)?;
            true
        } else if this.tcx.is_diagnostic_item(sym::sys_mutex_unlock, instance.def_id()) {
            debug!("GenMC: handling Mutex::unlock()");
            let (genmc_curr_thread, addr, size) = get_mutex_call_infos()?;

            let result = genmc_ctx.handle.borrow_mut().pin_mut().handle_mutex_unlock(
                genmc_curr_thread,
                addr,
                size,
            );
            if let Some(error) = result.error.as_ref() {
                // FIXME(genmc): improve error handling.
                throw_ub_format!("{}", error.to_string_lossy());
            }
            // NOTE: We don't write anything back to Miri's memory where the Mutex is located, that state is handled only by GenMC.
            true
        } else {
            // Nothing to intercept.
            false
        })
    }

    /// Handle an `assume` statement. This will tell GenMC to block the current thread if the `condition` is false.
    /// Returns `true` if the current thread should be blocked in Miri too.
    fn handle_genmc_verifier_assume(&mut self, condition: &OpTy<'tcx>) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let condition_bool = this.read_scalar(condition)?.to_bool()?;
        debug!("GenMC: handle_genmc_verifier_assume, condition: {condition:?} = {condition_bool}");
        if condition_bool {
            return interp_ok(());
        }
        let genmc_ctx = this.machine.data_race.as_genmc_ref().unwrap();
        genmc_ctx.handle_assume_block(&this.machine)?;
        this.block_thread(
            BlockReason::Genmc,
            None,
            callback!(
                @capture<'tcx> {}
                |_this, unblock: UnblockKind| {
                    assert_eq!(unblock, UnblockKind::Ready);
                    unreachable!("GenMC should never unblock a thread blocked by an `assume`.");
                }
            ),
        );
        interp_ok(())
    }
}
