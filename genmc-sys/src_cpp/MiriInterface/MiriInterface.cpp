#include "MiriInterface.hpp"

// Miri C++ helpers:
#include "Helper.hpp"
#include "LogLevel.hpp"

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

// Return -1 when no thread can/should be scheduled, or the thread id of the next thread
// NOTE: this is safe because ThreadId is 32 bit, and we return a 64 bit integer
// FIXME(genmc,cxx): could directly return std::optional if CXX ever supports sharing it (see
// https://github.com/dtolnay/cxx/issues/87).
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

/**** Functions available to Miri ****/

// NOLINTNEXTLINE(readability-convert-member-functions-to-static)
auto MiriGenMCShim::createHandle(const GenmcParams &config, bool estimation_mode)
	-> std::unique_ptr<MiriGenMCShim>
{
	auto conf = std::make_shared<Config>();

	conf->skipNonAtomicInitializedCheck = true;

	// Miri needs all threads to be replayed, even fully completed ones.
	conf->replayCompletedThreads = true;

	// `1024` is the default value that GenMC uses.
	// If any thread has at least this many events, a warning/tip will be printed.
	//
	// Miri produces a lot more events than GenMC, so the graph size warning triggers on almost
	// all programs. The current value is large enough so the warning is not be triggered by any
	// reasonable programs.
	// FIXME(genmc): The emitted warning mentions features not supported by Miri ('--unroll'
	// parameter).
	// FIXME(genmc): A more appropriate limit should be chosen once the warning is useful for
	// Miri.
	conf->warnOnGraphSize = 1024 * 1024;

	// The `logLevel` is not part of the config struct, but the static variable `logLevel`.
	logLevel = to_genmc_verbosity_level(config.log_level);

	// We only support the RC11 memory model for Rust.
	conf->model = ModelType::RC11;

	conf->printRandomScheduleSeed = config.print_random_schedule_seed;

	// FIXME(genmc): check if we can enable IPR:
	conf->ipr = false;
	// FIXME(genmc): check if we can enable BAM:
	conf->disableBAM = true;
	// FIXME(genmc): check if we can do instruction caching (probably not)
	conf->instructionCaching = false;

	// FIXME(genmc): implement symmetry reduction.
	ERROR_ON(config.do_symmetry_reduction,
		 "Symmetry reduction is currently unsupported in GenMC mode.");
	conf->symmetryReduction = config.do_symmetry_reduction;

	// FIXME(genmc): expose this setting to Miri (useful for testing Miri-GenMC).
	conf->schedulePolicy = SchedulePolicy::WF;

	conf->estimate = estimation_mode;
	conf->estimationMax = config.estimation_max;
	const auto mode = conf->estimate ? GenMCDriver::Mode(GenMCDriver::EstimationMode{})
					 : GenMCDriver::Mode(GenMCDriver::VerificationMode{});

	// Running Miri-GenMC without race detection is not supported.
	// Disabling this option also changes the behavior of the replay scheduler to only schedule
	// at atomic operations, which is required with Miri. This happens because Miri can generate
	// multiple GenMC events for a single MIR terminator. Without this option, the scheduler
	// might incorrectly schedule an atomic MIR terminator because the first event it creates is
	// a non-atomic (e.g., `StorageLive`).
	conf->disableRaceDetection = false;

	// Miri can already check for unfreed memory. Also, GenMC cannot distinguish between memory
	// that is allowed to leak and memory that is not.
	conf->warnUnfreedMemory = false;

	// FIXME(GenMC): This function currently exits on error, but will return an error value in
	// the future.
	checkConfig(*conf);

	auto driver = std::make_unique<MiriGenMCShim>(std::move(conf), mode);

	auto *driverPtr = driver.get();
	auto initValGetter = [driverPtr](const AAccess &access) {
		const auto addr = access.getAddr();
		if (!driverPtr->initVals_.contains(addr)) {
			LOG(VerbosityLevel::Warning)
				<< "WARNING: TODO GENMC: requested initial value for address "
				<< addr << ", but there is none.\n";
			return SVal(0xCC00CC00);
			// BUG_ON(!driverPtr->initVals_.contains(addr));
		}
		auto result = driverPtr->initVals_[addr];
		if (!result.is_init) {
			LOG(VerbosityLevel::Warning)
				<< "WARNING: TODO GENMC: requested initial value for address "
				<< addr << ", but the memory is uninitialized.\n";
			return SVal(0xFF00FF00);
		}
		LOG(VerbosityLevel::Warning)
			<< "MiriGenMCShim: requested initial value for address " << addr
			<< " == " << addr.get() << ", returning: " << result << "\n";
		return result.toSVal();
	};
	driver->getExec().getGraph().setInitValGetter(initValGetter);

	return driver;
}

// This needs to be available to Miri, but clang-tidy wants it static
// NOLINTNEXTLINE(misc-use-internal-linkage)
auto createGenmcHandle(const GenmcParams &config, bool estimation_mode)
	-> std::unique_ptr<MiriGenMCShim>
{
	return MiriGenMCShim::createHandle(config, estimation_mode);
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

/**** Thread management ****/

void MiriGenMCShim::handleThreadCreate(ThreadId thread_id, ThreadId parent_id)
{
	// NOTE: The threadCreate event happens in the parent:
	auto pos = incPos(parent_id);

	// FIXME(genmc): for supporting symmetry reduction, these will need to be properly set:
	const unsigned fun_id = 0;
	const SVal arg = SVal(0);
	const ThreadInfo childInfo = ThreadInfo{thread_id, parent_id, fun_id, arg};

	// NOTE: Default memory ordering (`Release`) used here.
	auto childTid = GenMCDriver::handleThreadCreate(pos, childInfo, EventDeps());

	BUG_ON(childTid != thread_id || childTid <= 0 || childTid != threadsAction.size());
	threadsAction.push_back(Action(ActionKind::Load, Event(childTid, 0)));
}

void MiriGenMCShim::handleThreadJoin(ThreadId thread_id, ThreadId child_id)
{
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

void MiriGenMCShim::handleThreadFinish(ThreadId thread_id, uint64_t ret_val)
{
	const auto pos = incPos(thread_id);
	// NOTE: Default memory ordering (`Release`) used here.
	GenMCDriver::handleThreadFinish(pos, SVal(ret_val));
}

void MiriGenMCShim::handleThreadKill(ThreadId thread_id)
{
	const auto pos = incPos(thread_id);
	GenMCDriver::handleThreadKill(pos);
}

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
