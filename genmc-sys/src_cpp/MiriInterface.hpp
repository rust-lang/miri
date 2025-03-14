#ifndef GENMC_GENMC_MIRI_INTERFACE_HPP
#define GENMC_GENMC_MIRI_INTERFACE_HPP

#include "rust/cxx.h"

#include "ExecutionGraph/EventLabel.hpp"
#include "Support/MemOrdering.hpp"
#include "Support/RMWOps.hpp"
#include "Verification/GenMCDriver.hpp"
#include "Verification/VerificationConfig.hpp"

#include <cstdint>
#include <iomanip>
#include <memory>

/**** Types available to Miri ****/

struct GenmcParams;

using ThreadId = int;

enum class StoreEventType : uint8_t {
	Normal,
	ReadModifyWrite,
	CompareExchange,
	MutexUnlockWrite,
};

struct MutexLockResult {
	bool is_lock_acquired;
	std::unique_ptr<ModelCheckerError> error; // TODO GENMC: pass more error info here

	MutexLockResult(bool is_lock_acquired) : is_lock_acquired(is_lock_acquired), error(nullptr)
	{}

	static auto fromError(std::string msg) -> MutexLockResult
	{
		auto res = MutexLockResult(false);
		res.error = std::make_unique<ModelCheckerError>(msg);
		return res;
	}
};

// TODO GENMC: fix naming conventions

struct MiriGenMCShim : private GenMCDriver {

public:
	MiriGenMCShim(std::shared_ptr<const VerificationConfig> vConf, Mode mode /* = VerificationMode{} */)
		: GenMCDriver(std::move(vConf), nullptr, mode)
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
					      MemOrdering ord, StoreEventType store_event_type);

	void handleFence(ThreadId thread_id, MemOrdering ord);

	/**** Memory (de)allocation ****/

	uintptr_t handleMalloc(ThreadId thread_id, uint64_t size, uint64_t alignment);
	void handleFree(ThreadId thread_id, uint64_t address, uint64_t size);

	/**** Thread management ****/

	void handleThreadCreate(ThreadId thread_id, ThreadId parent_id);
	void handleThreadJoin(ThreadId thread_id, ThreadId child_id);
	void handleThreadFinish(ThreadId thread_id, uint64_t ret_val);

	/**** Blocking instructions ****/

	void handleUserBlock(ThreadId thread_id);

	/**** Mutex handling ****/
	auto handleMutexLock(ThreadId thread_id, uint64_t address, uint64_t size)
		-> MutexLockResult;
	auto handleMutexTryLock(ThreadId thread_id, uint64_t address, uint64_t size)
		-> MutexLockResult;
	auto handleMutexUnlock(ThreadId thread_id, uint64_t address, uint64_t size) -> StoreResult;

	/**** Scheduling queries ****/

	// TODO GENMC: implement

	auto scheduleNext(const int curr_thread_id, const ActionKind curr_thread_next_instr_kind)
		-> int64_t;

	/**** TODO GENMC: Other stuff: ****/

	auto getStuckExecutionCount() const -> uint64_t
	{
		return static_cast<uint64_t>(getResult().exploredBlocked);
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

	void printGraph() { GenMCDriver::debugPrintGraph(); }

	void printEstimationResults(const double elapsed_time_sec) const
	{
		// TODO GENMC(CLEANUP): should this happen on the Rust side?
		const auto &res = getResult();
		const auto *vConf = getConf();

		auto mean = std::llround(res.estimationMean);
		auto sd = std::llround(std::sqrt(res.estimationVariance));
		auto meanTimeSecs = (long double)elapsed_time_sec / (res.explored + res.exploredBlocked);
		// FIXME(io): restore the old precision after the print?
		PRINT(VerbosityLevel::Error)
			<< "Finished estimation in " << std::setprecision(2) << elapsed_time_sec << " seconds.\n\n"
			<< "Total executions estimate: " << mean << " (+- " << sd << ")\n"
			<< "Time to completion estimate: "
			 << std::setprecision(2) << (meanTimeSecs * mean) << "s\n";
		GENMC_DEBUG(if (vConf->printEstimationStats) PRINT(VerbosityLevel::Error)
				    << "Estimation moot: " << res.exploredMoot << "\n"
				    << "Estimation blocked: " << res.exploredBlocked << "\n"
				    << "Estimation complete: " << res.explored << "\n";);
	}

	static std::unique_ptr<MiriGenMCShim> createHandle(const GenmcParams &config,
							   bool estimation_mode);

private:
	/**
	 * @brief Try to insert the initial value of a memory location.
	 * @param addr
	 * @param value
	 * */
	void handleOldVal(const SAddr addr, GenmcScalar value)
	{
		MIRI_LOG() << "handleOldVal: " << addr << ", " << value.value << ", " << value.extra
			   << ", " << value.is_init << "\n";
		// if (!value.is_init) {
		// 	// // TODO GENMC(uninit value handling)
		// 	// MIRI_LOG() << "WARNING: got uninitialized old value, ignoring ...\n";
		// 	// return;
		// 	MIRI_LOG() << "WARNING: got uninitialized old value, converting to dummy "
		// 		      "value ...\n";
		// 	value.is_init = true;
		// 	value.value = 0xAAFFAAFF;
		// }

		// TODO GENMC(CLEANUP): Pass this as a parameter:
		auto &g = getExec().getGraph();
		auto *coLab = g.co_max(addr);
		MIRI_LOG() << "handleOldVal: coLab: " << *coLab << "\n";
		if (auto *wLab = llvm::dyn_cast<WriteLabel>(coLab)) {
			MIRI_LOG() << "handleOldVal: got WriteLabel, atomic: " << wLab->isAtomic()
				   << "\n";
			if (!value.is_init)
				MIRI_LOG() << "WARNING: TODO GENMC: handleOldVal tried to "
					      "overwrite value of NA "
					      "reads-from label, but old value is `uninit`\n";
			else if (wLab->isNotAtomic())
				wLab->setVal(value.toSVal());
		} else if (const auto *wLab = llvm::dyn_cast<InitLabel>(coLab)) {
			if (value.is_init) {
				auto result = initVals_.insert(std::make_pair(addr, value));
				MIRI_LOG() << "handleOldVal: got InitLabel, insertion result: "
					   << result.first->second << ", " << result.second << "\n";
				BUG_ON(result.second &&
				       (*result.first).second !=
					       value); /* Attempt to replace initial value */
			} else {
				// LOG(VerbosityLevel::Error) <<
				MIRI_LOG() << "WARNING: TODO GENMC: handleOldVal tried set initial "
					      "value, but old "
					      "value is `uninit`\n";
			}
		} else {
			BUG(); /* Invalid label */
		}
		// either initLabel	==> update initValGetter
		// or WriteLabel    ==> Update its value in place (only if non-atomic)
	}

	// TODO GENMC(mixed-size accesses):
	std::unordered_map<SAddr, GenmcScalar> initVals_{};

	std::vector<Action> globalInstructions;

	std::unordered_map<uint64_t, unsigned int> annotation_id{};
	unsigned int annotation_id_counter = 0;
};

/**** Functions available to Miri ****/

// NOTE: CXX doesn't seem to support exposing static methods to Rust, so we expose this
// function instead
std::unique_ptr<MiriGenMCShim> createGenmcHandle(const GenmcParams &config, bool estimation_mode);

constexpr auto getGlobalAllocStaticMask() -> uint64_t { return SAddr::staticMask; }

#endif /* GENMC_GENMC_MIRI_INTERFACE_HPP */
