use genmc_sys::LogLevel;

use super::GenmcParams;

/// Configuration for GenMC mode.
/// The `params` field is shared with the C++ side.
/// The remaining options are kept on the Rust side.
#[derive(Debug, Default, Clone)]
pub struct GenmcConfig {
    /// Parameters sent to the C++ side to create a new handle to the GenMC model checker.
    pub(super) params: GenmcParams,
    /// Print the output message that GenMC generates when an error occurs.
    /// This error message is currently hard to use, since there is no clear mapping between the events that GenMC sees and the Rust code location where this event was produced.
    pub(super) print_genmc_output: bool,
    /// The log level for GenMC.
    pub(super) log_level: LogLevel,
}

impl GenmcConfig {
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
            genmc_config.log_level = log_level.parse()?;
        } else if let Some(trimmed_arg) = trimmed_arg.strip_prefix("print-exec-graphs") {
            use genmc_sys::ExecutiongraphPrinting;
            genmc_config.params.print_execution_graphs = match trimmed_arg {
                "=none" => ExecutiongraphPrinting::None,
                // Make GenMC print explored executions.
                "" | "=explored" => ExecutiongraphPrinting::Explored,
                // Make GenMC print blocked executions.
                "=blocked" => ExecutiongraphPrinting::Blocked,
                // Make GenMC print all executions.
                "=all" => ExecutiongraphPrinting::ExploredAndBlocked,
                _ =>
                    return Err(format!(
                        "Invalid suffix to GenMC argument '-Zmiri-genmc-print-exec-graphs', expected '', '=none', '=explored', '=blocked' or '=all'"
                    )),
            }
        } else if trimmed_arg == "print-genmc-output" {
            genmc_config.print_genmc_output = true;
        } else {
            return Err(format!("Invalid GenMC argument: \"-Zmiri-genmc-{trimmed_arg}\""));
        }
        Ok(())
    }
}
