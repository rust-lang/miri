#include "MiriInterface.hpp"

#include "genmc-sys/src/lib.rs.h"

#include "ExecutionGraph/EventLabel.hpp"
#include "Support/ASize.hpp"
#include "Support/Error.hpp"
#include "Support/Logger.hpp"
#include "Support/MemAccess.hpp"
#include "Support/MemOrdering.hpp"
#include "Support/MemoryModel.hpp"
#include "Support/RMWOps.hpp"
#include "Support/SAddr.hpp"
#include "Support/SVal.hpp"
#include "Support/ThreadInfo.hpp"
#include "Support/Verbosity.hpp"
#include "Verification/DriverEnumAPI.hpp"
#include "Verification/GenMCDriver.hpp"

#include <cstddef>
#include <cstdint>
#include <memory>
#include <utility>

using AnnotID = ModuleID_ID;
using AnnotT = SExpr<AnnotID>;

// Return -1 when no thread can/should be scheduled, or the thread id of the next thread
// NOTE: this is safe because ThreadId is 32 bit, and we return a 64 bit integer
// TODO GENMC: could directly return std::optional if CXX ever supports this
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
	auto vConf = std::make_shared<VerificationConfig>();
	// TODO GENMC: Can we get some default values somehow?
	// Config::saveConfigOptions(*vConf);

	// NOTE: Miri already initialization checks, so we can disable them in GenMC
	vConf->skipNonAtomicInitializedCheck = true;

	// Miri needs all threads to be replayed, even fully completed ones.
	vConf->replayCompletedThreads = true;

	// TODO GENMC: make sure this doesn't affect any tests, and maybe make it changeable from
	// Miri:
	constexpr unsigned int DEFAULT_WARN_ON_GRAPH_SIZE = 16 * 1024;
	vConf->warnOnGraphSize = DEFAULT_WARN_ON_GRAPH_SIZE;
	vConf->model = ModelType::RC11;
	vConf->randomScheduleSeed =
		"42"; // TODO GENMC: only for random exploration/scheduling mode in GenMC
	vConf->printRandomScheduleSeed = config.print_random_schedule_seed;
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

	// TODO GENMC (EXTRA): check if we can enable IPR:
	vConf->ipr = false;
	// TODO GENMC (EXTRA): check if we can enable BAM:
	vConf->disableBAM = true;
	// TODO GENMC (EXTRA): check if we can enable Symmetry Reduction:
	vConf->symmetryReduction = config.do_symmetry_reduction;

	// TODO GENMC (EXTRA): check if we can do instruction caching (probably not)
	vConf->instructionCaching = false;

	// TODO GENMC: Should there be a way to change this option from Miri?
	vConf->schedulePolicy = SchedulePolicy::WF;

	vConf->estimate = estimation_mode;
	vConf->estimationMax = config.estimation_max;
	const auto mode = vConf->estimate ? GenMCDriver::Mode(GenMCDriver::EstimationMode{})
					 : GenMCDriver::Mode(GenMCDriver::VerificationMode{});

	// With `disableRaceDetection = true`, the scheduler would be incorrectly replaying
	// executions with Miri, since we can only schedule at Mir terminators, and each Mir
	// terminator can generate multiple events in the ExecutionGraph.
	// Users running Miri-GenMC most likely want to always have race detection enabled anyway.
	vConf->disableRaceDetection = false;

	// Miri can already check for unfreed memory. Also, GenMC cannot distinguish between memory
	// that is allowed to leak and memory that is not.
	vConf->warnUnfreedMemory = false;

	checkVerificationConfigOptions(*vConf);

	auto driver = std::make_unique<MiriGenMCShim>(std::move(vConf), mode);

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
	// TODO GENMC: reset completely or just set to init event for each thread?
	globalInstructions.clear();
	globalInstructions.push_back(Action(ActionKind::Load, Event::getInit()));
	// for (auto &action : globalInstructions) {
	// 	action.event.index = 0;
	// 	action.kind = ActionKind::Load;
	// }
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

	const unsigned funId = 0; // TODO GENMC
	const SVal arg = SVal(0); // TODO GENMC
	const ThreadInfo childInfo = ThreadInfo{thread_id, parent_id, funId, arg};

	// NOTE: Default GenMC ordering used here
	auto tcLab = std::make_unique<ThreadCreateLabel>(pos, childInfo);
	auto createLab = GenMCDriver::handleThreadCreate(std::move(tcLab));
	auto genmcTid = createLab->getChildId();

	BUG_ON(genmcTid != thread_id);
	BUG_ON(genmcTid == -1); // TODO GENMC (ERROR HANDLING): proper error handling
	BUG_ON(genmcTid > globalInstructions.size());

	// TODO GENMC: should both these be possible?
	if (genmcTid >= globalInstructions.size())
		globalInstructions.push_back(Action(ActionKind::Load, Event(genmcTid, 0)));
	else
		globalInstructions[genmcTid] = Action(ActionKind::Load, Event(genmcTid, 0));
}

void MiriGenMCShim::handleThreadJoin(ThreadId thread_id, ThreadId child_id)
{
	auto parentTid = thread_id;
	auto childTid = child_id;

	// NOTE: The thread join event happens in the parent:
	auto pos = incPos(parentTid);

	// NOTE: Default GenMC ordering used here
	auto lab = std::make_unique<ThreadJoinLabel>(pos, childTid);
	auto res = GenMCDriver::handleThreadJoin(std::move(lab));
	// TODO GENMC: use return value if needed
	if (res.has_value()) {
		MIRI_LOG() << "TODO GENMC: GenMC::handleThreadJoin: returned value: "
			   << res.getValue() << "\n";
	} else {
		MIRI_LOG() << "MiriGenMCShim::handleThreadJoin got no value.";
		decPos(parentTid);
	}
}

void MiriGenMCShim::handleThreadFinish(ThreadId thread_id, uint64_t ret_val)
{
	MIRI_LOG() << "GenMC:   handleThreadFinish: thread id: " << thread_id << "\n";

	auto pos = incPos(thread_id);
	auto retVal = SVal(ret_val);

	// NOTE: Default GenMC ordering used here
	auto eLab = std::make_unique<ThreadFinishLabel>(pos, retVal);

	GenMCDriver::handleThreadFinish(std::move(eLab));
}

/**** Blocking instructions ****/

void MiriGenMCShim::handleUserBlock(ThreadId thread_id)
{

	auto pos = incPos(thread_id);
	auto bLab = UserBlockLabel::create(/* EventLabelKind::UserBlock, */ pos);
	GenMCDriver::handleBlock(std::move(bLab));
	// TODO GENMC: could this ever fail?
}

/**** Memory access handling ****/

[[nodiscard]] auto MiriGenMCShim::handleLoad(ThreadId thread_id, uint64_t address, uint64_t size,
					     MemOrdering ord, GenmcScalar old_val) -> LoadResult
{
	MIRI_LOG() << "Received Load from Miri at address: " << address << ", size " << size
		   << " with ordering " << ord << "\n";

	auto pos = incPos(thread_id);

	auto loc = SAddr(address);
	auto aSize = ASize(size);
	auto type = AType::Unsigned; // TODO GENMC: get correct type from Miri

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

	auto loc = SAddr(address); // TODO GENMC: called addr for write, loc for read?
	auto aSize = ASize(size);
	auto type = AType::Unsigned; // TODO GENMC: get from Miri

	// TODO GENMC: u128 support
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
	case StoreEventType::MutexUnlockWrite:
		wLab = UnlockWriteLabel::create(pos, ord, loc, aSize, AType::Signed, val);
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
	BUG_ON(size == 0);
	auto pos = incPos(thread_id);

	MIRI_LOG() << "handleMalloc: thread " << thread_id << ", new MallocLabel at position {"
		   << pos.thread << ", " << pos.index << "}\n";

	auto sd = StorageDuration::SD_Heap;   // TODO GENMC: get from Miri
	auto stype = StorageType::ST_Durable; // TODO GENMC
	auto spc = AddressSpace::AS_User;     // TODO GENMC

	auto deps = EventDeps(); // TODO GENMC: without this, constructor is ambiguous

	// TODO GENMC (types): size_t vs unsigned int
	auto aLab = std::make_unique<MallocLabel>(pos, size, alignment, sd, stype, spc, deps);

	SAddr retVal = GenMCDriver::handleMalloc(std::move(aLab));

	BUG_ON(retVal.get() == 0);

	auto address = retVal.get();
	return address;
}

void MiriGenMCShim::handleFree(ThreadId thread_id, uint64_t address, uint64_t size)
{
	MIRI_LOG() << "GENMC: handleFree called (address: " << address << ", size: " << size
		   << ")\n";
	BUG_ON(size == 0);

	auto addr = SAddr(address);
	auto alloc_size = SAddr(size);
	BUG_ON(addr.get() == 0);

	auto pos = incPos(thread_id);

	auto dLab = std::make_unique<FreeLabel>(pos, addr, size);
	GenMCDriver::handleFree(std::move(dLab));
}

/**** Mutex handling ****/

auto MiriGenMCShim::handleMutexLock(ThreadId thread_id, uint64_t address, uint64_t size)
	-> MutexLockResult
{
	// TODO GENMC: this needs to be identical even in multithreading
	unsigned int annot_id;
	if (annotation_id.contains(address)) {
		annot_id = annotation_id.at(address);
	} else {
		annot_id = annotation_id_counter++;
		annotation_id.insert(std::make_pair(address, annot_id));
	}
	auto annot = std::move(Annotation(
		AssumeType::Spinloop,
		Annotation::ExprVP(NeExpr<AnnotID>::create(
					   RegisterExpr<AnnotID>::create(size * CHAR_BIT, annot_id),
					   ConcreteExpr<AnnotID>::create(size * CHAR_BIT, SVal(1)))
					   .release())));

	auto &currPos = globalInstructions[thread_id].event;
	// auto rLab = LockCasReadLabel::create(++currPos, address, size);
	auto rLab = LockCasReadLabel::create(++currPos, address, size, annot);

	// Mutex starts out unlocked, so we always say the previous value is "unlocked".
	auto oldValSetter = [this](SAddr loc) { this->handleOldVal(loc, SVal(0)); };
	LoadResult loadResult = GenMCDriver::handleLoad(std::move(rLab), oldValSetter);
	if (loadResult.is_error()) {
		--currPos;
		return MutexLockResult::fromError(*loadResult.error);
	} else if (loadResult.is_read_opt) {
		--currPos;
		// TODO GENMC: is_read_opt == Mutex is acquired
		// None	--> Someone else has lock, this thread will be rescheduled later (currently
		// block) 0	--> Got the lock 1 	--> Someone else has lock, this thread will
		// not be rescheduled later (block on Miri side)
		return MutexLockResult(false);
	}
	// TODO GENMC(QUESTION): is the `isBlocked` even needed?
	// if (!loadResult.has_value() || getCurThr().isBlocked())
	//     return;

	const bool is_lock_acquired = loadResult.getValue() == SVal(0);
	if (is_lock_acquired) {
		auto wLab = LockCasWriteLabel::create(++currPos, address, size);
		StoreResult storeResult = GenMCDriver::handleStore(std::move(wLab), oldValSetter);
		if (storeResult.is_error())
			return MutexLockResult::fromError(*storeResult.error);

	} else {
		auto bLab = LockNotAcqBlockLabel::create(++currPos);
		GenMCDriver::handleBlock(std::move(bLab));
	}

	return MutexLockResult(is_lock_acquired);
}

auto MiriGenMCShim::handleMutexTryLock(ThreadId thread_id, uint64_t address, uint64_t size)
	-> MutexLockResult
{
	auto &currPos = globalInstructions[thread_id].event;
	auto rLab = TrylockCasReadLabel::create(++currPos, address, size);
	// Mutex starts out unlocked, so we always say the previous value is "unlocked".
	auto oldValSetter = [this](SAddr loc) { this->handleOldVal(loc, SVal(0)); };
	LoadResult loadResult = GenMCDriver::handleLoad(std::move(rLab), oldValSetter);
	if (!loadResult.has_value()) {
		--currPos;
		// TODO GENMC: maybe use std move and make it take a unique_ptr<string> ?
		return MutexLockResult::fromError(*loadResult.error);
	}

	const bool is_lock_acquired = loadResult.getValue() == SVal(0);
	if (!is_lock_acquired)
		return MutexLockResult(false); /* Lock already held. */

	auto wLab = TrylockCasWriteLabel::create(++currPos, address, size);
	StoreResult storeResult = GenMCDriver::handleStore(std::move(wLab), oldValSetter);
	if (storeResult.is_error())
		return MutexLockResult::fromError(*storeResult.error);

	return MutexLockResult(true);
}

auto MiriGenMCShim::handleMutexUnlock(ThreadId thread_id, uint64_t address, uint64_t size)
	-> StoreResult
{
	return handleStore(thread_id, address, size, SVal(0), SVal(0xDEADBEEF),
			   MemOrdering::Release, StoreEventType::MutexUnlockWrite);
}
