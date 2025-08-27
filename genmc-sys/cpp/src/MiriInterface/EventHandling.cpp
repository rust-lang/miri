/** This file contains functionality related to handling events encountered
 * during an execution, such as loads, stores or memory (de)allocation. */

#include "MiriInterface.hpp"

// CXX.rs generated headers:
#include "genmc-sys/src/lib.rs.h"

// GenMC headers:
#include "ADT/value_ptr.hpp"
#include "ExecutionGraph/EventLabel.hpp"
#include "ExecutionGraph/LoadAnnotation.hpp"
#include "Runtime/InterpreterEnumAPI.hpp"
#include "Static/ModuleID.hpp"
#include "Support/ASize.hpp"
#include "Support/Error.hpp"
#include "Support/Logger.hpp"
#include "Support/MemAccess.hpp"
#include "Support/RMWOps.hpp"
#include "Support/SAddr.hpp"
#include "Support/SVal.hpp"
#include "Support/ThreadInfo.hpp"
#include "Support/Verbosity.hpp"
#include "Verification/GenMCDriver.hpp"
#include "Verification/MemoryModel.hpp"

// C++ headers:
#include <cstddef>
#include <cstdint>
#include <memory>
#include <utility>

/**** Memory access handling ****/

[[nodiscard]] auto MiriGenmcShim::handle_load(
    ThreadId thread_id,
    uint64_t address,
    uint64_t size,
    MemOrdering ord,
    GenmcScalar old_val
) -> LoadResult {
    // `type` is only used for printing.
    const auto type = AType::Unsigned;
    const auto ret = handle_load_reset_if_none<EventLabel::EventLabelKind::Read>(
        thread_id,
        ord,
        SAddr(address),
        ASize(size),
        type
    );

    if (const auto* err = std::get_if<VerificationError>(&ret))
        return LoadResult::from_error(*err);
    const auto* ret_val = std::get_if<SVal>(&ret);
    if (ret_val != nullptr)
        return LoadResult::from_value(*ret_val);
    ERROR("Unimplemented: load returned unexpected result.");
}

[[nodiscard]] auto MiriGenmcShim::handle_store(
    ThreadId thread_id,
    uint64_t address,
    uint64_t size,
    GenmcScalar value,
    GenmcScalar old_val,
    MemOrdering ord
) -> StoreResult {
    auto pos = inc_pos(thread_id);

    auto addr = SAddr(address);
    // `type` is only used for printing.
    auto type = AType::Unsigned;
    const auto ret = GenMCDriver::handleStore<EventLabel::EventLabelKind::Write>(
        pos,
        ord,
        addr,
        ASize(size),
        type,
        value.to_genmc_sval(),
        EventDeps()
    );

    if (const auto* err = std::get_if<VerificationError>(&ret))
        return StoreResult::from_error(*err);
    if (!std::holds_alternative<std::monostate>(ret))
        ERROR("store returned unexpected result");

    // FIXME(mixed-accesses): calculate this value
    const auto& g = getExec().getGraph();
    const bool isCoMaxWrite = g.co_max(addr)->getPos() == pos;
    return StoreResult::ok(isCoMaxWrite);
}

/**** Memory (de)allocation ****/

auto MiriGenmcShim::handle_malloc(ThreadId thread_id, uint64_t size, uint64_t alignment)
    -> uint64_t {
    auto pos = inc_pos(thread_id);

    // These are only used for printing and features Miri-GenMC doesn't support (yet).
    auto sd = StorageDuration::SD_Heap;
    auto stype = StorageType::ST_Volatile;
    auto spc = AddressSpace::AS_User;

    const SVal ret_val =
        GenMCDriver::handleMalloc(pos, size, alignment, sd, stype, spc, EventDeps());
    return ret_val.get();
}

void MiriGenmcShim::handle_free(ThreadId thread_id, uint64_t address) {
    const auto pos = inc_pos(thread_id);
    GenMCDriver::handleFree(pos, SAddr(address), EventDeps());
}
