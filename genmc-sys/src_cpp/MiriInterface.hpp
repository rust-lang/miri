#ifndef GENMC_MIRI_INTERFACE_HPP
#define GENMC_MIRI_INTERFACE_HPP

#include "rust/cxx.h"

#include "config.h"

#include "Config/Config.hpp"
#include "ExecutionGraph/EventLabel.hpp"
#include "Static/ModuleID.hpp"
#include "Support/MemOrdering.hpp"
#include "Support/RMWOps.hpp"
#include "Support/ResultHandling.hpp"
#include "Verification/GenMCDriver.hpp"

#include <cstdint>
#include <format>
#include <iomanip>
#include <memory>

/**** Types available to Miri ****/

struct GenmcParams;

using ThreadId = int;

struct MiriGenMCShim : private GenMCDriver
{

public:
	MiriGenMCShim(std::shared_ptr<const Config> conf, Mode mode /* = VerificationMode{} */)
		: GenMCDriver(std::move(conf), nullptr, mode)
	{
		globalInstructions.reserve(8);
		globalInstructions.push_back(Action(ActionKind::Load, Event::getInit()));
	}

	virtual ~MiriGenMCShim() {}

	/**** Execution start/end handling ****/

	void handleExecutionStart();
	std::unique_ptr<ModelCheckerError> handleExecutionEnd();

	/**** Memory access handling ****/

	[[nodiscard]] LoadResult handleLoad(ThreadId thread_id, uint64_t address, uint64_t size,
										MemOrdering ord, GenmcScalar old_val);
	[[nodiscard]] StoreResult handleStore(ThreadId thread_id, uint64_t address, uint64_t size,
										  GenmcScalar value, GenmcScalar old_val,
										  MemOrdering ord);

	/**** Memory (de)allocation ****/

	uintptr_t handleMalloc(ThreadId thread_id, uint64_t size, uint64_t alignment);
	void handleFree(ThreadId thread_id, uint64_t address, uint64_t size);

	/**** Thread management ****/

	void handleThreadCreate(ThreadId thread_id, ThreadId parent_id);
	void handleThreadJoin(ThreadId thread_id, ThreadId child_id);
	void handleThreadFinish(ThreadId thread_id, uint64_t ret_val);
	void handleThreadKill(ThreadId thread_id);

	/**** Scheduling queries ****/

	auto scheduleNext(const int curr_thread_id, const ActionKind curr_thread_next_instr_kind)
		-> int64_t;

	/**** TODO GENMC: Other stuff: ****/

	auto getBlockedExecutionCount() const -> uint64_t
	{
		return static_cast<uint64_t>(getResult().exploredBlocked);
	}

	auto getExploredExecutionCount() const -> uint64_t
	{
		return static_cast<uint64_t>(getResult().explored);
	}

	bool isExplorationDone() { return GenMCDriver::done(); }

	/**** OTHER ****/

	auto incPos(ThreadId tid) -> Event
	{
		ERROR_ON(tid >= globalInstructions.size(), "ThreadId out of bounds");
		return ++globalInstructions[tid].event;
	}
	auto decPos(ThreadId tid) -> Event
	{
		ERROR_ON(tid >= globalInstructions.size(), "ThreadId out of bounds");
		return --globalInstructions[tid].event;
	}

	static std::unique_ptr<MiriGenMCShim> createHandle(const GenmcParams &config);

private:
	std::vector<Action> globalInstructions;
};

/**** Functions available to Miri ****/

// NOTE: CXX doesn't support exposing static methods to Rust currently, so we expose this function instead.
std::unique_ptr<MiriGenMCShim> createGenmcHandle(const GenmcParams &config);

constexpr auto getGlobalAllocStaticMask() -> uint64_t { return SAddr::staticMask; }

#endif /* GENMC_MIRI_INTERFACE_HPP */
