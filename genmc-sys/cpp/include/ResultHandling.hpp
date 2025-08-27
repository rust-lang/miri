#ifndef GENMC_RESULT_HANDLING_HPP
#define GENMC_RESULT_HANDLING_HPP

// CXX.rs generated headers:
#include "Support/SVal.hpp"
#include "Verification/VerificationError.hpp"
#include "rust/cxx.h"

#include <cstdint>
#include <memory>
#include <ostream>
#include <string>

/** Information about an error, formatted as a string to avoid having to share an error enum and
 * printing functionality with the Rust side. */
using ModelCheckerError = std::string;

static auto format_error(VerificationError err) -> ModelCheckerError {
    auto buf = std::string();
    auto s = llvm::raw_string_ostream(buf);
    s << err;
    return s.str();
}

/**
 * This type is the Miri equivalent to GenMC's `SVal`, but with the addition of a field to mark the
 * value as uninitialized.
 */
struct GenmcScalar {
    uint64_t value;
    bool is_init;

    explicit GenmcScalar() : value(0), is_init(false) {}
    explicit GenmcScalar(uint64_t value) : value(value), is_init(true) {}
    explicit GenmcScalar(SVal val) : value(val.get()), is_init(true) {}

    /** Convert to a GenMC SVal. Panics if the value is uninitialized. */
    auto to_genmc_sval() const -> SVal {
        ERROR_ON(!is_init, "attempt to convert uninitialized GenmcScalar to an SVal\n");
        return SVal(value);
    }

    bool operator==(const GenmcScalar& other) const {
        // Treat uninitialized values as being equal.
        if (!is_init && !other.is_init)
            return true;

        // An initialized scalar is never equal to an uninitialized one.
        if (is_init != other.is_init)
            return false;

        // Compare the actual values
        return value == other.value;
    }

    friend auto operator<<(llvm::raw_ostream& rhs, const GenmcScalar& v) -> llvm::raw_ostream&;
};

/**** Types for scheduling queries. ****/

enum class ExecutionState : std::uint8_t {
    Ok,
    Blocked,
    Finished,
};

struct SchedulingResult {
    ExecutionState exec_state;
    int32_t next_thread;
};

/**** Types for event handling. ****/

struct LoadResult {
    /// If not null, contains the error encountered during the handling of the load.
    std::unique_ptr<ModelCheckerError> error;
    /// Indicates whether a value was read or not.
    bool has_value;
    /// The value that was read. Should not be used if `has_value` is `false`.
    GenmcScalar read_value;

  private:
    explicit LoadResult(bool has_value, SVal value)
        : has_value(true), read_value(GenmcScalar(value)), error(nullptr) {}
    explicit LoadResult(bool has_value)
        : has_value(has_value), read_value(GenmcScalar()), error(nullptr) {}
    explicit LoadResult(std::string error)
        : has_value(false), read_value(GenmcScalar()), error(std::make_unique<std::string>(error)) {
    }

  public:
    /**** Construction functions: ****/

    static LoadResult no_value() {
        return LoadResult(false);
    }
    static LoadResult from_value(SVal value) {
        return LoadResult(true, value);
    }
    static LoadResult from_error(VerificationError err) {
        return LoadResult(format_error(err));
    }
};

struct StoreResult {
    /// If not null, contains the error encountered during the handling of the store.
    std::unique_ptr<ModelCheckerError> error;
    /// `true` if the write should also be reflected in Miri's memory representation.
    bool is_coherence_order_maximal_write;

    static StoreResult ok(bool is_coherence_order_maximal_write) {
        return StoreResult { nullptr, is_coherence_order_maximal_write };
    }

    static StoreResult from_error(VerificationError err) {
        return StoreResult { /* error: */ std::make_unique<std::string>(format_error(err)),
                             /* is_coherence_order_maximal_write: */ false };
    }
};

#endif /* GENMC_RESULT_HANDLING_HPP */
