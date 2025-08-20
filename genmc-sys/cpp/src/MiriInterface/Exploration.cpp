/** This file contains functionality related to exploration, such as scheduling.  */

#include "MiriInterface.hpp"

// Miri C++ helpers:
#include "Helper.hpp"

// CXX.rs generated headers:
#include "genmc-sys/src/lib.rs.h"

// GenMC headers:
#include "Support/Error.hpp"
#include "Support/Verbosity.hpp"

// C++ headers:
#include <cstdint>

auto MiriGenMCShim::scheduleNext(const int curr_thread_id,
				 const ActionKind curr_thread_next_instr_kind) -> int64_t
{
	// The current thread is the only one where the `kind` could have changed since we last made
	// a scheduling decision.
	threadsAction[curr_thread_id].kind = curr_thread_next_instr_kind;

	if (const auto result = GenMCDriver::scheduleNext(threadsAction))
		return static_cast<int64_t>(result.value());
	return -1;
}

/**** Execution start/end handling ****/

void MiriGenMCShim::handleExecutionStart()
{
	threadsAction.clear();
	threadsAction.push_back(Action(ActionKind::Load, Event::getInit()));
	GenMCDriver::handleExecutionStart();
}

auto MiriGenMCShim::handleExecutionEnd() -> std::unique_ptr<ModelCheckerError>
{
	GenMCDriver::handleExecutionEnd(threadsAction);
	// TODO GENMC: check if an error happened here?
	// ERROR_ON(isHalting(),
	// 	 "handleExecutionEnd found an error, but there is no error handling for that yet.");
	return {};
}