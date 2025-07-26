#include "MiriInterface.hpp"

#include "genmc-sys/src/lib.rs.h"

#include "ADT/value_ptr.hpp"
#include "Config/MemoryModel.hpp"
#include "Config/Verbosity.hpp"
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
#include "Verification/GenMCDriver.hpp"

#include <cstddef>
#include <cstdint>
#include <memory>
#include <utility>

using AnnotID = ModuleID::ID;
using AnnotT = SExpr<AnnotID>;

// Return -1 when no thread can/should be scheduled, or the thread id of the next thread
// NOTE: this is safe because ThreadId is 32 bit, and we return a 64 bit integer
// FIXME(genmc,cxx): could directly return std::optional if CXX ever supports sharing it (see https://github.com/dtolnay/cxx/issues/87).
auto MiriGenMCShim::scheduleNext(const int curr_thread_id,
				 const ActionKind curr_thread_next_instr_kind) -> int64_t
{
	// The current thread is the only one where the `kind` could have changed since we last made
	// a scheduling decision.
	globalInstructions[curr_thread_id].kind = curr_thread_next_instr_kind;

	auto result = GenMCDriver::scheduleNext(globalInstructions);
	if (result.has_value()) {
		return static_cast<int64_t>(result.value());
	}
	return -1;
}

/**** Functions available to Miri ****/

// NOLINTNEXTLINE(readability-convert-member-functions-to-static)
auto MiriGenMCShim::createHandle(const GenmcParams &config, bool estimation_mode)
	-> std::unique_ptr<MiriGenMCShim>
{
	auto conf = std::make_shared<Config>();
	// TODO GENMC: Can we get some default values somehow?
	// Config::saveConfigOptions(*conf);

	// NOTE: Miri already initialization checks, so we can disable them in GenMC
	conf->skipNonAtomicInitializedCheck = true;

	// Miri needs all threads to be replayed, even fully completed ones.
	conf->replayCompletedThreads = true;

	// FIXME(genmc): make sure this doesn't affect any tests, and maybe make it changeable from Miri:
	constexpr unsigned int DEFAULT_WARN_ON_GRAPH_SIZE = 16 * 1024;
	conf->warnOnGraphSize = DEFAULT_WARN_ON_GRAPH_SIZE;

	// We only support the RC11 memory model for Rust.
	conf->model = ModelType::RC11;

	// FIXME(genmc): expose this setting to Miri
	conf->randomScheduleSeed = "42";
	conf->printRandomScheduleSeed = config.print_random_schedule_seed;
	if (config.quiet) {
		// logLevel = VerbosityLevel::Quiet;
		// TODO GENMC: error might be better (or new level for `BUG`)
		// logLevel = VerbosityLevel::Quiet;
		logLevel = VerbosityLevel::Error;
	} else if (config.log_level_trace) {
		logLevel = VerbosityLevel::Trace;
	} else {
		logLevel = VerbosityLevel::Tip;
	}

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

	checkConfigOptions(*conf, true);

	auto driver = std::make_unique<MiriGenMCShim>(std::move(conf), mode);

	auto *driverPtr = driver.get();
	auto initValGetter = [driverPtr](const AAccess &access) {
		const auto addr = access.getAddr();
		if (!driverPtr->initVals_.contains(addr)) {
			MIRI_LOG() << "WARNING: TODO GENMC: requested initial value for address "
				   << addr << ", but there is none.\n";
			return SVal(0xCC00CC00);
			// BUG_ON(!driverPtr->initVals_.contains(addr));
		}
		auto result = driverPtr->initVals_[addr];
		if (!result.is_init) {
			MIRI_LOG() << "WARNING: TODO GENMC: requested initial value for address "
				   << addr << ", but the memory is uninitialized.\n";
			return SVal(0xFF00FF00);
		}
		MIRI_LOG() << "MiriGenMCShim: requested initial value for address " << addr
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
	globalInstructions.clear();
	globalInstructions.push_back(Action(ActionKind::Load, Event::getInit()));
	GenMCDriver::handleExecutionStart();
}

auto MiriGenMCShim::handleExecutionEnd() -> std::unique_ptr<ModelCheckerError>
{
	return GenMCDriver::handleExecutionEnd(globalInstructions);
}

/**** Thread management ****/

void MiriGenMCShim::handleThreadCreate(ThreadId thread_id, ThreadId parent_id)
{
	// NOTE: The threadCreate event happens in the parent:
	auto pos = incPos(parent_id);

	// FIXME(genmc): for supporting symmetry reduction, these will need to be properly set:
	const unsigned funId = 0;
	const SVal arg = SVal(0);
	const ThreadInfo childInfo = ThreadInfo{thread_id, parent_id, funId, arg};

	// NOTE: Default memory ordering (`Release`) used here.
	auto tcLab = std::make_unique<ThreadCreateLabel>(pos, childInfo);
	auto createLab = GenMCDriver::handleThreadCreate(std::move(tcLab));
	auto genmcTid = createLab->getChildId();

	BUG_ON(genmcTid != thread_id || genmcTid == -1 || genmcTid != globalInstructions.size());
	globalInstructions.push_back(Action(ActionKind::Load, Event(genmcTid, 0)));
}

void MiriGenMCShim::handleThreadJoin(ThreadId thread_id, ThreadId child_id)
{
	// The thread join event happens in the parent.
	auto pos = incPos(thread_id);

	// NOTE: Default memory ordering (`Acquire`) used here.
	auto lab = std::make_unique<ThreadJoinLabel>(pos, child_id);
	auto res = GenMCDriver::handleThreadJoin(std::move(lab));
	// If the join failed, decrease the event index again:
	if (!res.has_value())
		decPos(thread_id);

	// NOTE: Thread return value is ignored, since Miri doesn't need it.
}

void MiriGenMCShim::handleThreadFinish(ThreadId thread_id, uint64_t ret_val)
{
	MIRI_LOG() << "GenMC:   handleThreadFinish: thread id: " << thread_id << "\n";

	auto pos = incPos(thread_id);
	auto retVal = SVal(ret_val);

	// NOTE: Default memory ordering (`Release`) used here.
	auto eLab = std::make_unique<ThreadFinishLabel>(pos, retVal);

	GenMCDriver::handleThreadFinish(std::move(eLab));
}

void MiriGenMCShim::handleThreadKill(ThreadId thread_id) {
	auto pos = incPos(thread_id);
	auto kLab = std::make_unique<ThreadKillLabel>(pos);

	GenMCDriver::handleThreadKill(std::move(kLab));
}

/**** Memory access handling ****/

[[nodiscard]] auto MiriGenMCShim::handleLoad(ThreadId thread_id, uint64_t address, uint64_t size,
					     MemOrdering ord, GenmcScalar old_val) -> LoadResult
{
	auto pos = incPos(thread_id);
	MIRI_LOG() << "Received Load from Miri at address: " << address << ", size " << size
		   << " with ordering " << ord << ", event: " << pos << "\n";

	auto loc = SAddr(address);
	auto aSize = ASize(size);
	// `type` is only used for printing.
	auto type = AType::Unsigned;

	auto newLab = std::make_unique<ReadLabel>(pos, ord, loc, aSize, type);

	auto oldValSetter = [this, old_val](SAddr loc) { this->handleOldVal(loc, old_val); };
	auto result = GenMCDriver::handleLoad(std::move(newLab), oldValSetter);
	return result;
}

[[nodiscard]] auto MiriGenMCShim::handleReadModifyWrite(ThreadId thread_id, uint64_t address,
							uint64_t size, MemOrdering loadOrd,
							MemOrdering store_ordering, RMWBinOp rmw_op,
							GenmcScalar rhs_value, GenmcScalar old_val)
	-> ReadModifyWriteResult
{
	MIRI_LOG() << "Received Read-Modify-Write from Miri at address: " << address << ", size "
		   << size << " with orderings (" << loadOrd << ", " << store_ordering
		   << "), rmw op: " << static_cast<uint64_t>(rmw_op) << "\n";

	auto pos = incPos(thread_id);

	auto loc = SAddr(address);
	auto aSize = ASize(size);
	// `type` is only used for printing.
	auto type = AType::Unsigned;

	auto rhsVal = rhs_value.toSVal();
	auto newLab =
		std::make_unique<FaiReadLabel>(pos, loadOrd, loc, aSize, type, rmw_op, rhsVal);

	auto oldValSetter = [this, old_val](SAddr loc) { this->handleOldVal(loc, old_val); };
	auto result = GenMCDriver::handleLoad(std::move(newLab), oldValSetter);
	if (const auto *error = result.error.get()) {
		return ReadModifyWriteResult::fromError(*error);
	}

	auto oldVal = result.scalar.toSVal(); // TODO GENMC: u128 handling
	auto newVal = executeRMWBinOp(oldVal, rhsVal, size, rmw_op);

	auto store_result = handleStore(thread_id, address, size, GenmcScalar(newVal), old_val,
					store_ordering, StoreEventType::ReadModifyWrite);

	if (store_result.is_error())
		return ReadModifyWriteResult::fromError(*store_result.error.get());
	return ReadModifyWriteResult(oldVal, newVal, store_result.isCoMaxWrite);
}

[[nodiscard]] auto MiriGenMCShim::handleCompareExchange(
	ThreadId thread_id, uint64_t address, uint64_t size, GenmcScalar expected_value,
	GenmcScalar new_value, GenmcScalar old_val, MemOrdering success_load_ordering,
	MemOrdering success_store_ordering, MemOrdering fail_load_ordering,
	bool can_fail_spuriously) -> CompareExchangeResult
{

	MIRI_LOG() << "Received Compare-Exchange from Miri (value: " << expected_value << " --> "
		   << new_value << ", old value: " << old_val << ") at address: " << address
		   << ", size " << size << " with success orderings (" << success_load_ordering
		   << ", " << success_store_ordering
		   << "), fail load ordering: " << fail_load_ordering
		   << ", is weak (can fail spuriously): " << can_fail_spuriously << "\n";

	auto pos = incPos(thread_id);

	auto loc = SAddr(address);
	auto aSize = ASize(size);
	// `type` is only used for printing.
	auto type = AType::Unsigned;

	auto expectedVal = expected_value.toSVal();
	auto newVal = new_value.toSVal();

	// FIXME(GenMC): properly handle failure memory ordering.

	auto newLab = std::make_unique<CasReadLabel>(pos, success_load_ordering, loc, aSize, type,
						     expectedVal, newVal);

	auto oldValSetter = [this, old_val](SAddr loc) { this->handleOldVal(loc, old_val); };
	auto result = GenMCDriver::handleLoad(std::move(newLab), oldValSetter);
	if (const auto *error = result.error.get()) {
		return CompareExchangeResult::fromError(*error);
	}

	auto oldVal = result.scalar.toSVal();
	if (oldVal != expectedVal)
		return CompareExchangeResult::failure(oldVal);

	auto store_result = handleStore(thread_id, address, size, GenmcScalar(newVal), old_val,
					success_store_ordering, StoreEventType::CompareExchange);

	if (store_result.is_error())
		return CompareExchangeResult::fromError(*store_result.error);
	return CompareExchangeResult::success(oldVal, store_result.isCoMaxWrite);
}

[[nodiscard]] auto MiriGenMCShim::handleStore(ThreadId thread_id, uint64_t address, uint64_t size,
					      GenmcScalar value, GenmcScalar old_val,
					      MemOrdering ord, StoreEventType store_event_type)
	-> StoreResult
{
	MIRI_LOG() << "Received Store from Miri at address " << address << ", size " << size
		   << " with ordering " << ord << ", is part of rmw: ("
		   << static_cast<uint64_t>(store_event_type) << ")\n";

	auto pos = incPos(thread_id);

	auto loc = SAddr(address);
	auto aSize = ASize(size);
	// `type` is only used for printing.
	auto type = AType::Unsigned;

	auto val = value.toSVal();

	std::unique_ptr<WriteLabel> wLab;
	switch (store_event_type) {
	case StoreEventType::Normal:
		wLab = std::make_unique<WriteLabel>(pos, ord, loc, aSize, type, val);
		break;
	case StoreEventType::ReadModifyWrite:
		wLab = std::make_unique<FaiWriteLabel>(pos, ord, loc, aSize, type, val);
		break;
	case StoreEventType::CompareExchange:
		wLab = std::make_unique<CasWriteLabel>(pos, ord, loc, aSize, type, val);
		break;
	default:
		ERROR("Unsupported Store Event Type");
	}

	auto oldValSetter = [this, old_val](SAddr loc) {
		this->handleOldVal(loc,
				   old_val); // TODO GENMC(HACK): is this the correct way to do it?
	};

	return GenMCDriver::handleStore(std::move(wLab), oldValSetter);
}

void MiriGenMCShim::handleFence(ThreadId thread_id, MemOrdering ord)
{
	MIRI_LOG() << "Received fence operation from Miri with ordering " << ord << "\n";

	auto pos = incPos(thread_id);

	auto fLab = std::make_unique<FenceLabel>(pos, ord);
	GenMCDriver::handleFence(std::move(fLab));
}

/**** Memory (de)allocation ****/

auto MiriGenMCShim::handleMalloc(ThreadId thread_id, uint64_t size, uint64_t alignment) -> uintptr_t
{
	auto pos = incPos(thread_id);

	// These are only used for printing and features Miri-GenMC doesn't support (yet).
	auto sd = StorageDuration::SD_Heap;
	auto stype = StorageType::ST_Volatile;
	auto spc = AddressSpace::AS_User;

	auto aLab = std::make_unique<MallocLabel>(pos, size, alignment, sd, stype, spc, EventDeps());

	SAddr retVal = GenMCDriver::handleMalloc(std::move(aLab));

	BUG_ON(retVal.get() == 0);

	auto address = retVal.get();
	return address;
}

void MiriGenMCShim::handleFree(ThreadId thread_id, uint64_t address, uint64_t size)
{
	auto addr = SAddr(address);
	auto alloc_size = SAddr(size);

	auto pos = incPos(thread_id);

	auto dLab = std::make_unique<FreeLabel>(pos, addr, size);
	GenMCDriver::handleFree(std::move(dLab));
}
