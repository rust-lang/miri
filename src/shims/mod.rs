#![warn(clippy::arithmetic_side_effects)]

mod alloc;
mod backtrace;
mod files;
mod math;
#[cfg(all(feature = "native-lib", unix))]
pub mod native_lib;
mod unix;
mod windows;

pub mod cpu_affinity;
pub mod env;
pub mod extern_static;
pub mod foreign_items;
pub mod global_ctor;
pub mod io_error;
pub mod os_str;
pub mod panic;
pub mod readiness;
pub mod sig;
pub mod time;
pub mod tls;
pub mod unwind;

use std::rc::Rc;

use rustc_const_eval::interpret::InterpResult;
use rustc_data_structures::fx::FxHashMap;
use rustc_middle::ty::{Ty, TyCtxt};
use rustc_span::Symbol;
use rustc_target::callconv::FnAbi;

pub use self::cpu_affinity::CpuAffinityMask;
pub use self::files::{FdId, FdTable, FileDescription, FileDescriptionRef, WeakFileDescriptionRef};
#[cfg(all(feature = "native-lib", unix))]
pub use self::native_lib::trace::{init_sv, register_retcode_sv};
pub use self::unix::DirTable;
use crate::shims::foreign_items::EvalContextExt;
use crate::{MPlaceTy, MiriInterpCx, OpTy};

/// What needs to be done after emulating an item (a shim or an intrinsic) is done.
pub enum EmulateItemResult {
    /// The caller is expected to jump to the return block.
    NeedsReturn,
    /// The caller is expected to jump to the unwind block.
    NeedsUnwind,
    /// Jumping to the next block has already been taken care of.
    AlreadyJumped,
    /// The item is not supported.
    NotSupported,
}

impl EmulateItemResult {
    pub fn jump_to_next_block<'tcx, T: Default>(
        self,
        ecx: &mut crate::MiriInterpCx<'tcx>,
        dest: &crate::PlaceTy<'tcx>,
        ret: Option<rustc_middle::mir::BasicBlock>,
        unwind: Option<rustc_middle::mir::UnwindAction>,
        not_supported: impl FnOnce(&mut crate::MiriInterpCx<'tcx>) -> crate::InterpResult<'tcx, T>,
    ) -> crate::InterpResult<'tcx, T> {
        use crate::*;

        match self {
            EmulateItemResult::NeedsReturn => {
                trace!("{:?}", ecx.dump_place(dest));
                ecx.return_to_block(ret)?;
                interp_ok(T::default())
            }
            EmulateItemResult::NeedsUnwind => {
                // Jump to the unwind block to begin unwinding.
                ecx.unwind_to_block(unwind.unwrap())?;
                interp_ok(T::default())
            }
            EmulateItemResult::AlreadyJumped => interp_ok(T::default()),
            EmulateItemResult::NotSupported => not_supported(ecx),
        }
    }
}

/// Describes the signature of a call to a shimmed function.
pub struct ShimCall<'tcx, 'a> {
    pub link_name: Symbol,
    pub abi: &'a FnAbi<'tcx, Ty<'tcx>>,
    pub args: &'a [OpTy<'tcx>],
    pub dest: &'a MPlaceTy<'tcx>,
}

type Shim<'tcx> = Rc<
    dyn Fn(&mut MiriInterpCx<'tcx>, ShimCall<'tcx, '_>) -> InterpResult<'tcx, EmulateItemResult>
        + 'tcx,
>;

pub struct ShimTable<'tcx>(FxHashMap<Symbol, Shim<'tcx>>);

impl<'tcx> ShimTable<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>) -> Self {
        let mut shims = ShimTable(FxHashMap::default());
        MiriInterpCx::register_shims(tcx, &mut shims);
        shims
    }

    pub fn register(&mut self, foreign_name: Symbol, shim: Shim<'tcx>) {
        self.0
            .try_insert(foreign_name, shim)
            .unwrap_or_else(|_| panic!("attempted to register multiple shims for {foreign_name}"));
    }

    pub fn get(&self, foreign_name: &Symbol) -> Option<Shim<'tcx>> {
        self.0.get(foreign_name).cloned()
    }
}
