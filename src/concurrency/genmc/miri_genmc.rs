use std::rc::Rc;

use rustc_middle::ty::TyCtxt;

use crate::rustc_const_eval::interpret::PointerArithmetic;
use crate::{GenmcCtx, MiriConfig};

/// Do a complete run of the program in GenMC mode.
/// This will call `eval_entry` multiple times, until either:
/// - An error is detected
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

    for rep in 0u64.. {
        tracing::info!("Miri-GenMC loop {}", rep + 1);

        // Execute the program until completion to get the return value, or return if an error happens:
        let return_code = eval_entry(genmc_ctx.clone())?;

        // Some errors are not returned immediately during execution, so check for them here:
        if let Some(error) = genmc_ctx.try_get_error() {
            eprintln!("(GenMC) Error detected: {error}");
            eprintln!();
            // FIXME(genmc): we may want to print this message every time we finish the verification/find an error.
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
    tracing::error!("GenMC mode did not finish in 2^64 iterations!");
    None
}
