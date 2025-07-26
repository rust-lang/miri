#ifndef GENMC_RESULT_HANDLING_HPP
#define GENMC_RESULT_HANDLING_HPP

#include "Support/SVal.hpp"

#include <cstdint>
#include <memory>
#include <ostream>
#include <string>

/** Information about an error, formatted as a string to avoid having to share an error enum and
 * printing functionality with the Rust side. */
using ModelCheckerError = std::string;

/**
 * This type is the Miri equivalent to GenMC's `SVal`, but with the addition of a field to mark the
 * value as uninitialized.
 */
struct GenmcScalar {
	uint64_t value;
	bool is_init;

	explicit GenmcScalar() : value(0), is_init(false) {}
	explicit GenmcScalar(uint64_t value, uint64_t extra)
		: value(value), is_init(true)
	{}
	explicit GenmcScalar(SVal val) : value(val.get()), is_init(true) {}

	/** Convert to a GenMC SVal. Panics if the value is uninitialized. */
	auto toSVal() const -> SVal
	{
		ERROR_ON(!is_init, "attempt to convert uninitialized GenmcScalar to an SVal\n");
		return SVal(value);
	}

	bool operator==(const GenmcScalar &other) const
	{
		// Treat uninitialized values as being equal.
		if (!is_init && !other.is_init)
			return true;

		// An initialized scalar is never equal to an uninitialized one.
		if (is_init != other.is_init)
			return false;

		// Compare the actual values
		return value == other.value;
	}

	friend auto operator<<(llvm::raw_ostream &rhs, const GenmcScalar &v) -> llvm::raw_ostream &;
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
	/// If there is an error, it will be stored in `error`, otherwise it is `None`
	std::unique_ptr<ModelCheckerError> error;
	/// Indicates whether a value was read or not.
	bool has_value;
	/// The value that was read. Should not be used if `has_value` is `false`.
	GenmcScalar read_value;

private:
	explicit LoadResult(bool has_value, SVal value)
		: has_value(true), read_value(GenmcScalar(value)), error(nullptr)
	{}
	explicit LoadResult(bool has_value)
		: has_value(has_value), read_value(GenmcScalar()), error(nullptr)
	{}
	explicit LoadResult(std::string error)
		: has_value(false), read_value(GenmcScalar()),
		  error(std::make_unique<ModelCheckerError>(error))
	{}

public:
	/**** Construction functions: ****/

	static LoadResult noValue() { return LoadResult(false); }
	static LoadResult fromValue(SVal value) { return LoadResult(true, value); }
	static LoadResult fromError(std::string msg) { return LoadResult(msg); }

	/**** Operators: ****/

	LoadResult &operator=(const LoadResult &rhs)
	{
		has_value = rhs.has_value;
		read_value = rhs.read_value;
		if (rhs.error.get() != nullptr) {
			error = std::make_unique<ModelCheckerError>(*rhs.error);
		}
		else
		{
			error = nullptr;
		}
		return *this;
	}
};

struct StoreResult {
	/// if not `nullptr`, it contains an error encountered during the handling of the store.
	std::unique_ptr<ModelCheckerError> error;
	/// `true` if the write should also be reflected in Miri's memory representation.
	bool isCoMaxWrite;

	static StoreResult ok(bool isCoMaxWrite) { return StoreResult{nullptr, isCoMaxWrite}; }

	static StoreResult fromError(std::string msg)
	{
		auto store_result = StoreResult{};
		store_result.error = std::make_unique<ModelCheckerError>(msg);
		return store_result;
	}
};

#endif /* GENMC_RESULT_HANDLING_HPP */
