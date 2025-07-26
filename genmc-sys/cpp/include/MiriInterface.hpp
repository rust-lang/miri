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

using AnnotID = ModuleID::ID;
using AnnotT = SExpr<AnnotID>;

// TODO GENMC: fix naming conventions

struct MiriGenMCShim : private GenMCDriver {

public:
	MiriGenMCShim(std::shared_ptr<const Config> conf, Mode mode /* = VerificationMode{} */)
		: GenMCDriver(std::move(conf), nullptr, mode)
	{}

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
	[[nodiscard]] ReadModifyWriteResult
	handleReadModifyWrite(ThreadId thread_id, uint64_t address, uint64_t size,
			      MemOrdering loadOrd, MemOrdering store_ordering, RMWBinOp rmw_op,
			      GenmcScalar rhs_value, GenmcScalar old_val);
	[[nodiscard]] CompareExchangeResult
	handleCompareExchange(ThreadId thread_id, uint64_t address, uint64_t size,
			      GenmcScalar expected_value, GenmcScalar new_value,
			      GenmcScalar old_val, MemOrdering success_load_ordering,
			      MemOrdering success_store_ordering, MemOrdering fail_load_ordering,
			      bool can_fail_spuriously);
	[[nodiscard]] StoreResult handleStore(ThreadId thread_id, uint64_t address, uint64_t size,
					      GenmcScalar value, GenmcScalar old_val,
					      MemOrdering ord);

	void handleFence(ThreadId thread_id, MemOrdering ord);

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
	bool isExplorationDone() { return GenMCDriver::done(); }

	/**** Result querying functionality. ****/

	// NOTE: We don't want to share the `VerificationResult` type with the Rust side, since it
	// is very large, uses features that CXX.rs doesn't support and may change as GenMC changes.
	// Instead, we only use the result on the C++ side, and only expose these getter function to
	// the Rust side.

	/// Get the number of blocked executions encountered by GenMC (cast into a fixed with
	/// integer)
	auto getBlockedExecutionCount() const -> uint64_t
	{
		return static_cast<uint64_t>(getResult().exploredBlocked);
	}

	/// Get the number of executions explored by GenMC (cast into a fixed with integer)
	auto getExploredExecutionCount() const -> uint64_t
	{
		return static_cast<uint64_t>(getResult().explored);
	}

	/// Get all messages that GenMC produced (errors, warnings).
	auto getResultMessage() const -> std::unique_ptr<std::string>
	{
		return std::make_unique<std::string>(getResult().message);
	}

	/// If an error occurred, return a string describing the error, otherwise, return `nullptr`.
	auto getErrorString() const -> std::unique_ptr<std::string>
	{
		const auto &result = GenMCDriver::getResult();
		if (result.status.has_value()) {
			// FIXME(genmc): format the error once std::format changes are merged into
			// GenMC.
			return std::make_unique<std::string>("FIXME(genmc): show error string");
		}
		return nullptr;
	}

	/**** Printing and estimation mode functionality. ****/

	void printEstimationResults(const double elapsed_time_sec) const
	{
		// FIXME(GenMC,cleanup): should this happen on the Rust side?
		const auto &res = getResult();
		const auto *conf = getConf();

		long long mean = std::llround(res.estimationMean);
		long long sd = std::llround(std::sqrt(res.estimationVariance));
		long double meanTimeSecs =
			(long double)elapsed_time_sec / (res.explored + res.exploredBlocked);
		PRINT(VerbosityLevel::Error)
			<< "Finished estimation in " << std::format("%.2f", elapsed_time_sec)
			<< " seconds.\n\n"
			<< std::format("Total executions estimate: {} (+- {})\n", mean, sd)
			<< "Time to completion estimate: "
			<< std::format("%.2f", meanTimeSecs * mean) << "s\n";
		GENMC_DEBUG(if (conf->printEstimationStats) PRINT(VerbosityLevel::Error)
				    << "Estimation moot: " << res.exploredMoot << "\n"
				    << "Estimation blocked: " << res.exploredBlocked << "\n"
				    << "Estimation complete: " << res.explored << "\n";);
	}

	static std::unique_ptr<MiriGenMCShim> createHandle(const GenmcParams &config,
							   bool estimation_mode);

private:
	/** Increment the event index in the given thread by 1 and return the new event. */
	[[nodiscard]] inline auto incPos(ThreadId tid) -> Event
	{
		ERROR_ON(tid >= threadsAction.size(), "ThreadId out of bounds");
		return ++threadsAction[tid].event;
	}
	/** Decrement the event index in the given thread by 1 and return the new event. */
	inline auto decPos(ThreadId tid) -> Event
	{
		ERROR_ON(tid >= threadsAction.size(), "ThreadId out of bounds");
		return --threadsAction[tid].event;
	}

	/**
	 * @brief Try to insert the initial value of a memory location.
	 * @param addr
	 * @param value
	 * */
	void handleOldVal(const SAddr addr, GenmcScalar value)
	{
		LOG(VerbosityLevel::Warning)
			<< "handleOldVal: " << addr << ", " << value.value << ", " << value.extra
			<< ", " << value.is_init << "\n";

		// TODO GENMC(CLEANUP): Pass this as a parameter:
		auto &g = getExec().getGraph();
		auto *coLab = g.co_max(addr);
		if (auto *wLab = llvm::dyn_cast<WriteLabel>(coLab)) {
			LOG(VerbosityLevel::Warning)
				<< "handleOldVal: got WriteLabel, atomic: " << !wLab->isNotAtomic()
				<< "\n";
			if (!value.is_init)
				LOG(VerbosityLevel::Warning)
					<< "WARNING: TODO GENMC: handleOldVal tried to "
					   "overwrite value of NA "
					   "reads-from label, but old value is `uninit`\n";
			else if (wLab->isNotAtomic())
				wLab->setVal(value.toSVal());
		} else if (const auto *wLab = llvm::dyn_cast<InitLabel>(coLab)) {
			if (value.is_init) {
				auto result = initVals_.insert(std::make_pair(addr, value));
				LOG(VerbosityLevel::Warning)
					<< "handleOldVal: got InitLabel, insertion result: "
					<< result.first->second << ", " << result.second << "\n";
				BUG_ON(result.second &&
				       (*result.first).second !=
					       value); /* Attempt to replace initial value */
			} else {
				LOG(VerbosityLevel::Warning)
					<< "WARNING: TODO GENMC: handleOldVal tried set initial "
					   "value, but old "
					   "value is `uninit`\n";
			}
		} else {
			BUG(); /* Invalid label */
		}
		// either initLabel	==> update initValGetter
		// or WriteLabel    ==> Update its value in place (only if non-atomic)
	}

	/**
	 * Helper function for loads that need to reset the event counter when no value is returned.
	 * Same syntax as `GenMCDriver::handleLoad`, but this takes a thread id instead of an Event.
	 * Automatically calls `incPos` and `decPos` where needed for the given thread.
	 */
	template <EventLabel::EventLabelKind k, typename... Ts>
	HandleResult<SVal> handleLoadResetIfNone(std::function<void(SAddr)> oldValSetter,
						 ThreadId tid, Ts &&...params)
	{
		const auto pos = incPos(tid);
		const auto ret =
			GenMCDriver::handleLoad<k>(oldValSetter, pos, std::forward<Ts>(params)...);
		// If we didn't get a value, we reset the index of the current thread.
		if (!std::holds_alternative<SVal>(ret)) {
			decPos(tid);
		}
		return ret;
	}

	// TODO GENMC(mixed-size accesses):
	std::unordered_map<SAddr, GenmcScalar> initVals_{};

	/**
	 * Currently, the interpreter is responsible for maintaining `ExecutionGraph` event indices.
	 * The interpreter is also responsible for informing GenMC about the `ActionKind` of the
	 * next instruction in each thread.
	 *
	 * This vector contains this data in the expected format `Action`, which consists of the
	 * `ActionKind` of the next instruction and the last event index added to the ExecutionGraph
	 * in a given thread.
	 */
	std::vector<Action> threadsAction;

	/**
	 * Map of already used annotation ids (e.g., for mutexes).
	 * FIXME(GenMC): For multithreading support, this map and the counter below need to be
	 * synchronized across threads.
	 */
	std::unordered_map<uint64_t, ModuleID::ID> annotation_id{};
	ModuleID::ID annotation_id_counter = 0;
};

/**** Functions available to Miri ****/

// NOTE: CXX doesn't seem to support exposing static methods to Rust, so we expose this
// function instead
std::unique_ptr<MiriGenMCShim> createGenmcHandle(const GenmcParams &config, bool estimation_mode);

constexpr auto getGlobalAllocStaticMask() -> uint64_t { return SAddr::staticMask; }

#endif /* GENMC_MIRI_INTERFACE_HPP */
