use rustc_middle::throw_unsup_format;
use tracing::debug;

use crate::concurrency::genmc::MAX_ACCESS_SIZE;
use crate::concurrency::thread::EvalContextExt as _;
use crate::{
    BlockReason, InterpResult, MachineCallback, MiriInterpCx, OpTy, Scalar, UnblockKind,
    VisitProvenance, VisitWith, callback, interp_ok, throw_ub_format,
};

// Handling of code intercepted by Miri in GenMC mode, such as assume statement or `std::sync::Mutex`.

#[derive(Clone, Copy)]
struct MutexMethodArgs {
    address: u64,
    size: u64,
}

impl<'tcx> EvalContextExtPriv<'tcx> for crate::MiriInterpCx<'tcx> {}
trait EvalContextExtPriv<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn parse_mutex_method_args(
        &self,
        args: &[rustc_const_eval::interpret::FnArg<'tcx, crate::Provenance>],
    ) -> InterpResult<'tcx, MutexMethodArgs> {
        assert_eq!(args.len(), 1, "Mutex lock/unlock/try_lock should take exactly 1 argument.");
        let this = self.eval_context_ref();
        let arg = this.copy_fn_arg(&args[0]);
        // FIXME(genmc): use actual size of the pointee of `arg`.
        let size = 1;
        // GenMC does not support large accesses, we limit the size to the maximum access size.
        interp_ok(MutexMethodArgs {
            address: this.read_target_usize(&arg)?,
            size: size.min(MAX_ACCESS_SIZE),
        })
    }

    fn intercept_mutex_lock(&mut self, args: MutexMethodArgs) -> InterpResult<'tcx> {
        debug!("GenMC: handling Mutex::lock()");
        let MutexMethodArgs { address, size } = args;
        let this = self.eval_context_mut();
        let genmc_ctx = this.machine.data_race.as_genmc_ref().unwrap();
        let genmc_tid = genmc_ctx.active_thread_genmc_tid(&this.machine);
        let result =
            genmc_ctx.handle.borrow_mut().pin_mut().handle_mutex_lock(genmc_tid, address, size);
        if let Some(error) = result.error.as_ref() {
            // FIXME(genmc): improve error handling.
            throw_ub_format!("{}", error.to_string_lossy());
        }
        if result.is_lock_acquired {
            debug!("GenMC: handling Mutex::lock: success: lock acquired.");
            return interp_ok(());
        }
        debug!("GenMC: handling Mutex::lock failed, blocking thread");
        // NOTE: We don't write anything back to Miri's memory, the Mutex state is handled only by GenMC.

        this.block_thread(
                crate::BlockReason::Genmc,
                None,
                crate::callback!(
                    @capture<'tcx> {
                        genmc_tid: i32,
                        address: u64,
                        size: u64,
                    }
                    |this, unblock: crate::UnblockKind| {
                        debug!("GenMC: handling Mutex::lock: unblocking callback called.");
                        assert_eq!(unblock, crate::UnblockKind::Ready);
                        let genmc_ctx = this.machine.data_race.as_genmc_ref().unwrap();
                        let result = genmc_ctx.handle
                            .borrow_mut()
                            .pin_mut()
                            .handle_mutex_lock(genmc_tid, address, size);
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
        interp_ok(())
    }

    fn intercept_mutex_try_lock(
        &mut self,
        args: MutexMethodArgs,
        dest: &crate::PlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        debug!("GenMC: handling Mutex::try_lock()");
        let this = self.eval_context_mut();
        let genmc_ctx = this.machine.data_race.as_genmc_ref().unwrap();
        let result = genmc_ctx.handle.borrow_mut().pin_mut().handle_mutex_try_lock(
            genmc_ctx.active_thread_genmc_tid(&this.machine),
            args.address,
            args.size,
        );
        if let Some(error) = result.error.as_ref() {
            // FIXME(genmc): improve error handling.
            throw_ub_format!("{}", error.to_string_lossy());
        }
        debug!("GenMC: Mutex::try_lock(): is_lock_acquired: {}", result.is_lock_acquired);
        // Write the return value of try_lock, i.e., whether we acquired the mutex.
        this.write_scalar(Scalar::from_bool(result.is_lock_acquired), dest)?;
        // NOTE: We don't write anything back to Miri's memory where the Mutex is located, that state is handled only by GenMC.
        interp_ok(())
    }

    fn intercept_mutex_unlock(&self, args: MutexMethodArgs) -> InterpResult<'tcx> {
        debug!("GenMC: handling Mutex::unlock()");
        let this = self.eval_context_ref();
        let genmc_ctx = this.machine.data_race.as_genmc_ref().unwrap();
        let result = genmc_ctx.handle.borrow_mut().pin_mut().handle_mutex_unlock(
            genmc_ctx.active_thread_genmc_tid(&this.machine),
            args.address,
            args.size,
        );
        if let Some(error) = result.error.as_ref() {
            // FIXME(genmc): improve error handling.
            throw_ub_format!("{}", error.to_string_lossy());
        }
        // NOTE: We don't write anything back to Miri's memory where the Mutex is located, that state is handled only by GenMC.}
        interp_ok(())
    }
}

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
        assert!(
            this.machine.data_race.as_genmc_ref().is_some(),
            "This function should only be called in GenMC mode."
        );

        // NOTE: When adding new intercepted functions here, they must also be added to `fn get_function_kind` in `concurrency/genmc/scheduling.rs`.
        use rustc_span::sym;
        interp_ok(if this.tcx.is_diagnostic_item(sym::sys_mutex_lock, instance.def_id()) {
            this.intercept_mutex_lock(this.parse_mutex_method_args(args)?)?;
            true
        } else if this.tcx.is_diagnostic_item(sym::sys_mutex_try_lock, instance.def_id()) {
            this.intercept_mutex_try_lock(this.parse_mutex_method_args(args)?, dest)?;
            true
        } else if this.tcx.is_diagnostic_item(sym::sys_mutex_unlock, instance.def_id()) {
            this.intercept_mutex_unlock(this.parse_mutex_method_args(args)?)?;
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
