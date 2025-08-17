#ifndef GENMC_MIRI_LOG_LEVEL_HPP
#define GENMC_MIRI_LOG_LEVEL_HPP

// CXX.rs generated headers:
#include "genmc-sys/src/lib.rs.h"

// GenMC headers:
#include "Support/Error.hpp"
#include "Support/Verbosity.hpp"

// C++ headers:
#include <cstdint>

/**
 * Translate the Miri-GenMC `LogLevel` to the GenMC `VerbosityLevel`.
 * Downgrade any debug options to `Tip` if `ENABLE_GENMC_DEBUG` is not enabled.
 */
auto to_genmc_verbosity_level(const LogLevel log_level) -> VerbosityLevel
{
	switch (log_level) {
	case LogLevel::Quiet:
		return VerbosityLevel::Quiet;
	case LogLevel::Error:
		return VerbosityLevel::Error;
	case LogLevel::Warning:
		return VerbosityLevel::Warning;
	case LogLevel::Tip:
		return VerbosityLevel::Tip;
#ifdef ENABLE_GENMC_DEBUG
	case LogLevel::Debug1Revisits:
		return VerbosityLevel::Debug1;
	case LogLevel::Debug2MemoryAccesses:
		return VerbosityLevel::Debug2;
	case LogLevel::Debug3ReadsFrom:
		return VerbosityLevel::Debug3;
#else
	// Downgrade to `Tip` if the debug levels are not available.
	case LogLevel::Debug1Revisits:
	case LogLevel::Debug2MemoryAccesses:
	case LogLevel::Debug3ReadsFrom:
		return VerbosityLevel::Tip;
#endif
	default:
		WARN_ONCE("unknown-log-level",
			  "Unknown `LogLevel`, defaulting to `VerbosityLevel::Tip`.");
		return VerbosityLevel::Tip;
	}
}

#endif /* GENMC_MIRI_LOG_LEVEL_HPP */
