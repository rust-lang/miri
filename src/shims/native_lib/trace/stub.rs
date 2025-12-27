use rustc_const_eval::interpret::InterpResult;

static SUPERVISOR: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct Supervisor;

#[derive(Debug)]
pub struct SvInitError;

impl Supervisor {
    #[inline(always)]
    pub fn is_enabled() -> bool {
        false
    }
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn do_ffi(
        &mut self,
        f: impl FnOnce() -> InterpResult<'tcx, crate::ImmTy<'tcx>>,
    ) -> InterpResult<'tcx, (crate::ImmTy<'tcx>, Option<super::MemEvents>)> {
        // We acquire the lock to ensure that no two FFI calls run concurrently.
        let _g = SUPERVISOR.lock().unwrap();
        f().map(|v| (v, None))
    }
}

#[inline(always)]
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn init_sv() -> Result<!, SvInitError> {
    Err(SvInitError)
}

#[inline(always)]
#[allow(dead_code)]
pub fn register_retcode_sv<T>(_: T) {}
