use std::fmt::Display;
use std::rc::Rc;
use std::time::Instant;

use crate::{GenmcConfig, GenmcCtx, MiriConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Estimation,
    Verification,
}

impl Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Mode::Estimation => "Estimation",
            Mode::Verification => "Verification",
        })
    }
}

pub fn run_genmc_mode(
    config: &MiriConfig,
    genmc_config: &GenmcConfig,
    eval_entry: impl Fn(Rc<GenmcCtx>) -> Option<i32>,
    mode: Mode,
) -> Option<i32> {
    let time_start = Instant::now();
    let genmc_ctx = Rc::new(GenmcCtx::new(config, genmc_config, mode));

    for rep in 0u64.. {
        tracing::info!("Miri-GenMC loop {}", rep + 1);
        let result = eval_entry(genmc_ctx.clone());

        if genmc_config.print_exec_graphs() {
            genmc_ctx.print_genmc_graph();
        }

        // TODO GENMC (ERROR REPORTING): we currently do this here, so we can still print the GenMC graph above
        let return_code = result?;

        let is_exploration_done = genmc_ctx.is_exploration_done();

        tracing::info!(
            "(GenMC Mode) Execution done (return code: {return_code}), is_exploration_done: {is_exploration_done}",
        );

        if is_exploration_done {
            eprintln!();
            eprintln!("(GenMC) {mode} complete. No errors were detected.",);

            if mode == Mode::Estimation && return_code == 0 {
                let elapsed_time = Instant::now().duration_since(time_start);
                genmc_ctx.print_estimation_result(elapsed_time);
                return Some(0);
            }

            // TODO GENMC: proper message here, which info should be printed?
            let blocked_execution_count = genmc_ctx.get_blocked_execution_count();
            // TODO GENMC: use VerificationResult instead:
            let explored_execution_count = rep + 1 - blocked_execution_count;
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
