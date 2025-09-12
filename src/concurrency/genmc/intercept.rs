use tracing::debug;

use crate::concurrency::thread::EvalContextExt as _;
use crate::{
    BlockReason, InterpResult, MachineCallback, MiriInterpCx, OpTy, UnblockKind, VisitProvenance,
    VisitWith, callback, interp_ok,
};

// Handling of code intercepted by Miri in GenMC mode, such as assume statement or `std::sync::Mutex`.

/// Other functionality not directly related to event handling
impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
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
