#include "MiriInterface.hpp"

#include "genmc-sys/src/lib.rs.h"

auto MiriGenMCShim::createHandle(const GenmcParams &config)
	-> std::unique_ptr<MiriGenMCShim>
{
	auto vConf = std::make_shared<VerificationConfig>();

	// Miri needs all threads to be replayed, even fully completed ones.
	vConf->replayCompletedThreads = true;

	// We only support the RC11 memory model for Rust.
	vConf->model = ModelType::RC11;

	vConf->printRandomScheduleSeed = config.print_random_schedule_seed;

	// FIXME(GenMC): disable any options we don't support currently:
	vConf->ipr = false;
	vConf->disableBAM = true;
	vConf->instructionCaching = false;

	ERROR_ON(config.do_symmetry_reduction, "Symmetry reduction is currently unsupported in GenMC mode.");
	vConf->symmetryReduction = config.do_symmetry_reduction;

	// FIXME(GenMC): Should there be a way to change this option from Miri?
	vConf->schedulePolicy = SchedulePolicy::WF;

	// FIXME(GenMC): implement estimation mode:
	vConf->estimate = false;
	vConf->estimationMax = 1000;
	const auto mode = vConf->estimate ? GenMCDriver::Mode(GenMCDriver::EstimationMode{})
									  : GenMCDriver::Mode(GenMCDriver::VerificationMode{});

	// Running Miri-GenMC without race detection is not supported.
	// Disabling this option also changes the behavior of the replay scheduler to only schedule at atomic operations, which is required with Miri.
	// This happens because Miri can generate multiple GenMC events for a single MIR terminator. Without this option,
	// the scheduler might incorrectly schedule an atomic MIR terminator because the first event it creates is a non-atomic (e.g., `StorageLive`).
	vConf->disableRaceDetection = false;

	// Miri can already check for unfreed memory. Also, GenMC cannot distinguish between memory
	// that is allowed to leak and memory that is not.
	vConf->warnUnfreedMemory = false;

	checkVerificationConfigOptions(*vConf);

	auto driver = std::make_unique<MiriGenMCShim>(std::move(vConf), mode);
	return driver;
}
