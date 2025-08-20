#ifndef GENMC_MIRI_HELPER
#define GENMC_MIRI_HELPER

/** If `ret` contains a `VerificationError`, return an `ErrorType::fromError(err)`. */
#define RETURN_IF_ERROR(ret, ErrorType)                                                            \
	do {                                                                                       \
		const auto *err = std::get_if<VerificationError>(&ret);                            \
		if (nullptr != err)                                                                \
			return ErrorType::fromError("FIXME(GenMC): show actual error here.");      \
	} while (0)

#endif /* GENMC_MIRI_HELPER */