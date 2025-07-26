/** This file contains functionality related to handling events encountered
 * during an execution, such as loads, stores or memory (de)allocation. */

#include "MiriInterface.hpp"

// Miri C++ helpers:
#include "Helper.hpp"

// CXX.rs generated headers:
#include "genmc-sys/src/lib.rs.h"

// GenMC headers:
#include "ADT/value_ptr.hpp"
#include "ExecutionGraph/EventLabel.hpp"
#include "ExecutionGraph/LoadAnnotation.hpp"
#include "Runtime/InterpreterEnumAPI.hpp"
#include "Static/ModuleID.hpp"
#include "Support/ASize.hpp"
#include "Support/Error.hpp"
#include "Support/Logger.hpp"
#include "Support/MemAccess.hpp"
#include "Support/RMWOps.hpp"
#include "Support/SAddr.hpp"
#include "Support/SVal.hpp"
#include "Support/ThreadInfo.hpp"
#include "Support/Verbosity.hpp"
#include "Verification/GenMCDriver.hpp"
#include "Verification/MemoryModel.hpp"

// C++ headers:
#include <cstddef>
#include <cstdint>
#include <memory>
#include <utility>

/**** Memory access handling ****/

[[nodiscard]] auto MiriGenMCShim::handleLoad(ThreadId thread_id, uint64_t address, uint64_t size,
					     MemOrdering ord, GenmcScalar old_val) -> LoadResult
{
	const auto addr = SAddr(address);
	const auto aSize = ASize(size);
	// `type` is only used for printing.
	const auto type = AType::Unsigned;

	const auto oldValSetter = [this, old_val](SAddr addr) {
		this->handleOldVal(addr, old_val);
	};
	const auto ret = handleLoadResetIfNone<EventLabel::EventLabelKind::Read>(
		oldValSetter, thread_id, ord, addr, aSize, type);
	RETURN_IF_ERROR(ret, LoadResult);

	const auto *retVal = std::get_if<SVal>(&ret);
	if (retVal != nullptr)
		return LoadResult::fromValue(*retVal);

	ERROR("Unimplemented: load returned unexpected result.");
}

[[nodiscard]] auto MiriGenMCShim::handleStore(ThreadId thread_id, uint64_t address, uint64_t size,
					      GenmcScalar value, GenmcScalar old_val,
					      MemOrdering ord) -> StoreResult
{
	auto pos = incPos(thread_id);

	auto addr = SAddr(address);
	auto aSize = ASize(size);
	// `type` is only used for printing.
	auto type = AType::Unsigned;

	auto val = value.toSVal();

	auto oldValSetter = [this, old_val](SAddr addr) {
		this->handleOldVal(addr,
				   old_val); // TODO GENMC(HACK): is this the correct way to do it?
	};

	const auto ret = GenMCDriver::handleStore<EventLabel::EventLabelKind::Write>(
		oldValSetter, pos, ord, addr, aSize, type, val, EventDeps());

	RETURN_IF_ERROR(ret, StoreResult);

	if (!std::holds_alternative<std::monostate>(ret)) {
		ERROR("store returned unexpected result");
	}

	// TODO GENMC(mixed-accesses): calculate this value
	const auto &g = getExec().getGraph();
	const bool isCoMaxWrite = g.co_max(addr)->getPos() == pos;
	return StoreResult::ok(isCoMaxWrite);
}

/**** Memory (de)allocation ****/

auto MiriGenMCShim::handleMalloc(ThreadId thread_id, uint64_t size, uint64_t alignment) -> uint64_t
{
	auto pos = incPos(thread_id);

	// These are only used for printing and features Miri-GenMC doesn't support (yet).
	auto sd = StorageDuration::SD_Heap;
	auto stype = StorageType::ST_Volatile;
	auto spc = AddressSpace::AS_User;

	const SVal retVal =
		GenMCDriver::handleMalloc(pos, size, alignment, sd, stype, spc, EventDeps());
	return retVal.get();
}

void MiriGenMCShim::handleFree(ThreadId thread_id, uint64_t address)
{
	const auto pos = incPos(thread_id);
	const auto addr = SAddr(address);
	GenMCDriver::handleFree(pos, addr, EventDeps());
}
