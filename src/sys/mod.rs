use cfg_if::cfg_if;

use rustc::ty;
use rustc::ty::layout::{Align, LayoutOf, Size};
use rustc::hir::def_id::DefId;
use rustc::mir;
use syntax::attr;
use syntax::symbol::sym;
use crate::*;

pub trait PlatformExt<'mir, 'tcx>: crate::MiriEvalContextExt<'mir, 'tcx> {
    fn eval_ffi(
        &mut self,
        def_id: DefId,
        args: &[OpTy<'tcx, Tag>],
        dest: PlaceTy<'tcx, Tag>,
        ret: mir::BasicBlock,
        link_name: &str
    ) -> InterpResult<'tcx, Option<&'mir mir::Body<'tcx>>>;
}

cfg_if! {
    if #[cfg(target_os = "linux")] {
        mod linux;
        pub use linux::*;
    }
}
