use rustc_middle::ty::Ty;
use rustc_span::Symbol;
use rustc_target::callconv::FnAbi;

use crate::shims::unix::env::EvalContextExt;
use crate::shims::unix::linux_like::eventfd::EvalContextExt as _;
use crate::shims::unix::linux_like::sync::futex;
use crate::shims::unix::socket::EvalContextExt as _;
use crate::*;

/// Checks `SYS_foo(a: T, ...)` varargs and runs the handler with `&OpTy` bindings.
macro_rules! dispatch_syscall_check_impl {
    (
        $ecx:expr, $varargs:expr, $SYS:ident, $handler:block,
        [$($types:tt)*] [$($names:ident)*]
    ) => {{
        let msg = concat!("syscall(", stringify!($SYS), ", ...)");
        let ([$($names),*], _rest) = $ecx.check_varargs(
            shim_varargs![$($types)*],
            $varargs,
            msg,
        )?;
        $handler
    }};
    // `*_` needs its own arm since it is not a valid `:ty`.
    (
        $ecx:expr, $varargs:expr, $SYS:ident, $handler:block,
        [$($types:tt)*] [$($names:ident)*]
        $name:ident : *_ $(, $($tail:tt)*)?
    ) => {
        dispatch_syscall_check_impl!(
            $ecx, $varargs, $SYS, $handler,
            [$($types)* *_,] [$($names)* $name]
            $($($tail)*)?
        )
    };
    (
        $ecx:expr, $varargs:expr, $SYS:ident, $handler:block,
        [$($types:tt)*] [$($names:ident)*]
        $name:ident : $ty:tt $(, $($tail:tt)*)?
    ) => {
        dispatch_syscall_check_impl!(
            $ecx, $varargs, $SYS, $handler,
            [$($types)* $ty,] [$($names)* $name]
            $($($tail)*)?
        )
    };
}

macro_rules! dispatch_syscall_check {
    ($ecx:expr, $varargs:expr, $SYS:ident, $handler:block,) => {
        $handler
    };
    ($ecx:expr, $varargs:expr, $SYS:ident, $handler:block, $($pairs:tt)*) => {
        dispatch_syscall_check_impl!(
            $ecx, $varargs, $SYS, $handler,
            [] []
            $($pairs)*
        )
    };
}

/// Dispatches `SYS_*` numbers to shims; `SYS_foo(a: T, ...)` checks varargs.
/// Bare `SYS_foo` passes `varargs` through untouched.
macro_rules! dispatch_syscalls {
    ($ecx:expr, $op:expr, $varargs:expr, $($SYS:ident $(($($pairs:tt)*))? => $handler:block),* $(,)?) => {{
        $(#[allow(non_snake_case)] let $SYS = $ecx.eval_libc(stringify!($SYS)).to_target_usize($ecx)?;)*
        match $ecx.read_target_usize($op)? {
            $(
                id if id == $SYS => {
                    dispatch_syscall_check!($ecx, $varargs, $SYS, $handler, $($($pairs)*)?);
                }
            )*
            num => throw_unsup_format!("syscall: unsupported syscall number {num}"),
        }
    }};
}

pub fn syscall<'tcx>(
    ecx: &mut MiriInterpCx<'tcx>,
    link_name: Symbol,
    abi: &FnAbi<'tcx, Ty<'tcx>>,
    args: &[OpTy<'tcx>],
    dest: &MPlaceTy<'tcx>,
) -> InterpResult<'tcx> {
    let ([op], varargs) = ecx.check_shim_sig_variadic(
        shim_sig!(extern "C" fn(isize, ...) -> isize),
        (link_name, abi, args),
    )?;
    // The syscall variadic function is legal to call with more arguments than needed,
    // extra arguments are simply ignored. The important check is that when we use an
    // argument, we have to also check all arguments *before* it to ensure that they
    // have the right type.

    dispatch_syscalls!(ecx, op, varargs,
        SYS_getrandom(ptr: *_, len: usize, flags: i32) => {
            // `libc::syscall(NR_GETRANDOM, buf.as_mut_ptr(), buf.len(), GRND_NONBLOCK)`
            // is called if a `HashMap` is created the regular way (e.g. HashMap<K, V>).
            // Used by getrandom 0.1
            let ptr = ecx.read_pointer(ptr)?;
            let len = ecx.read_target_usize(len)?;
            // The only supported flags are GRND_RANDOM and GRND_NONBLOCK,
            // neither of which have any effect on our current PRNG.
            // See <https://github.com/rust-lang/rust/pull/79196> for a discussion of argument sizes.
            let _flags = ecx.read_scalar(flags)?.to_i32()?;

            ecx.gen_random(ptr, len)?;
            ecx.write_scalar(Scalar::from_target_usize(len, ecx), dest)?;
        },
        SYS_futex => {
            // `futex` is used by some synchronization primitives.
            futex(ecx, varargs, dest)?;
        },
        SYS_eventfd2(initval: u32, flags: i32) => {
            let result = ecx.eventfd(initval, flags)?;
            ecx.write_int(result.to_i32()?, dest)?;
        },
        SYS_gettid => {
            let result = ecx.unix_gettid("SYS_gettid")?;
            ecx.write_int(result.to_u32()?, dest)?;
        },
        SYS_accept4(socket: i32, address: *_, address_len: *_, flags: i32) => {
            // Used on Android.
            ecx.accept4(socket, address, address_len, Some(flags), dest)?;
        },
    );

    interp_ok(())
}
