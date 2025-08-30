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
struct GenmcScalar;
struct SchedulingResult;
struct LoadResult;
struct StoreResult;

// GenMC uses `int` for its thread IDs.
using ThreadId = int;

struct MiriGenmcShim : private GenMCDriver {

  public:
    MiriGenmcShim(std::shared_ptr<const Config> conf, Mode mode /* = VerificationMode{} */)
        : GenMCDriver(std::move(conf), nullptr, mode) {}

    virtual ~MiriGenmcShim() {}

    /**** Execution start/end handling ****/

    // This function must be called at the start of any execution, before any events are
    // reported to GenMC.
    void handle_execution_start();
    // This function must be called at the end of any execution, even if an error was found
    // during the execution.
    // Returns `null`, or a string containing an error message if an error occured.
    std::unique_ptr<std::string> handle_execution_end();

    /***** Functions for handling events encountered during program execution. *****/

    /**** Memory access handling ****/

    [[nodiscard]] LoadResult handle_load(
        ThreadId thread_id,
        uint64_t address,
        uint64_t size,
        MemOrdering ord,
        GenmcScalar old_val
    );
    [[nodiscard]] StoreResult handle_store(
        ThreadId thread_id,
        uint64_t address,
        uint64_t size,
        GenmcScalar value,
        GenmcScalar old_val,
        MemOrdering ord
    );

    /**** Memory (de)allocation ****/
    auto handle_malloc(ThreadId thread_id, uint64_t size, uint64_t alignment) -> uint64_t;
    void handle_free(ThreadId thread_id, uint64_t address);

    /**** Thread management ****/
    void handle_thread_create(ThreadId thread_id, ThreadId parent_id);
    void handle_thread_join(ThreadId thread_id, ThreadId child_id);
    void handle_thread_finish(ThreadId thread_id, uint64_t ret_val);
    void handle_thread_kill(ThreadId thread_id);

    /***** Exploration related functionality *****/

    /** Ask the GenMC scheduler for a new thread to schedule and return whether the execution is
     * finished, blocked, or can continue.
     * Updates the next instruction kind for the given thread id. */
    auto schedule_next(const int curr_thread_id, const ActionKind curr_thread_next_instr_kind)
        -> SchedulingResult;

    /**
     * Check whether there are more executions to explore.
     * If there are more executions, this method prepares for the next execution and returns
     * `true`. Returns true if there are no more executions to explore. */
    auto is_exploration_done() -> bool {
        return GenMCDriver::done();
    }

    /**** Result querying functionality. ****/

    // NOTE: We don't want to share the `VerificationResult` type with the Rust side, since it
    // is very large, uses features that CXX.rs doesn't support and may change as GenMC changes.
    // Instead, we only use the result on the C++ side, and only expose these getter function to
    // the Rust side.

    // Note that CXX.rs doesn't support returning a C++ string to Rust by value,
    // it must be behind an indirection like a `unique_ptr` (tested with CXX 1.0.170).

    /// Get the number of blocked executions encountered by GenMC (cast into a fixed with
    /// integer)
    auto get_blocked_execution_count() const -> uint64_t {
        return static_cast<uint64_t>(getResult().exploredBlocked);
    }

    /// Get the number of executions explored by GenMC (cast into a fixed with integer)
    auto get_explored_execution_count() const -> uint64_t {
        return static_cast<uint64_t>(getResult().explored);
    }

    /// Get all messages that GenMC produced (errors, warnings), combined into one string.
    auto get_result_message() const -> std::unique_ptr<std::string> {
        return std::make_unique<std::string>(getResult().message);
    }

    /// If an error occurred, return a string describing the error, otherwise, return `nullptr`.
    auto get_error_string() const -> std::unique_ptr<std::string> {
        const auto& result = GenMCDriver::getResult();
        if (result.status.has_value())
            return format_error(result.status.value());
        return nullptr;
    }

    static auto create_handle(const GenmcParams& params) -> std::unique_ptr<MiriGenmcShim>;

  private:
    /** Increment the event index in the given thread by 1 and return the new event. */
    [[nodiscard]] inline auto inc_pos(ThreadId tid) -> Event {
        ERROR_ON(tid >= threads_action_.size(), "ThreadId out of bounds");
        return ++threads_action_[tid].event;
    }
    /** Decrement the event index in the given thread by 1 and return the new event. */
    inline auto dec_pos(ThreadId tid) -> Event {
        ERROR_ON(tid >= threads_action_.size(), "ThreadId out of bounds");
        return --threads_action_[tid].event;
    }

    /**
     * Helper function for loads that need to reset the event counter when no value is returned.
     * Same syntax as `GenMCDriver::handleLoad`, but this takes a thread id instead of an Event.
     * Automatically calls `inc_pos` and `dec_pos` where needed for the given thread.
     */
    template <EventLabel::EventLabelKind k, typename... Ts>
    auto handle_load_reset_if_none(ThreadId tid, Ts&&... params) -> HandleResult<SVal> {
        const auto pos = inc_pos(tid);
        const auto ret = GenMCDriver::handleLoad<k>(pos, std::forward<Ts>(params)...);
        // If we didn't get a value, we reset the index of the current thread.
        if (!std::holds_alternative<SVal>(ret)) {
            dec_pos(tid);
        }
        return ret;
    }

    /**
     * GenMC uses the term `Action` to refer to a struct of:
     * - `ActionKind`, storing whether the next instruction in a thread may be a load
     * - `Event`, storing the most recent event index added for a thread
     *
     * Here we store the "action" for each thread. In particular we use this to assign event
     * indices, since GenMC expects us to do that.
     */
    std::vector<Action> threads_action_;
};

/**** Functions available to Miri ****/

constexpr auto get_global_alloc_static_mask() -> uint64_t {
    return SAddr::staticMask;
}

#endif /* GENMC_MIRI_INTERFACE_HPP */
