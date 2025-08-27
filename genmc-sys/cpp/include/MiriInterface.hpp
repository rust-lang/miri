#ifndef GENMC_MIRI_INTERFACE_HPP
#define GENMC_MIRI_INTERFACE_HPP

// CXX.rs generated headers:
#include "rust/cxx.h"

// GenMC generated headers:
#include "config.h"

// Miri `genmc-sys/src_cpp` headers:
#include "ResultHandling.hpp"

// GenMC headers:
#include "ExecutionGraph/EventLabel.hpp"
#include "Static/ModuleID.hpp"
#include "Support/MemOrdering.hpp"
#include "Support/RMWOps.hpp"
#include "Verification/Config.hpp"
#include "Verification/GenMCDriver.hpp"

// C++ headers:
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
	}

	virtual ~MiriGenMCShim() {}

	/**** Execution start/end handling ****/

	// This function must be called at the start of any execution, before any events are
	// reported to GenMC.
	void handleExecutionStart();
	// This function must be called at the end of any execution, even if an error was found
	// during the execution.
	std::unique_ptr<ModelCheckerError> handleExecutionEnd();

	/***** Functions for handling events encountered during program execution. *****/

	/**** Memory access handling ****/

	[[nodiscard]] LoadResult handleLoad(ThreadId thread_id, uint64_t address, uint64_t size,
										MemOrdering ord, GenmcScalar old_val);
	[[nodiscard]] StoreResult handleStore(ThreadId thread_id, uint64_t address, uint64_t size,
										  GenmcScalar value, GenmcScalar old_val,
										  MemOrdering ord);

	/**** Memory (de)allocation ****/
	uintptr_t handleMalloc(ThreadId thread_id, uint64_t size, uint64_t alignment);
	void handleFree(ThreadId thread_id, uint64_t address);

	/**** Thread management ****/
	void handleThreadCreate(ThreadId thread_id, ThreadId parent_id);
	void handleThreadJoin(ThreadId thread_id, ThreadId child_id);
	void handleThreadFinish(ThreadId thread_id, uint64_t ret_val);
	void handleThreadKill(ThreadId thread_id);

	/***** Exploration related functionality *****/

	/** Ask the GenMC scheduler for a new thread to schedule and return whether the execution is
	 * finished, blocked, or can continue. */
	auto scheduleNext(const int curr_thread_id, const ActionKind curr_thread_next_instr_kind)
		-> SchedulingResult;

	/**
	 * Check whether there are more executions to explore.
	 * If there are more executions, this method prepares for the next execution and returns
	 * `true`. Returns true if there are no more executions to explore. */
	bool is_exploration_done() { return GenMCDriver::done(); }

	/**** Result querying functionality. ****/

	// NOTE: We don't want to share the `VerificationResult` type with the Rust side, since it
	// is very large, uses features that CXX.rs doesn't support and may change as GenMC changes.
	// Instead, we only use the result on the C++ side, and only expose these getter function to
	// the Rust side.

	/// Get the number of blocked executions encountered by GenMC (cast into a fixed with
	/// integer)
	auto get_blocked_execution_count() const -> uint64_t
	{
		return static_cast<uint64_t>(getResult().exploredBlocked);
	}

	/// Get the number of executions explored by GenMC (cast into a fixed with integer)
	auto get_explored_execution_count() const -> uint64_t
	{
		return static_cast<uint64_t>(getResult().explored);
	}

	/// Get all messages that GenMC produced (errors, warnings).
	auto get_result_message() const -> std::unique_ptr<std::string>
	{
		return std::make_unique<std::string>(getResult().message);
	}

	/// If an error occurred, return a string describing the error, otherwise, return `nullptr`.
	auto get_error_string() const -> std::unique_ptr<std::string>
	{
		const auto &result = GenMCDriver::getResult();
		if (result.status.has_value())
			return std::make_unique<std::string>(format_error(result.status.value()));
		return nullptr;
	}

	static std::unique_ptr<MiriGenMCShim> create_handle(const GenmcParams &config);

private:
	/** Increment the event index in the given thread by 1 and return the new event. */
	[[nodiscard]] inline auto incPos(ThreadId tid) -> Event
	{
		ERROR_ON(tid >= threads_action_.size(), "ThreadId out of bounds");
		return ++threads_action_[tid].event;
	}
	/** Decrement the event index in the given thread by 1 and return the new event. */
	inline auto decPos(ThreadId tid) -> Event
	{
		ERROR_ON(tid >= threads_action_.size(), "ThreadId out of bounds");
		return --threads_action_[tid].event;
	}

	/**
	 * Helper function for loads that need to reset the event counter when no value is returned.
	 * Same syntax as `GenMCDriver::handleLoad`, but this takes a thread id instead of an Event.
	 * Automatically calls `incPos` and `decPos` where needed for the given thread.
	 */
	template <EventLabel::EventLabelKind k, typename... Ts>
	HandleResult<SVal> handleLoadResetIfNone(ThreadId tid, Ts &&...params)
	{
		const auto pos = incPos(tid);
		const auto ret =
			GenMCDriver::handleLoad<k>(pos, std::forward<Ts>(params)...);
		// If we didn't get a value, we reset the index of the current thread.
		if (!std::holds_alternative<SVal>(ret))
		{
			decPos(tid);
		}
		return ret;
	}

	/**
	 * Currently, the interpreter is responsible for maintaining `ExecutionGraph` event indices.
	 * The interpreter is also responsible for informing GenMC about the `ActionKind` of the
	 * next instruction in each thread.
	 *
	 * This vector contains this data in the expected format `Action`, which consists of the
	 * `ActionKind` of the next instruction and the last event index added to the ExecutionGraph
	 * in a given thread.
	 */
	std::vector<Action> threads_action_;
};

/**** Functions available to Miri ****/

// NOTE: CXX doesn't support exposing static methods to Rust currently, so we expose this function instead.
std::unique_ptr<MiriGenMCShim> create_genmc_handle(const GenmcParams &config);

constexpr auto get_global_alloc_static_mask() -> uint64_t { return SAddr::staticMask; }

#endif /* GENMC_MIRI_INTERFACE_HPP */
