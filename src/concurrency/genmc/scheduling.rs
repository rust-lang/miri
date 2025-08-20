use genmc_sys::{ActionKind, ExecutionState};
use rustc_middle::mir::{Terminator, TerminatorKind};
use rustc_middle::ty::{self, Ty};
use tracing::info;

use super::GenmcCtx;
use crate::concurrency::thread::{EvalContextExt as _, ThreadState};
use crate::{
    BlockReason, InterpCx, InterpResult, MiriMachine, TerminationInfo, ThreadId, interp_ok,
    throw_machine_stop,
};

/// Check if a MIR terminator could be an atomic load operation.
/// Currently this check is very conservative; all atomics are seen as possibly being loads.
/// NOTE: This function panics if called with a thread that is not currently the active one.
fn is_terminator_atomic_load<'tcx>(
    ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
    terminator: &Terminator<'tcx>,
    thread_id: ThreadId,
) -> InterpResult<'tcx, bool> {
    assert_eq!(
        thread_id,
        ecx.machine.threads.active_thread(),
        "Can only call this function on the active thread."
    );
    match &terminator.kind {
        // All atomics are modeled as function calls to intrinsic functions.
        // The one exception is thread joining, but those are also calls.
        TerminatorKind::Call { func, .. } | TerminatorKind::TailCall { func, .. } => {
            let frame = ecx.machine.threads.active_thread_stack().last().unwrap();
            let func_ty = func.ty(&frame.body().local_decls, *ecx.tcx);
            info!("GenMC: terminator is a call with operand: {func:?}, ty of operand: {func_ty:?}");

            has_function_atomic_load_semantics(ecx, func_ty)
        }
        _ => interp_ok(false),
    }
}

/// Check if a call or tail-call could have atomic load semantics.
fn has_function_atomic_load_semantics<'tcx>(
    ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
    func_ty: Ty<'tcx>,
) -> InterpResult<'tcx, bool> {
    let callee_def_id = match func_ty.kind() {
        ty::FnDef(def_id, _args) => def_id,
        _ => return interp_ok(true), // we don't know the callee, might be an intrinsic or pthread_join
    };
    if ecx.tcx.is_foreign_item(*callee_def_id) {
        // Some shims, like pthread_join, must be considered loads. So just consider them all loads,
        // these calls are not *that* common.
        return interp_ok(true);
    }

    let Some(intrinsic_def) = ecx.tcx.intrinsic(callee_def_id) else {
        // FIXME(genmc): Make this work for other platforms.
        let item_name = ecx.tcx.item_name(*callee_def_id);
        return interp_ok(matches!(item_name.as_str(), "pthread_join" | "WaitForSingleObject"));
    };
    let intrinsice_name = intrinsic_def.name.as_str();
    info!("GenMC:   intrinsic name: '{intrinsice_name}'");
    // FIXME(genmc): make this more precise (only loads). How can we make this maintainable?
    interp_ok(intrinsice_name.starts_with("atomic_"))
}

impl GenmcCtx {
    fn schedule_thread<'tcx>(
        &self,
        ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
    ) -> InterpResult<'tcx, ThreadId> {
        let thread_manager = &ecx.machine.threads;
        let active_thread_id = thread_manager.active_thread();

        // Determine whether the next instruction in the current thread might be a load.
        let curr_thread_next_instr_kind =
            if !thread_manager.active_thread_ref().get_state().is_enabled() {
                // The current thread can get blocked (e.g., due to a thread join, assume statement, ...), then we need to ask GenMC for another thread to schedule.
                // `Load` is a safe default for the next instruction type, since we may not know what the next instruction is.
                ActionKind::Load
            } else {
                let Some(frame) = thread_manager.active_thread_stack().last() else {
                    return interp_ok(active_thread_id);
                };
                let either::Either::Left(loc) = frame.current_loc() else {
                    // We are unwinding, so the next step is definitely not atomic.
                    return interp_ok(active_thread_id);
                };
                let basic_block = &frame.body().basic_blocks[loc.block];
                if let Some(_statement) = basic_block.statements.get(loc.statement_index) {
                    // Statements can't be atomic.
                    return interp_ok(active_thread_id);
                }

                if is_terminator_atomic_load(ecx, basic_block.terminator(), active_thread_id)? {
                    ActionKind::Load
                } else {
                    ActionKind::NonLoad
                }
            };

        info!(
            "GenMC: schedule_thread, active thread: {active_thread_id:?}, next instr.: '{curr_thread_next_instr_kind:?}'"
        );

        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let curr_thread_info = thread_infos.get_genmc_tid(active_thread_id);

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut();
        let result = pinned_mc.scheduleNext(curr_thread_info, curr_thread_next_instr_kind);
        // Depending on the exec_state, we either schedule the given thread, or we are finished with this execution.
        match result.exec_state {
            ExecutionState::Ok =>
                return interp_ok(
                    thread_infos
                        .try_get_miri_tid(result.next_thread)
                        .expect("A thread id returned from GenMC should exist."),
                ),
            ExecutionState::Blocked => throw_machine_stop!(TerminationInfo::GenmcBlockedExecution),
            ExecutionState::Finished => {
                let exit_status = self.exec_state.exit_status.get().expect(
                    "If the execution is finished, we should have a return value from the program.",
                );
                let leak_check = exit_status.do_leak_check();
                throw_machine_stop!(TerminationInfo::Exit {
                    code: exit_status.exit_code,
                    leak_check
                });
            }
            _ => unreachable!(),
        }
    }
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
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
}
