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

/**** Blocking instructions ****/

void MiriGenMCShim::handleUserBlock(ThreadId thread_id)
{
	GenMCDriver::handleAssume(incPos(thread_id), AssumeType::User);
}

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

[[nodiscard]] auto MiriGenMCShim::handleReadModifyWrite(ThreadId thread_id, uint64_t address,
							uint64_t size, MemOrdering loadOrd,
							MemOrdering store_ordering, RMWBinOp rmw_op,
							GenmcScalar rhs_value, GenmcScalar old_val)
	-> ReadModifyWriteResult
{
	const auto addr = SAddr(address);
	const auto aSize = ASize(size);
	// `type` is only used for printing.
	const auto type = AType::Unsigned;

	const auto rhsVal = rhs_value.toSVal();

	const auto oldValSetter = [this, old_val](SAddr addr) {
		this->handleOldVal(addr, old_val);
	};
	const auto ret = handleLoadResetIfNone<EventLabel::EventLabelKind::FaiRead>(
		oldValSetter, thread_id, loadOrd, addr, aSize, type, rmw_op, rhsVal, EventDeps());
	RETURN_IF_ERROR(ret, ReadModifyWriteResult);

	const auto *retVal = std::get_if<SVal>(&ret);
	if (nullptr == retVal) {
		ERROR("Unimplemented: read-modify-write returned unhandled result.");
	}
	const auto oldVal = *retVal;
	const auto newVal = executeRMWBinOp(oldVal, rhsVal, size, rmw_op);

	const auto storePos = incPos(thread_id);
	const auto storeRet = GenMCDriver::handleStore<EventLabel::EventLabelKind::FaiWrite>(
		oldValSetter, storePos, store_ordering, addr, aSize, type, newVal);
	RETURN_IF_ERROR(storeRet, ReadModifyWriteResult);

	const auto *storeRetVal = std::get_if<SVal>(&ret);
	ERROR_ON(nullptr == storeRetVal, "Unimplemented: load returned unexpected result.");

	// TODO GENMC(mixed-accesses): calculate this value
	const auto &g = getExec().getGraph();
	const bool isCoMaxWrite = g.co_max(addr)->getPos() == storePos;
	LOG(VerbosityLevel::Tip) << "TODO GENMC: calcuate isCoMaxWrite!!!\n";
	return ReadModifyWriteResult(oldVal, newVal, isCoMaxWrite);
}

[[nodiscard]] auto MiriGenMCShim::handleCompareExchange(
	ThreadId thread_id, uint64_t address, uint64_t size, GenmcScalar expected_value,
	GenmcScalar new_value, GenmcScalar old_val, MemOrdering success_load_ordering,
	MemOrdering success_store_ordering, MemOrdering fail_load_ordering,
	bool can_fail_spuriously) -> CompareExchangeResult
{
	auto addr = SAddr(address);
	auto aSize = ASize(size);
	// `type` is only used for printing.
	auto type = AType::Unsigned;

	auto expectedVal = expected_value.toSVal();
	auto newVal = new_value.toSVal();

	// FIXME(GenMC): properly handle failure memory ordering.

	auto oldValSetter = [this, old_val](SAddr addr) { this->handleOldVal(addr, old_val); };
	const auto ret = handleLoadResetIfNone<EventLabel::EventLabelKind::CasRead>(
		oldValSetter, thread_id, success_load_ordering, addr, aSize, type, expectedVal,
		newVal);
	RETURN_IF_ERROR(ret, CompareExchangeResult);

	const auto *retVal = std::get_if<SVal>(&ret);
	ERROR_ON(nullptr == retVal, "Unimplemented: load returned unexpected result.");

	const auto oldVal = *retVal;
	if (oldVal != expectedVal)
		return CompareExchangeResult::failure(oldVal);

	// FIXME(GenMC): Add support for modelling spurious failures.

	const auto storePos = incPos(thread_id);
	const auto storeRet = GenMCDriver::handleStore<EventLabel::EventLabelKind::CasWrite>(
		oldValSetter, storePos, success_store_ordering, addr, aSize, type, newVal);
	RETURN_IF_ERROR(storeRet, CompareExchangeResult);

	const auto *storeRetVal = std::get_if<SVal>(&ret);
	ERROR_ON(nullptr == storeRetVal, "Unimplemented: load returned unexpected result.");

	// TODO GENMC(mixed-accesses): calculate this value
	const auto &g = getExec().getGraph();
	const bool isCoMaxWrite = g.co_max(addr)->getPos() == storePos;
	LOG(VerbosityLevel::Tip) << "TODO GENMC: calcuate isCoMaxWrite!!!\n";
	return CompareExchangeResult::success(oldVal, isCoMaxWrite);
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

void MiriGenMCShim::handleFence(ThreadId thread_id, MemOrdering ord)
{
	const auto pos = incPos(thread_id);
	GenMCDriver::handleFence(pos, ord, EventDeps());
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
