pub type CallResult<'tcx> =
    rustc_const_eval::interpret::InterpResult<'tcx, (crate::ImmTy<'tcx>, Option<!>)>;

pub struct Supervisor;

impl Supervisor {
    #[inline(always)]
    pub fn is_enabled() -> bool {
        false
    }

    #[inline(always)]
    pub unsafe fn start_ffi<T>(_: T) {}

    #[inline(always)]
    pub unsafe fn end_ffi<T>(_: T) -> Option<!> {
        None
    }
}

#[expect(clippy::missing_safety_doc)]
#[inline(always)]
pub unsafe fn init_sv() -> Result<(), !> {
    Ok(())
}

#[inline(always)]
pub fn register_retcode_sv<T>(_: T) {}
