use rustc_const_eval::interpret::InterpResult;

pub type CallResult<'tcx> = InterpResult<'tcx, (crate::ImmTy<'tcx>, Option<!>)>;

pub struct Supervisor;

impl Supervisor {
    #[inline(always)]
    pub fn is_enabled() -> bool {
        false
    }
}

pub fn do_ffi<'tcx, T>(
    _: T,
    f: impl FnOnce() -> InterpResult<'tcx, crate::ImmTy<'tcx>>,
) -> CallResult<'tcx> {
    f().map(|v| (v, None))
}

#[expect(clippy::missing_safety_doc)]
#[inline(always)]
pub unsafe fn init_sv() -> Result<(), !> {
    Ok(())
}

#[inline(always)]
pub fn register_retcode_sv<T>(_: T) {}
