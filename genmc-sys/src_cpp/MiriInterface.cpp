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

// Return -1 when no thread can/should be scheduled, or the thread id of the next thread
// NOTE: this is safe because ThreadId is 32 bit, and we return a 64 bit integer
// FIXME(genmc,cxx): could directly return std::optional if CXX ever supports sharing it (see https://github.com/dtolnay/cxx/issues/87).
auto MiriGenMCShim::scheduleNext(const int curr_thread_id,
								 const ActionKind curr_thread_next_instr_kind) -> int64_t
{
	// The current thread is the only one where the `kind` could have changed since we last made
	// a scheduling decision.
	globalInstructions[curr_thread_id].kind = curr_thread_next_instr_kind;

	if (const auto result = GenMCDriver::scheduleNext(globalInstructions))
		return static_cast<int64_t>(result.value());
	return -1;
}

/**** Functions available to Miri ****/

// NOLINTNEXTLINE(readability-convert-member-functions-to-static)
auto MiriGenMCShim::createHandle(const GenmcParams &config)
	-> std::unique_ptr<MiriGenMCShim>
{
	auto conf = std::make_shared<Config>();

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

	// FIXME(genmc): Add support for setting this from the Miri side.
	// FIXME(genmc): Decide on what to do about warnings from GenMC (keep them disabled until then).
	logLevel = VerbosityLevel::Error;

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

	// FIXME(genmc): implement estimation mode.
	conf->estimate = false;
	const auto mode = GenMCDriver::Mode(GenMCDriver::VerificationMode{});

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
	auto initValGetter = [driverPtr](const AAccess &access)
	{
		// FIXME(genmc): Add proper support for initial values.
		return SVal(0xff);
	};
	driver->getExec().getGraph().setInitValGetter(initValGetter);

	return driver;
}

// This needs to be available to Miri, but clang-tidy wants it static
// NOLINTNEXTLINE(misc-use-internal-linkage)
auto createGenmcHandle(const GenmcParams &config)
	-> std::unique_ptr<MiriGenMCShim>
{
	return MiriGenMCShim::createHandle(config);
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

	auto loc = SAddr(address);
	auto aSize = ASize(size);
	// `type` is only used for printing.
	auto type = AType::Unsigned;

	auto newLab = std::make_unique<ReadLabel>(pos, ord, loc, aSize, type);

	auto result = GenMCDriver::handleLoad(std::move(newLab));
	return result;
}

[[nodiscard]] auto MiriGenMCShim::handleStore(ThreadId thread_id, uint64_t address, uint64_t size,
											  GenmcScalar value, GenmcScalar old_val,
											  MemOrdering ord)
	-> StoreResult
{
	auto pos = incPos(thread_id);

	auto loc = SAddr(address);
	auto aSize = ASize(size);
	// `type` is only used for printing.
	auto type = AType::Unsigned;

	auto val = value.toSVal();

	std::unique_ptr<WriteLabel> wLab = std::make_unique<WriteLabel>(pos, ord, loc, aSize, type, val);

	return GenMCDriver::handleStore(std::move(wLab));
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
