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
	uint64_t extra;
	bool is_init;

	explicit GenmcScalar() : value(0), extra(0), is_init(false) {}
	explicit GenmcScalar(uint64_t value, uint64_t extra)
		: value(value), extra(extra), is_init(true)
	{}
	explicit GenmcScalar(SVal val) : value(val.get()), extra(val.getExtra()), is_init(true) {}

	/** Convert to a GenMC SVal. Panics if the value is uninitialized. */
	auto toSVal() const -> SVal
	{
		ERROR_ON(!is_init, "attempt to convert uninitialized GenmcScalar to an SVal\n");
		return SVal(value, extra);
	}

	bool operator==(const GenmcScalar &other) const
	{
		// Treat uninitialized values as being equal.
		if (!is_init && !other.is_init)
			return true;

		// An initialized scalar is never equal to an uninitialized one.
		if (is_init != other.is_init)
			return false;

		// Compare the actual values.
		return value == other.value && extra == other.extra;
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
		} else {
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

struct ReadModifyWriteResult {
	/// if not `nullptr`, it contains an error encountered during the handling of the RMW.
	std::unique_ptr<ModelCheckerError> error;
	/// The value that was read by the RMW operation as the left operand.
	GenmcScalar old_value;
	/// The value that was produced by the RMW operation.
	GenmcScalar new_value;
	/// `true` if the write should also be reflected in Miri's memory representation.
	bool isCoMaxWrite;

private:
	ReadModifyWriteResult(std::string msg) : error(std::make_unique<ModelCheckerError>(msg))
	{
		old_value = GenmcScalar();
		new_value = GenmcScalar();
	}

public:
	ReadModifyWriteResult(SVal old_value, SVal new_value, bool isCoMaxWrite)
		: old_value(GenmcScalar(old_value)), new_value(GenmcScalar(new_value)),
		  isCoMaxWrite(isCoMaxWrite), error(nullptr)
	{}

	static ReadModifyWriteResult fromError(std::string msg)
	{
		return ReadModifyWriteResult(msg);
	}
};

struct CompareExchangeResult {
	/// if not `nullptr`, it contains an error encountered during the handling of the RMW op.
	std::unique_ptr<ModelCheckerError> error;
	/// The value that was read by the compare-exchange.
	GenmcScalar old_value;
	/// `true` if compare_exchange op was successful.
	bool is_success;
	/// `true` if the write should also be reflected in Miri's memory representation.
	bool isCoMaxWrite;

	static CompareExchangeResult success(SVal old_value, bool isCoMaxWrite)
	{
		return CompareExchangeResult{nullptr, GenmcScalar(old_value), true, isCoMaxWrite};
	}

	static CompareExchangeResult failure(SVal old_value)
	{
		return CompareExchangeResult{nullptr, GenmcScalar(old_value), false, false};
	}

	static CompareExchangeResult fromError(std::string msg)
	{
		const auto dummy_scalar = GenmcScalar();
		return CompareExchangeResult{std::make_unique<ModelCheckerError>(msg), dummy_scalar,
					     false, false};
	}
};

struct MutexLockResult {
	/// if not `nullptr`, it contains an error encountered during the handling of the mutex op.
	std::unique_ptr<ModelCheckerError> error;
	/// Indicate whether the lock was acquired by this thread.
	bool is_lock_acquired;

	MutexLockResult(bool is_lock_acquired) : is_lock_acquired(is_lock_acquired), error(nullptr)
	{}

	static auto fromError(std::string msg) -> MutexLockResult
	{
		auto res = MutexLockResult(false);
		res.error = std::make_unique<ModelCheckerError>(msg);
		return res;
	}
};

#endif /* GENMC_RESULT_HANDLING_HPP */
