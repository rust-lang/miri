use std::env::{self, VarError};
use std::str::FromStr;
use std::sync::Once;

use rustc_middle::ty::TyCtxt;
use rustc_session::{CtfeBacktrace, EarlyDiagCtxt};
use tracing_subscriber::Registry;

/// The tracing layer from `tracing-chrome` starts a thread in the background that saves data to
/// file and closes the file when stopped. If the thread is not stopped properly, the file will be
/// missing end terminators (`]` for JSON arrays) and other data may also not be flushed. Therefore
/// we need to keep a guard that, when [Drop]ped, will send a signal to stop the thread. Make sure
/// to manually drop this guard, if you are exiting the program with [std::process::exit]!
#[must_use]
pub struct TracingGuard {
    #[cfg(feature = "tracing")]
    _chrome: super::tracing_chrome::FlushGuard,
    _no_construct: (),
}

// This ensures TracingGuard is always a drop-type, even when the `_chrome` field is disabled.
impl Drop for TracingGuard {
    fn drop(&mut self) {}
}

fn rustc_logger_config() -> rustc_log::LoggerConfig {
    // Start with the usual env vars.
    let mut cfg = rustc_log::LoggerConfig::from_env("RUSTC_LOG");

    // Overwrite if MIRI_LOG is set.
    if let Ok(var) = env::var("MIRI_LOG") {
        // MIRI_LOG serves as default for RUSTC_LOG, if that is not set.
        if matches!(cfg.filter, Err(VarError::NotPresent)) {
            // We try to be a bit clever here: if `MIRI_LOG` is just a single level
            // used for everything, we only apply it to the parts of rustc that are
            // CTFE-related. Otherwise, we use it verbatim for `RUSTC_LOG`.
            // This way, if you set `MIRI_LOG=trace`, you get only the right parts of
            // rustc traced, but you can also do `MIRI_LOG=miri=trace,rustc_const_eval::interpret=debug`.
            if tracing::Level::from_str(&var).is_ok() {
                cfg.filter = Ok(format!(
                    "rustc_middle::mir::interpret={var},rustc_const_eval::interpret={var},miri={var}"
                ));
            } else {
                cfg.filter = Ok(var);
            }
        }
    }

    cfg
}

/// The global logger can only be set once per process, so track
/// whether that already happened.
static LOGGER_INITED: Once = Once::new();

#[must_use]
fn init_logger_once(early_dcx: &EarlyDiagCtxt) -> Option<TracingGuard> {
    // If the logger is not yet initialized, initialize it.
    let mut guard = None;
    LOGGER_INITED.call_once(|| {
        #[cfg(feature = "tracing")]
        let (chrome_layer, chrome_guard) =
            super::tracing_chrome::ChromeLayerBuilder::new().include_args(true).build();
        guard = Some(TracingGuard {
            #[cfg(feature = "tracing")]
            _chrome: chrome_guard,
            _no_construct: (),
        });
        rustc_driver::init_logger_with_additional_layer(early_dcx, rustc_logger_config(), || {
            let registry = Registry::default();
            #[cfg(feature = "tracing")]
            let registry = tracing_subscriber::layer::SubscriberExt::with(registry, chrome_layer);
            registry
        });
    });
    guard
}

#[must_use]
pub fn init_early_loggers(early_dcx: &EarlyDiagCtxt) -> Option<TracingGuard> {
    // We only initialize `rustc` if the env var is set (so the user asked for it).
    // If it is not set, we avoid initializing now so that we can initialize later with our custom
    // settings, and *not* log anything for what happens before `miri` starts interpreting.
    if env::var_os("RUSTC_LOG").is_some() { init_logger_once(early_dcx) } else { None }
}

#[must_use]
pub fn init_late_loggers(early_dcx: &EarlyDiagCtxt, tcx: TyCtxt<'_>) -> Option<TracingGuard> {
    // If the logger is not yet initialized, initialize it.
    let guard = init_logger_once(early_dcx);

    // If `MIRI_BACKTRACE` is set and `RUSTC_CTFE_BACKTRACE` is not, set `RUSTC_CTFE_BACKTRACE`.
    // Do this late, so we ideally only apply this to Miri's errors.
    if let Some(val) = env::var_os("MIRI_BACKTRACE") {
        let ctfe_backtrace = match &*val.to_string_lossy() {
            "immediate" => CtfeBacktrace::Immediate,
            "0" => CtfeBacktrace::Disabled,
            _ => CtfeBacktrace::Capture,
        };
        *tcx.sess.ctfe_backtrace.borrow_mut() = ctfe_backtrace;
    }

    guard
}
