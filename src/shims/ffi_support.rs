use libffi::{high::call::*, low::CodePtr};
use std::ops::Deref;

use rustc_middle::ty::{IntTy, Ty, TyKind, UintTy};
use rustc_span::Symbol;
use rustc_target::abi::HasDataLayout;

use crate::*;

impl<'mir, 'tcx: 'mir> EvalContextExt<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {}

pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriEvalContextExt<'mir, 'tcx> {
    /// Extract the scalar value from the result of reading a scalar from the machine,
    /// and convert it to a `CArg`.
    fn scalar_to_carg(
        k: ScalarMaybeUninit<Tag>,
        arg_type: &Ty<'tcx>,
        cx: &impl HasDataLayout,
    ) -> InterpResult<'tcx, CArg> {
        match arg_type.kind() {
            // If the primitive provided can be converted to a type matching the hir type pattern
            // then create a `CArg` of this primitive value with the corresponding `CArg` constructor.
            // the ints
            TyKind::Int(IntTy::I8) => {
                return Ok(CArg::Int8(k.to_i8()?));
            }
            TyKind::Int(IntTy::I16) => {
                return Ok(CArg::Int16(k.to_i16()?));
            }
            TyKind::Int(IntTy::I32) => {
                return Ok(CArg::Int32(k.to_i32()?));
            }
            TyKind::Int(IntTy::I64) => {
                return Ok(CArg::Int64(k.to_i64()?));
            }
            TyKind::Int(IntTy::Isize) => {
                return Ok(CArg::ISize(k.to_machine_isize(cx)?.try_into().unwrap()));
            }
            // the uints
            TyKind::Uint(UintTy::U8) => {
                return Ok(CArg::UInt8(k.to_u8()?));
            }
            TyKind::Uint(UintTy::U16) => {
                return Ok(CArg::UInt16(k.to_u16()?));
            }
            TyKind::Uint(UintTy::U32) => {
                return Ok(CArg::UInt32(k.to_u32()?));
            }
            TyKind::Uint(UintTy::U64) => {
                return Ok(CArg::UInt64(k.to_u64()?));
            }
            TyKind::Uint(UintTy::Usize) => {
                return Ok(CArg::USize(k.to_machine_usize(cx)?.try_into().unwrap()));
            }
            _ => {}
        }
        // If no primitives were returned then we have an unsupported type.
        throw_unsup_format!(
            "unsupported scalar argument type to external C function: {:?}",
            arg_type
        );
    }

    /// Call external C function and
    /// store output, depending on return type in the function signature.
    fn call_external_c_and_store_return<'a>(
        &mut self,
        external_fct_defn: ExternalCFuncDeclRep<'tcx>,
        dest: &PlaceTy<'tcx, Tag>,
        ptr: CodePtr,
        libffi_args: Vec<libffi::high::Arg<'a>>,
    ) -> InterpResult<'tcx, ()> {
        let this = self.eval_context_mut();

        // Unsafe because of the call to external C code.
        // Because this is calling a C function it is not necessarily sound,
        // but there is no way around this and we've checked as much as we can.
        unsafe {
            // If the return type of a function is a primitive integer type,
            // then call the function (`ptr`) with arguments `libffi_args`, store the return value as the specified
            // primitive integer type, and then write this value out to the miri memory as an integer.
            match external_fct_defn.output_type.kind() {
                // ints
                TyKind::Int(IntTy::I8) => {
                    let x = call::<i8>(ptr, libffi_args.as_slice());
                    this.write_int(x, dest)?;
                    return Ok(());
                }
                TyKind::Int(IntTy::I16) => {
                    let x = call::<i16>(ptr, libffi_args.as_slice());
                    this.write_int(x, dest)?;
                    return Ok(());
                }
                TyKind::Int(IntTy::I32) => {
                    let x = call::<i32>(ptr, libffi_args.as_slice());
                    this.write_int(x, dest)?;
                    return Ok(());
                }
                TyKind::Int(IntTy::I64) => {
                    let x = call::<i64>(ptr, libffi_args.as_slice());
                    this.write_int(x, dest)?;
                    return Ok(());
                }
                TyKind::Int(IntTy::Isize) => {
                    let x = call::<isize>(ptr, libffi_args.as_slice());
                    // `isize` doesn't `impl Into<i128>`, so convert manually.
                    // Convert to `i64` since this covers both 32- and 64-bit machines.
                    this.write_int(i64::try_from(x).unwrap(), dest)?;
                    return Ok(());
                }
                // uints
                TyKind::Uint(UintTy::U8) => {
                    let x = call::<u8>(ptr, libffi_args.as_slice());
                    this.write_int(x, dest)?;
                    return Ok(());
                }
                TyKind::Uint(UintTy::U16) => {
                    let x = call::<u16>(ptr, libffi_args.as_slice());
                    this.write_int(x, dest)?;
                    return Ok(());
                }
                TyKind::Uint(UintTy::U32) => {
                    let x = call::<u32>(ptr, libffi_args.as_slice());
                    this.write_int(x, dest)?;
                    return Ok(());
                }
                TyKind::Uint(UintTy::U64) => {
                    let x = call::<u64>(ptr, libffi_args.as_slice());
                    this.write_int(x, dest)?;
                    return Ok(());
                }
                TyKind::Uint(UintTy::Usize) => {
                    let x = call::<usize>(ptr, libffi_args.as_slice());
                    // `usize` doesn't `impl Into<i128>`, so convert manually.
                    // Convert to `u64` since this covers both 32- and 64-bit machines.
                    this.write_int(u64::try_from(x).unwrap(), dest)?;
                    return Ok(());
                }
                _ => {}
            }
            // Functions with no declared return type (i.e., the default return)
            // have the output_type `Tuple([])`.
            if let TyKind::Tuple(t_list) = external_fct_defn.output_type.kind() && t_list.len() == 0{
                call::<()>(ptr, libffi_args.as_slice());
                return Ok(());
            }
            // TODO ellen! deal with all the other return types
            throw_unsup_format!(
                "unsupported return type to external C function: {:?}",
                external_fct_defn.link_name
            );
        }
    }

    /// Call specified external C function, with supplied arguments.
    /// Need to convert all the arguments from their hir representations to
    /// a form compatible with C (through `libffi` call).
    /// Then, convert return from the C call into a corresponding form that
    /// can be stored in Miri internal memory.
    fn call_and_add_external_c_fct_to_context(
        &mut self,
        external_fct_defn: ExternalCFuncDeclRep<'tcx>,
        dest: &PlaceTy<'tcx, Tag>,
        args: &[OpTy<'tcx, Tag>],
    ) -> InterpResult<'tcx, bool> {
        let this = self.eval_context_mut();
        let link_name = external_fct_defn.link_name;
        let (lib, lib_path) = this.machine.external_so_lib.as_ref().unwrap();

        // Load the C function from the library.
        // Because this is getting the C function from the shared object file
        // it is not necessarily a sound operation, but there is no way around
        // this and we've checked as much as we can.
        let func: libloading::Symbol<'_, unsafe extern "C" fn()> = unsafe {
            match lib.get(link_name.as_str().as_bytes()) {
                Ok(x) => x,
                Err(_) => {
                    // Shared object file does not export this function -- try the shims next.
                    return Ok(false);
                }
            }
        };

        // FIXME: this is a hack!
        // The `libloading` crate will automatically load system libraries like `libc`.
        // So, in order to check if the function was actually found in the specified
        // `machine.external_so_lib` we need to check its `dli_fname` and compare it to
        // the specified SO file path.
        // This code is a reimplementation of the mechanism for getting `dli_fname` in `libloading`,
        // from: https://docs.rs/libloading/0.7.3/src/libloading/os/unix/mod.rs.html#411
        // using the `libc` crate where this interface is public.
        let mut info = std::mem::MaybeUninit::<libc::Dl_info>::uninit();
        unsafe {
            if libc::dladdr(*func.deref() as *const _, info.as_mut_ptr()) != 0 {
                if std::ffi::CStr::from_ptr(info.assume_init().dli_fname).to_str().unwrap()
                    != lib_path.to_str().unwrap()
                {
                    return Ok(false);
                } 
            }
        }

        // Get the function arguments, and convert them to `libffi`-compatible form.
        if args.len() != external_fct_defn.inputs_types.len() {
            throw_ub_format!(
                "calling function {:?} with {} arguments; expected {}",
                link_name,
                args.len(),
                external_fct_defn.inputs_types.len()
            );
        }
        let mut libffi_args = Vec::<CArg>::with_capacity(args.len());
        for (cur_arg, arg_type) in args.iter().zip(external_fct_defn.inputs_types.iter()) {
            libffi_args.push(Self::scalar_to_carg(this.read_scalar(cur_arg)?, arg_type, this)?);
        }

        // Convert them to `libffi::high::Arg` type.
        let libffi_args = libffi_args
            .iter()
            .map(|cur_arg| cur_arg.arg_downcast())
            .collect::<Vec<libffi::high::Arg<'_>>>();

        // Code pointer to C function.
        let ptr = CodePtr(*func.deref() as *mut _);
        // Call the functio and store output, depending on return type in the function signature.
        self.call_external_c_and_store_return(external_fct_defn, dest, ptr, libffi_args)?;
        Ok(true)
    }
}

#[derive(Debug)]
/// Signature of an external C function.
pub struct ExternalCFuncDeclRep<'tcx> {
    /// Function name.
    pub link_name: Symbol,
    /// Argument types.
    pub inputs_types: &'tcx [Ty<'tcx>],
    /// Return type.
    pub output_type: Ty<'tcx>,
}

#[derive(Debug, Clone)]
/// Enum of supported arguments to external C functions.
pub enum CArg {
    /// 8-bit signed integer.
    Int8(i8),
    /// 16-bit signed integer.
    Int16(i16),
    /// 32-bit signed integer.
    Int32(i32),
    /// 64-bit signed integer.
    Int64(i64),
    /// isize.
    ISize(isize),
    /// 8-bit unsigned integer.
    UInt8(u8),
    /// 16-bit unsigned integer.
    UInt16(u16),
    /// 32-bit unsigned integer.
    UInt32(u32),
    /// 64-bit unsigned integer.
    UInt64(u64),
    /// usize.
    USize(usize),
}

impl<'a> CArg {
    /// Convert a `CArg` to a `libffi` argument type.
    pub fn arg_downcast(&'a self) -> libffi::high::Arg<'a> {
        match self {
            CArg::Int8(i) => arg(i),
            CArg::Int16(i) => arg(i),
            CArg::Int32(i) => arg(i),
            CArg::Int64(i) => arg(i),
            CArg::ISize(i) => arg(i),
            CArg::UInt8(i) => arg(i),
            CArg::UInt16(i) => arg(i),
            CArg::UInt32(i) => arg(i),
            CArg::UInt64(i) => arg(i),
            CArg::USize(i) => arg(i),
        }
    }
}
