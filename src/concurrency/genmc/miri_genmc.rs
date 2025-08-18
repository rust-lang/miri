use std::fmt::Display;
use std::rc::Rc;
use std::time::Instant;

use crate::{GenmcConfig, GenmcCtx, MiriConfig};

#[derive(Clone, Copy, PartialEq, Eq)]
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

/// Do a complete run of the program in GenMC mode.
/// This will call `eval_entry` multiple times, until either:
/// - An error is detected
/// - All possible executions are explored (in `Mode::Verification`)
/// - Enough executions are explored to estimated the total number of executions (in `Mode::Estimation`)
///
/// Returns `None` is an error is detected, or `Some(return_value)` with the return value of the last run of the program.
pub fn run_genmc_mode(
    config: &MiriConfig,
    genmc_config: &GenmcConfig,
    eval_entry: impl Fn(Rc<GenmcCtx>) -> Option<i32>,
    target_usize_max: u64,
    mode: Mode,
) -> Option<i32> {
    let time_start = Instant::now();
    let genmc_ctx = Rc::new(GenmcCtx::new(config, target_usize_max, mode));

    for rep in 0u64.. {
        tracing::info!("Miri-GenMC loop {}", rep + 1);

        // Execute the program until completion or an error happens:
        let result = eval_entry(genmc_ctx.clone());
        // We always print the graph when requested, even on errors.
        // This may not be needed if Miri makes use of GenMC's error message at some point, since it already includes the graph.
        // FIXME(genmc): Currently GenMC is missing some info from Miri to be able to fully print the execution graph.
        if genmc_config.print_exec_graphs() {
            genmc_ctx.print_genmc_graph();
        }
        // Return if there was an error, or get the return code of the program.
        let return_code = result?;

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

        eprintln!("(GenMC) {mode} complete. No errors were detected.",);

        if mode == Mode::Estimation && return_code == 0 {
            let elapsed_time = Instant::now().duration_since(time_start);
            genmc_ctx.print_estimation_result(elapsed_time);
            return Some(0);
        }

        let explored_execution_count = genmc_ctx.get_explored_execution_count();
        let blocked_execution_count = genmc_ctx.get_blocked_execution_count();

        eprintln!("Number of complete executions explored: {explored_execution_count}");
        if blocked_execution_count > 0 {
            eprintln!("Number of blocked executions seen: {blocked_execution_count}");
        }

        // TODO GENMC: what is an appropriate return code? (since there are possibly many)
        return Some(return_code);
    }
    tracing::error!("GenMC mode did not finish in 2^64 iterations!");
    None
}
