use super::GenmcParams;

/// Configuration for GenMC mode.
/// The `params` field is shared with the C++ side.
/// The remaining options are kept on the Rust side.
#[derive(Debug, Default, Clone)]
pub struct GenmcConfig {
    pub(super) params: GenmcParams,
}

impl GenmcConfig {
    fn set_log_level_trace(&mut self) {
        self.params.quiet = false;
        self.params.log_level_trace = true;
    }

    /// Function for parsing command line options for GenMC mode.
    ///
    /// All GenMC arguments start with the string "-Zmiri-genmc".
    /// Passing any GenMC argument will enable GenMC mode.
    ///
    /// `trimmed_arg` should be the argument to be parsed, with the suffix "-Zmiri-genmc" removed.
    pub fn parse_arg(
        genmc_config: &mut Option<GenmcConfig>,
        trimmed_arg: &str,
    ) -> Result<(), String> {
        // FIXME(genmc): Ensure host == target somewhere.

        if genmc_config.is_none() {
            *genmc_config = Some(Default::default());
        }
        if trimmed_arg.is_empty() {
            return Ok(()); // this corresponds to "-Zmiri-genmc"
        }
        let genmc_config = genmc_config.as_mut().unwrap();
        let Some(trimmed_arg) = trimmed_arg.strip_prefix("-") else {
            return Err(format!("Invalid GenMC argument \"-Zmiri-genmc{trimmed_arg}\""));
        };
        if trimmed_arg == "log-trace" {
            // TODO GENMC: maybe expand to allow more control over log level?
            genmc_config.set_log_level_trace();
        } else if trimmed_arg == "symmetry-reduction" {
            // TODO GENMC (PERFORMANCE): maybe make this the default, have an option to turn it off instead
            genmc_config.params.do_symmetry_reduction = true;
        } else {
            // TODO GENMC: how to properly handle this?
            return Err(format!("Invalid GenMC argument: \"-Zmiri-genmc-{trimmed_arg}\""));
        }
        Ok(())
    }
}
