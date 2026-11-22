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

/// Create the registration function for a shim.
///
/// ```rust
/// register! {
///     name: poll,
///     sig: extern "C" fn(*libc::pollfd, libc::nfds_t, i32) -> i32
/// }
/// ```
///
/// This will create a function `register_poll` which will invoke `ecx.poll(args, dest)` when
/// a foreign function invocation on `poll` is encountered. `args` is the array of argument
/// operands and `dest` is the destination place of the foreign function invocation.
///
/// To indicate that the shimmed function is variadic, the signature can be prefixed with `variadic`:
/// ```rust
///     register! {
///     name: fcntl,
///     sig: variadic extern "C" fn(i32, i32, ...) -> i32,
/// }
/// ```
/// This will then create a function `register_fcntl` which will invoke `ecx.fcntl(args, varargs, dest)`
/// (notice the additional `varargs` argument compared to a non-variadic shim) when a foreign function
/// invocation on `fcntl` is encountered. The varargs are of type `Varargs`. The remaining arguments
/// stay the same as in the non-variadic case.
///
/// Besides the required `name` and `sig` "fields", the macro also supports a few optional
/// "fields":
///
/// - `call`: A closure-like "field" which allows customizing the handler for a shim. This is
///   useful when multiple shims share the same handler (e.g., in the case of `accept` and `accept4`):
///   ```rust
///   // `accept` doesn't have a `flags` argument, thus we add a `None` argument
///   // to the handler.
///   register! {
///       name: accept,
///       sig: extern "C" fn(i32, *libc::sockaddr, *libc::socklen_t) -> i32,
///       call: |ecx, args, dest| ecx.accept4(args, /* flags */ None, dest)
///   }
///   // `accept4` has a `flags` argument, thus we insert `Some(flags)` argument
///   // to the handler.
///   register! {
///       name: accept4,
///       sig: extern "C" fn(i32, *libc::sockaddr, *libc::socklen_t, i32) -> i32,
///       call: |ecx, [args @ .., flags], dest| ecx.accept4(args, Some(flags), dest)
///   }
///   ```
///   **Note**: If the shimmed function is variadic, the "closure" has an additional `varargs` "argument"
///   and thus it has the form `|ecx, args, varargs, dest|`.
///
/// - `result`: The result to return after a call to the shim succeeded. If this "field"
///   is omitted, [`EmulateItemResult::NeedsReturn`] is returned. Some shims need to return
///   a different result, for example the `miri_start_unwind` shim:
///   ```rust
///   register! {
///       name: miri_start_unwind,
///       sig: extern "Rust" fn(*()) -> !,
///       result: EmulateItemResult::NeedsUnwind
///   }
///   ```
#[macro_export]
macro_rules! register {
    // Initially we only match the name of the shim and whether the signature is variadic.
    // We also initialize the default "values" for all optional "fields".

    // The variadic case.
    (name: $name:ident, sig: variadic $($rest:tt)*) => {
        register! {
            @sig $name [variadic] [] (
            /* By default we just invoke `ecx.<name>(args, varargs, dest)` */ [ecx args varargs dest ecx.$name(args, varargs, dest)]
            /* The default result returned after a call */ [$crate::EmulateItemResult::NeedsReturn]
            ) ; $($rest)*
        }
    };
    // The non-variadic case.
    (name: $name:ident, sig: $($rest:tt)*) => {
        register! {
            @sig $name [] [] (
            /* By default we just invoke `ecx.<name>(args, dest)` */ [ecx args dest ecx.$name(args, dest)]
            /* The default result returned after a call */ [$crate::EmulateItemResult::NeedsReturn]
            ) ; $($rest)*
        }
    };

    // Since signatures can contain arbitrary token trees, we need to have a TT-mangler for them.
    // We consume everything into `sig` until we either encounter the start of a new "field"
    // (i.e., `, <ident>: <rest>`) or when only an optional trailing comma is left in the input.

    // Stop `sig` when the next token trees are the start of a new "field".
    (@sig $name:ident $variadic:tt [$($sig:tt)*] $defaults:tt ; , $field:ident : $($rest:tt)*) => {
        register!{ @emit $name $variadic [$($sig)*] $defaults ; $field: $($rest)* }
    };
    // Stop `sig` when there is only an optional trailing comma left.
    (@sig $name:ident $variadic:tt [$($sig:tt)*] $defaults:tt ; $(,)?) => {
        register!{ @emit $name $variadic [$($sig)*] $defaults ; }
    };
    // Consume a single token tree into `sig` and continue matching on the rest.
    (@sig $name:ident $variadic:tt [$($sig:tt)*] $defaults:tt ; $tok:tt $($rest:tt)*) => {
        register! { @sig $name $variadic [$($sig)* $tok] $defaults ; $($rest)* }
    };

    // For every optional "field" that is provided, overwrite the default "value" for that field
    // using the provided "value".

    // In the non-variadic case we don't have the `varargs` "closure" argument.
    (@emit $name:ident [] $sig:tt ($call:tt $result:tt) ; call: |$ecx:ident, $args:tt, $dest:ident| $handler:expr $(, $($rest:tt)*)?) => {
        register! { @emit $name [] $sig ([$ecx $args $dest $handler] $result) ; $($($rest)*)? }
    };
    // In the variadic case we also have the `varargs` "closure" argument.
    (@emit $name:ident [variadic] $sig:tt ($call:tt $result:tt) ; call: |$ecx:ident, $args:tt, $varargs:ident, $dest:ident| $handler:expr $(, $($rest:tt)*)?) => {
        register! { @emit $name [variadic] $sig ([$ecx $args $varargs $dest $handler] $result) ; $($($rest)*)? }
    };
    // We emit a compile error should one try to provide a "closure" without `varargs` for a variadic shim.
    (@emit $name:ident [variadic] $sig:tt ($call:tt $result:tt) ; call: |$ecx:ident, $args:tt, $dest:ident| $handler:expr $(, $($rest:tt)*)?) => {
        compile_error!("expected signature `|ecx, args, varargs, dest|` for variadic shim, but got `|ecx, args, dest|`");
    };
    (@emit $name:ident $variadic:tt $sig:tt ($call:tt $_:tt) ; result: $result:path $(, $($rest:tt)*)?) => {
        register! { @emit $name $variadic $sig ($call [$result]) ; $($($rest)*)? }
    };

    // When all "fields" are populated, we can emit the actual registration function.
    // The `sig` token trees are passed to the `shim_sig!` macro.

    (@emit $name:ident $variadic:tt [$($sig:tt)*] ([$ecx:ident $($args:tt)*] $result:tt) ; ) => {
        fn ${concat(register_, $name)}<'a>(
            #[allow(unused)] tcx: ::rustc_middle::ty::TyCtxt<'a>,
            shims: &mut $crate::shims::ShimTable<'a>
        ) {
            let cell = ::std::cell::OnceCell::new();
            let eval_shim_sig: fn(&mut $crate::MiriInterpCx<'a>) -> _ =
                shim_sig!($($sig)*);

            shims.register(
                ::rustc_span::Symbol::intern(stringify!($name)),
                ::std::rc::Rc::new(move |$ecx, call| {
                    let shim_sig = cell.get_or_init(|| eval_shim_sig($ecx));

                    register! { @emit_inner $variadic call shim_sig [$ecx $($args)*] $result }
                }),
            );
        }
    };

    // Depending on whether the signature is variadic or not, we need to emit different bodies for
    // the shim callback function.

    // In the non-variadic case, we internally use `call.check_sig`.
    (@emit_inner [] $call:ident $shim_sig:ident [$ecx:ident $args:tt $dest:ident $handler:expr] [$result:path]) => {
        let (args, ref $dest) = $call.check_sig($ecx, $shim_sig)?;
        let $args = args.each_ref();

        $handler.map(|()| $result)
    };

    // In the variadic case, we internally use `call.check_sig_variadic`.
    (@emit_inner [variadic] $call:ident $shim_sig:ident [$ecx:ident $args:tt $varargs:tt $dest:ident $handler:expr] [$result:path])  => {
        let (args, $varargs, ref $dest) = $call.check_sig_variadic($ecx, $shim_sig)?;
        let $args = args.each_ref();

        $handler.map(|()| $result)
    };
}
