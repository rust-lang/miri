use super::GenmcParams;

// TODO GENMC: document this:
#[derive(Debug, Default, Clone)]
pub struct GenmcConfig {
    pub(super) params: GenmcParams,
    print_exec_graphs: bool,
    do_estimation: bool,
}

impl GenmcConfig {
    fn set_log_level_trace(&mut self) {
        self.params.quiet = false;
        self.params.log_level_trace = true;
    }

    pub fn print_exec_graphs(&self) -> bool {
        self.print_exec_graphs
    }

    pub fn do_estimation(&self) -> bool {
        self.do_estimation
    }

    /// Function for parsing command line options for GenMC mode.
    /// All GenMC arguments start with the string "-Zmiri-genmc".
    ///
    /// `trimmed_arg` should be the argument to be parsed, with the suffix "-Zmiri-genmc" removed
    pub fn parse_arg(genmc_config: &mut Option<GenmcConfig>, trimmed_arg: &str) {
        if genmc_config.is_none() {
            *genmc_config = Some(Default::default());
        }
        if trimmed_arg.is_empty() {
            return; // this corresponds to "-Zmiri-genmc"
        }
        let genmc_config = genmc_config.as_mut().unwrap();
        let trimmed_arg = trimmed_arg
            .strip_prefix("-")
            .unwrap_or_else(|| panic!("Invalid GenMC argument \"-Zmiri-genmc{trimmed_arg}\""));
        if trimmed_arg == "log-trace" {
            // TODO GENMC: maybe expand to allow more control over log level?
            genmc_config.set_log_level_trace();
        } else if trimmed_arg == "print-graphs" {
            // TODO GENMC (DOCUMENTATION)
            genmc_config.print_exec_graphs = true;
        } else if trimmed_arg == "estimate" {
            // TODO GENMC (DOCUMENTATION): naming, off/on by default?
            genmc_config.do_estimation = true;
        } else if let Some(estimation_max_str) = trimmed_arg.strip_prefix("estimation-max=") {
            // TODO GENMC (DOCUMENTATION)
            let estimation_max = estimation_max_str
                .parse()
                .expect("Zmiri-genmc-estimation-max expects a positive integer argument");
            assert!(estimation_max > 0);
            genmc_config.params.estimation_max = estimation_max;
        } else if trimmed_arg == "symmetry-reduction" {
            // TODO GENMC (PERFORMANCE): maybe make this the default, have an option to turn it off instead
            genmc_config.params.do_symmetry_reduction = true;
        } else {
            // TODO GENMC: how to properly handle this?
            panic!("Invalid GenMC argument: \"-Zmiri-genmc-{trimmed_arg}\"");
        }
    }
}
