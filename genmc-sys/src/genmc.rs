use std::sync::OnceLock;

use cxx::UniquePtr;

use super::{GenmcParams, LogLevel, MiriGenmcShim};

static GENMC_LOG_LEVEL: OnceLock<LogLevel> = OnceLock::new();

#[derive(Clone, Copy)]
/// This struct is used to prevent unsafe access to GenMC.
/// Any operations that create a GenMC object or call a GenMC function should go through this struct.
pub struct Genmc {}

impl Genmc {
    pub fn new(genmc_log_level: LogLevel) -> Self {
        assert_eq!(
            &genmc_log_level,
            GENMC_LOG_LEVEL.get_or_init(|| {
                unsafe {
                    // SAFETY
                    // We only ever call this function once, before any GenMC functions can be called.
                    // Creating any handle to GenMC requires a `struct GenMC`, which can only be returned from this `new` function.
                    // Since the log level never changes after this point, no data races can occur.
                    super::set_log_level_raw(genmc_log_level);
                }
                genmc_log_level
            }),
            "Attempt to change the GenMC log level after it was already set"
        );
        Self {}
    }

    pub fn create_driver_handle(&self, params: &GenmcParams) -> UniquePtr<MiriGenmcShim> {
        // SAFETY
        // We only call `create_handle` after we've created a `struct Genmc`, which sets the log level safely.
        // Since the log level will only ever be read after this point, never written to, we can safely create as many `MiriGenmcShim` here as we want.
        unsafe { MiriGenmcShim::create_handle(params) }
    }
}
