use std::rc::Rc;

use rustc_middle::ty::TyCtxt;

use crate::rustc_const_eval::interpret::PointerArithmetic;
use crate::{GenmcCtx, MiriConfig};

/// Do a complete run of the program in GenMC mode.
/// This will call `eval_entry` multiple times, until either:
/// - An error is detected (indicated by a `None` return value)
/// - All possible executions are explored.
///
/// FIXME(genmc): add estimation mode setting.
pub fn run_genmc_mode<'tcx>(
    config: &MiriConfig,
    eval_entry: impl Fn(Rc<GenmcCtx>) -> Option<i32>,
    tcx: TyCtxt<'tcx>,
) -> Option<i32> {
    let target_usize_max = tcx.target_usize_max();
    let genmc_ctx = Rc::new(GenmcCtx::new(config, target_usize_max));

    // `rep` is used to report the progress, Miri will panic on wrap-around.
    for rep in 0u64.. {
        tracing::info!("Miri-GenMC loop {}", rep + 1);

        // Execute the program until completion to get the return value, or return if an error happens:
        // FIXME(genmc): add an option to allow the user to see the GenMC output message when the verification is done.
        let return_code = eval_entry(genmc_ctx.clone())?;

        // Some errors are not returned immediately during execution, so check for them here:
        if let Some(error) = genmc_ctx.try_get_error() {
            // Since we don't have any span information for the error at this point,
            // or the error is about the entire execution, we print GenMC's error message to give at least some feedback.
            eprintln!("(GenMC) Error detected: {error}");
            eprintln!();
            eprintln!("{}", genmc_ctx.get_result_message());
            return None;
        }

        // Check if we've explored enough executions:
        if !genmc_ctx.is_exploration_done() {
            continue;
        }

        eprintln!("(GenMC) Verification complete. No errors were detected.");

        let explored_execution_count = genmc_ctx.get_explored_execution_count();
        let blocked_execution_count = genmc_ctx.get_blocked_execution_count();

        eprintln!("Number of complete executions explored: {explored_execution_count}");
        if blocked_execution_count > 0 {
            eprintln!("Number of blocked executions seen: {blocked_execution_count}");
        }

        // Return the return code of the last execution.
        return Some(return_code);
    }
    unreachable!()
}
