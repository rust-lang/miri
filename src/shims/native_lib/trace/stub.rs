use rustc_const_eval::interpret::InterpResult;

pub struct Supervisor;

static SUPERVISOR: std::sync::Mutex<Option<Supervisor>> = std::sync::Mutex::new(None);

impl Supervisor {
    #[inline(always)]
    pub fn is_enabled() -> bool {
        false
    }

    pub fn do_ffi<'tcx, T>(
        _: T,
        f: impl FnOnce() -> InterpResult<'tcx, crate::ImmTy<'tcx>>,
    ) -> InterpResult<'tcx, (crate::ImmTy<'tcx>, Option<super::MemEvents>)> {
        let _g = SUPERVISOR.lock().unwrap();
        f().map(|v| (v, None))
    }
}

#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn init_sv() -> Result<(), !> {
    let mut sv_guard = SUPERVISOR.lock().unwrap();
    if sv_guard.is_none() {
        *sv_guard = Some(Supervisor);
    }
    Ok(())
}

#[inline(always)]
#[allow(dead_code)]
pub fn register_retcode_sv<T>(_: T) {}
