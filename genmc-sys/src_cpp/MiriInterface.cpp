// `genmc-sys/src_cpp` headers:
#include "MiriInterface.hpp"
#include "LogLevel.hpp"

// CXX.rs generated headers:
#include "genmc-sys/src/lib.rs.h"

// GenMC headers:
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

// C++ headers:
#include <cstddef>
#include <cstdint>
#include <memory>
#include <utility>

using AnnotID = ModuleID::ID;
using AnnotT = SExpr<AnnotID>;

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
	// TODO GENMC: Can we get some default values somehow?
	// Config::saveConfigOptions(*conf);

	// NOTE: Miri already does validity checks, so we can disable them in GenMC.
	conf->skipAccessValidityChecks = true;
	// Miri handles non-atomic accesses, so we skip the check for those in GenMC.
	// Mixed atomic-non-atomic mixed-size checks are still enabled.
	conf->allowNonAtomicMixedSizeAccesses = true;

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

	checkConfigOptions(*conf, true);

	auto driver = std::make_unique<MiriGenMCShim>(std::move(conf), mode);

	auto *driverPtr = driver.get();
	auto initValGetter = [driverPtr](const AAccess &access) {
		const auto addr = access.getAddr();
		if (!driverPtr->initVals_.contains(addr)) {
			LOG(VerbosityLevel::Warning) << "WARNING: TODO GENMC: requested initial value for address "
				   << addr << ", but there is none.\n";
			return SVal(0xCC00CC00);
			// BUG_ON(!driverPtr->initVals_.contains(addr));
		}
		auto result = driverPtr->initVals_[addr];
		if (!result.is_init) {
			LOG(VerbosityLevel::Warning) << "WARNING: TODO GENMC: requested initial value for address "
				   << addr << ", but the memory is uninitialized.\n";
			return SVal(0xFF00FF00);
		}
		LOG(VerbosityLevel::Warning) << "MiriGenMCShim: requested initial value for address " << addr
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
	return GenMCDriver::handleExecutionEnd(threadsAction);
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

	BUG_ON(genmcTid != thread_id || genmcTid == -1 || genmcTid != threadsAction.size());
	threadsAction.push_back(Action(ActionKind::Load, Event(genmcTid, 0)));
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
	auto pos = incPos(thread_id);
	auto retVal = SVal(ret_val);

	// NOTE: Default memory ordering (`Release`) used here.
	auto eLab = std::make_unique<ThreadFinishLabel>(pos, retVal);

	GenMCDriver::handleThreadFinish(std::move(eLab));
}

void MiriGenMCShim::handleThreadKill(ThreadId thread_id)
{
	auto pos = incPos(thread_id);
	auto kLab = std::make_unique<ThreadKillLabel>(pos);

	GenMCDriver::handleThreadKill(std::move(kLab));
}

/**** Blocking instructions ****/

void MiriGenMCShim::handleUserBlock(ThreadId thread_id)
{
	auto pos = incPos(thread_id);
	auto bLab = UserBlockLabel::create(pos);
	GenMCDriver::handleBlock(std::move(bLab));
}

/**** Memory access handling ****/

[[nodiscard]] auto MiriGenMCShim::handleLoad(ThreadId thread_id, uint64_t address, uint64_t size,
					     MemOrdering ord, GenmcScalar old_val) -> LoadResult
{
	auto pos = incPos(thread_id);

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

	auto aLab =
		std::make_unique<MallocLabel>(pos, size, alignment, sd, stype, spc, EventDeps());

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

/**** Mutex handling ****/

auto MiriGenMCShim::handleMutexLock(ThreadId thread_id, uint64_t address, uint64_t size)
	-> MutexLockResult
{
	// TODO GENMC: this needs to be identical even in multithreading
	ModuleID::ID annot_id;
	if (annotation_id.contains(address)) {
		annot_id = annotation_id.at(address);
	} else {
		annot_id = annotation_id_counter++;
		annotation_id.insert(std::make_pair(address, annot_id));
	}
	const auto aSize = ASize(size);
	auto annot = std::move(Annotation(
		AssumeType::Spinloop,
		Annotation::ExprVP(NeExpr<AnnotID>::create(
					   RegisterExpr<AnnotID>::create(aSize.getBits(), annot_id),
					   ConcreteExpr<AnnotID>::create(aSize.getBits(), SVal(1)))
					   .release())));

	auto &currPos = threadsAction[thread_id].event;
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

	const bool is_lock_acquired = loadResult.value() == SVal(0);
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
	auto &currPos = threadsAction[thread_id].event;
	auto rLab = TrylockCasReadLabel::create(++currPos, address, size);
	// Mutex starts out unlocked, so we always say the previous value is "unlocked".
	auto oldValSetter = [this](SAddr loc) { this->handleOldVal(loc, SVal(0)); };
	LoadResult loadResult = GenMCDriver::handleLoad(std::move(rLab), oldValSetter);
	if (!loadResult.has_value()) {
		--currPos;
		// TODO GENMC: maybe use std move and make it take a unique_ptr<string> ?
		return MutexLockResult::fromError(*loadResult.error);
	}

	const bool is_lock_acquired = loadResult.value() == SVal(0);
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
