
/** This file contains functionality related thread management (creation, finishing, join, etc.)  */

#include "MiriInterface.hpp"

// CXX.rs generated headers:
#include "genmc-sys/src/lib.rs.h"

// GenMC headers:
#include "Support/Error.hpp"
#include "Support/Verbosity.hpp"

// C++ headers:
#include <cstdint>

void MiriGenMCShim::handleThreadCreate(ThreadId thread_id, ThreadId parent_id) {
    // NOTE: The threadCreate event happens in the parent:
    auto pos = incPos(parent_id);

    // FIXME(genmc): for supporting symmetry reduction, these will need to be properly set:
    const unsigned fun_id = 0;
    const SVal arg = SVal(0);
    const ThreadInfo childInfo = ThreadInfo { thread_id, parent_id, fun_id, arg };

    // NOTE: Default memory ordering (`Release`) used here.
    auto child_tid = GenMCDriver::handleThreadCreate(pos, childInfo, EventDeps());
    // Sanity check the thread id. GenMC should respect the choice of thread id Miri made.
    BUG_ON(child_tid != thread_id || child_tid <= 0 || child_tid != threads_action_.size());
    threads_action_.push_back(Action(ActionKind::Load, Event(child_tid, 0)));
}

void MiriGenMCShim::handleThreadJoin(ThreadId thread_id, ThreadId child_id) {
    // The thread join event happens in the parent.
    auto pos = incPos(thread_id);

    // NOTE: Default memory ordering (`Acquire`) used here.
    auto ret = GenMCDriver::handleThreadJoin(pos, child_id, EventDeps());
    // If the join failed, decrease the event index again:
    if (!std::holds_alternative<SVal>(ret)) {
        decPos(thread_id);
    }

    // NOTE: Thread return value is ignored, since Miri doesn't need it.
}

void MiriGenMCShim::handleThreadFinish(ThreadId thread_id, uint64_t ret_val) {
    const auto pos = incPos(thread_id);
    // NOTE: Default memory ordering (`Release`) used here.
    GenMCDriver::handleThreadFinish(pos, SVal(ret_val));
}

void MiriGenMCShim::handleThreadKill(ThreadId thread_id) {
    const auto pos = incPos(thread_id);
    GenMCDriver::handleThreadKill(pos);
}
