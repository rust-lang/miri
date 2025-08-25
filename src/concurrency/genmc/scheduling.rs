use genmc_sys::{ActionKind, ExecutionState};

use super::GenmcCtx;
use crate::{
    InterpCx, InterpResult, MiriMachine, TerminationInfo, ThreadId, interp_ok, throw_machine_stop,
};

impl GenmcCtx {
    pub(crate) fn schedule_thread<'tcx>(
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

                // FIXME(genmc): determine terminator kind.
                ActionKind::Load
            };

        let thread_infos = self.exec_state.thread_id_manager.borrow();
        let curr_thread_info = thread_infos.get_genmc_tid(active_thread_id);

        let mut mc = self.handle.borrow_mut();
        let pinned_mc = mc.as_mut().unwrap();
        let result = pinned_mc.scheduleNext(curr_thread_info, curr_thread_next_instr_kind);
        // Depending on the exec_state, we either schedule the given thread, or we are finished with this execution.
        match result.exec_state {
            ExecutionState::Ok =>
                interp_ok(
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
