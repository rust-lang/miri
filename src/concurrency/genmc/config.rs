use super::GenmcParams;

/// Configuration for GenMC mode.
/// The `params` field is shared with the C++ side.
/// The remaining options are kept on the Rust side.
#[derive(Debug, Default, Clone)]
pub struct GenmcConfig {
    pub(super) params: GenmcParams,
    print_exec_graphs: bool,
    do_estimation: bool,
}

impl GenmcConfig {
    pub fn print_exec_graphs(&self) -> bool {
        self.print_exec_graphs
    }

    pub fn do_estimation(&self) -> bool {
        self.do_estimation
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
        if let Some(log_level) = trimmed_arg.strip_prefix("log=") {
            genmc_config.params.log_level = log_level.parse()?;
        } else if trimmed_arg == "print-graphs" {
            // TODO GENMC (DOCUMENTATION)
            genmc_config.print_exec_graphs = true;
        } else if trimmed_arg == "estimate" {
            // TODO GENMC (DOCUMENTATION): naming, off/on by default?
            genmc_config.do_estimation = true;
        } else if let Some(estimation_max_str) = trimmed_arg.strip_prefix("estimation-max=") {
            // TODO GENMC (DOCUMENTATION)
            let Some(estimation_max) =
                estimation_max_str.parse().ok().filter(|estimation_max| *estimation_max > 0)
            else {
                return Err(format!(
                    "-Zmiri-genmc-estimation-max expects a positive integer argument, but got '{estimation_max_str}'"
                ));
            };
            genmc_config.params.estimation_max = estimation_max;
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
