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

enum class StoreEventType : uint8_t
{
	Normal,
	CompareExchange,
};

// TODO GENMC: fix naming conventions

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

	///////////////////
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
	/**
	 * @brief Try to insert the initial value of a memory location.
	 * @param addr
	 * @param value
	 * */
	void handleOldVal(const SAddr addr, GenmcScalar value)
	{
		// TODO GENMC(CLEANUP): Pass this as a parameter:
		auto &g = getExec().getGraph();
		auto *coLab = g.co_max(addr);
		if (auto *wLab = llvm::dyn_cast<WriteLabel>(coLab))
		{
			if (value.is_init && wLab->isNotAtomic())
				wLab->setVal(value.toSVal());
		}
		else if (const auto *wLab = llvm::dyn_cast<InitLabel>(coLab))
		{
			if (value.is_init)
			{
				auto result = initVals_.insert(std::make_pair(addr, value));
				BUG_ON(result.second &&
					   (*result.first).second !=
						   value); /* Attempt to replace initial value */
			}
		}
		else
		{
			BUG(); /* Invalid label */
		}
		// either initLabel	==> update initValGetter
		// or WriteLabel    ==> Update its value in place (only if non-atomic)
	}

	// TODO GENMC(mixed-size accesses):
	std::unordered_map<SAddr, GenmcScalar> initVals_{};

	std::vector<Action> globalInstructions;

	std::unordered_map<uint64_t, ModuleID::ID> annotation_id{};
	ModuleID::ID annotation_id_counter = 0;
};

/**** Functions available to Miri ****/

// NOTE: CXX doesn't seem to support exposing static methods to Rust, so we expose this
// function instead
std::unique_ptr<MiriGenMCShim> createGenmcHandle(const GenmcParams &config);

constexpr auto getGlobalAllocStaticMask() -> uint64_t { return SAddr::staticMask; }

#endif /* GENMC_MIRI_INTERFACE_HPP */
