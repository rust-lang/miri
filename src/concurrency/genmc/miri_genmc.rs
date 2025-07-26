use std::rc::Rc;

use crate::{GenmcCtx, MiriConfig};

pub fn run_genmc_mode(
    config: &MiriConfig,
    eval_entry: impl Fn(Rc<GenmcCtx>) -> Option<i32>,
) -> Option<i32> {
    let genmc_ctx = Rc::new(GenmcCtx::new(config));

    for rep in 0u64.. {
        tracing::info!("Miri-GenMC loop {}", rep + 1);
        let result = eval_entry(genmc_ctx.clone());

        // TODO GENMC (ERROR REPORTING): we currently do this here, so we can still print the GenMC graph above
        let return_code = result?;

        let is_exploration_done = genmc_ctx.is_exploration_done();

        tracing::info!(
            "(GenMC Mode) Execution done (return code: {return_code}), is_exploration_done: {is_exploration_done}",
        );

        if is_exploration_done {
            eprintln!("(GenMC) Verification complete. No errors were detected.",);

            let explored_execution_count = genmc_ctx.get_explored_execution_count();
            let blocked_execution_count = genmc_ctx.get_blocked_execution_count();

            eprintln!("Number of complete executions explored: {explored_execution_count}");
            if blocked_execution_count > 0 {
                eprintln!("Number of blocked executions seen: {blocked_execution_count}");
            }

            // TODO GENMC: what is an appropriate return code? (since there are possibly many)
            return Some(return_code);
        }
    }
    tracing::error!("GenMC mode did not finish in 2^64 iterations!");
    None
}
