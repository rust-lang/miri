/** This file contains functionality related to handling mutexes.  */

#include "MiriInterface.hpp"

// Miri C++ helpers:
#include "Helper.hpp"

// CXX.rs generated headers:
#include "genmc-sys/src/lib.rs.h"

auto MiriGenMCShim::handleMutexLock(ThreadId thread_id, uint64_t address, uint64_t size)
	-> MutexLockResult
{
	// TODO GENMC: this needs to be identical even in multithreading
	ModuleID::ID annot_id;
	if (annotation_id.contains(address)) {
		annot_id = annotation_id.at(address);
	} else {
		annot_id = annotation_id_counter++;
		annotation_id.insert(std::make_pair(address, annot_id));
	}
	const auto aSize = ASize(size);
	auto annot = std::move(Annotation(
		AssumeType::Spinloop,
		Annotation::ExprVP(NeExpr<AnnotID>::create(
					   RegisterExpr<AnnotID>::create(aSize.getBits(), annot_id),
					   ConcreteExpr<AnnotID>::create(aSize.getBits(), SVal(1)))
					   .release())));

	// Mutex starts out unlocked, so we always say the previous value is "unlocked".
	auto oldValSetter = [this](SAddr addr) { this->handleOldVal(addr, GenmcScalar(0, 0)); };
	const auto ret = handleLoadResetIfNone<EventLabel::EventLabelKind::LockCasRead>(
		oldValSetter, thread_id, address, size, annot, EventDeps());
	RETURN_IF_ERROR(ret, MutexLockResult);

	const auto *retVal = std::get_if<SVal>(&ret);
	if (!retVal) {
		if (std::holds_alternative<Reset>(ret)) {
			// TODO TODO GENMC: what did I mean with this comment?
			// TODO GENMC: is_read_opt == Mutex is acquired
			// None	--> Someone else has lock, this thread will be rescheduled later
			// (currently block) 0	--> Got the lock 1 	--> Someone else has lock,
			// this thread will not be rescheduled later (block on Miri side)
			return MutexLockResult(false);
		}
		ERROR("Unimplemented: mutex lock returned unexpected result.");
	}

	const bool is_lock_acquired = *retVal == SVal(0);
	if (is_lock_acquired) {
		const auto ret = GenMCDriver::handleStore<EventLabel::EventLabelKind::LockCasWrite>(
			oldValSetter, incPos(thread_id), address, size, EventDeps());

		const auto *err = std::get_if<VerificationError>(&ret);
		if (err != nullptr) {
			return MutexLockResult::fromError(
				"TODO GENMC: format error once std::format change is merged");
		}
		ERROR_ON(!std::holds_alternative<std::monostate>(ret),
			 "Unsupported: mutex lock store returned unexpected result.");
	} else {
		GenMCDriver::handleAssume(incPos(thread_id), AssumeType::Spinloop);
	}

	return MutexLockResult(is_lock_acquired);
}

auto MiriGenMCShim::handleMutexTryLock(ThreadId thread_id, uint64_t address, uint64_t size)
	-> MutexLockResult
{
	const auto addr = SAddr(address);
	const auto aSize = ASize(size);

	auto &currPos = threadsAction[thread_id].event;
	// Mutex starts out unlocked, so we always say the previous value is "unlocked".
	auto oldValSetter = [this](SAddr addr) { this->handleOldVal(addr, GenmcScalar(0, 0)); };
	const auto ret0 = GenMCDriver::handleLoad<EventLabel::EventLabelKind::TrylockCasRead>(
		oldValSetter, ++currPos, addr, aSize);
	RETURN_IF_ERROR(ret0, MutexLockResult);

	const auto *retVal = std::get_if<SVal>(&ret0);
	if (nullptr == retVal) {
		// if (std::holds_alternative<Reset>(ret0)) {
		// 	// TODO TODO GENMC: what did I mean with this comment?
		// 	// TODO GENMC: is_read_opt == Mutex is acquired
		// 	// None	--> Someone else has lock, this thread will be rescheduled later
		// 	// (currently block) 0	--> Got the lock 1 	--> Someone else has lock,
		// 	// this thread will not be rescheduled later (block on Miri side)
		// 	return MutexLockResult(false);
		// }
		ERROR("Unimplemented: mutex trylock load returned unexpected result.");
	}

	const bool is_lock_acquired = *retVal == SVal(0);
	if (!is_lock_acquired)
		return MutexLockResult(false); /* Lock already held. */

	const auto ret1 = GenMCDriver::handleStore<EventLabel::EventLabelKind::TrylockCasWrite>(
		oldValSetter, ++currPos, addr, aSize);
	RETURN_IF_ERROR(ret1, MutexLockResult);

	if (!std::holds_alternative<std::monostate>(ret1)) {
		ERROR("Unimplemented: mutex trylock store returned unexpected result.");
	}
	// No error or unexpected result: lock is acquired.
	return MutexLockResult(true);
}

auto MiriGenMCShim::handleMutexUnlock(ThreadId thread_id, uint64_t address, uint64_t size)
	-> StoreResult
{
	const auto pos = incPos(thread_id);
	const auto addr = SAddr(address);
	const auto aSize = ASize(size);

	const auto oldValSetter = [this](SAddr addr) {
		// TODO GENMC(HACK): is this the best way to do it?
		this->handleOldVal(addr, GenmcScalar(0xDEADBEEF, 0));
	};
	const auto ret = GenMCDriver::handleStore<EventLabel::EventLabelKind::UnlockWrite>(
		oldValSetter, pos, MemOrdering::Release, addr, aSize, AType::Signed, SVal(0),
		EventDeps());
	RETURN_IF_ERROR(ret, StoreResult);

	if (!std::holds_alternative<std::monostate>(ret)) {
		ERROR("Unimplemented: mutex unlock store returned unexpected result.");
	}

	// TODO GENMC: Mixed-accesses (`false` should be fine, since we never want to update Miri's
	// memory for mutexes anyway)
	// const bool isCoMaxWrite = false;
	const auto &g = getExec().getGraph();
	const bool isCoMaxWrite = g.co_max(addr)->getPos() == pos;
	return StoreResult::ok(isCoMaxWrite);
}
