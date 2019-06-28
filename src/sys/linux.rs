use rustc::ty::{self, TyKind, Ty};
use rustc::ty::layout::{Align, LayoutOf, Size};
use rustc::hir::def_id::DefId;
use rustc::hir::Mutability;
use rustc::mir;
use syntax::attr;
use syntax::symbol::sym;
use syntax::ast::{IntTy, UintTy};

use std::ffi::CString;

use crate::*;
use crate::sys::PlatformExt;

use libc::{dlsym, RTLD_DEFAULT};
use libffi::middle::*;
use libffi::low;
use libffi::high::CType;


pub struct BoxWrapper {
    pub ptr: *mut libc::c_void,
    dtor: Option<Box<FnOnce(*mut libc::c_void)>>
}

impl BoxWrapper {
    fn new<T>(data: T) -> BoxWrapper {
        let dtor = Box::new(|raw_ptr| {
            unsafe { Box::from_raw(raw_ptr as *mut T) };
        });
        BoxWrapper {
            ptr: Box::into_raw(Box::new(data)) as *mut libc::c_void,
            dtor: Some(dtor)
        }
    }
}

impl Drop for BoxWrapper {
    fn drop(&mut self) {
        (self.dtor.take().unwrap())(self.ptr)
    }
}

fn ty_to_type<'tcx>(ty: Ty<'tcx>) -> InterpResult<'tcx, Type> {
    Ok(match ty.sty {
        TyKind::Bool => Type::u8(),
        TyKind::Int(IntTy::I8) => Type::i8(),
        TyKind::Int(IntTy::Isize) => Type::isize(),
        TyKind::Uint(UintTy::Usize) => Type::usize(),
        TyKind::RawPtr(_) => Type::pointer(),
        _ => return err!(Unimplemented(format!("Don't know how represent type {:?} in FFI", ty)))
    })
}

enum WrappedArg {
    Plain(BoxWrapper),
    Pointer(PointerData)
}

struct PointerData {
    pointer: Pointer<Tag>,
    data: Vec<u8>
}

fn convert_ty<'mir, 'tcx: 'mir>(this: &mut MiriEvalContext<'mir, 'tcx>,
                                arg: OpTy<'tcx, Tag>,
                                builder: &mut Builder,
                                args: &mut Vec<WrappedArg>) -> InterpResult<'tcx> {

    Ok(match arg.layout.ty.sty {
        TyKind::Bool => {
            args.push(WrappedArg::Plain(BoxWrapper::new(this.read_scalar(arg)?.to_bool()?)));
        },
        TyKind::Int(IntTy::I8) => {
            args.push(WrappedArg::Plain(BoxWrapper::new(this.read_scalar(arg)?.to_i8()?)));
        },
        TyKind::Int(IntTy::Isize) => {
            args.push(WrappedArg::Plain(BoxWrapper::new(this.read_scalar(arg)?.to_isize(this)?)));
        }
        TyKind::Uint(UintTy::Usize) => {
            args.push(WrappedArg::Plain(BoxWrapper::new(this.read_scalar(arg)?.to_usize(this)?)));
        },
        TyKind::RawPtr(_) => {
            let ptr = this.read_scalar(arg)?.to_ptr()?;
            let mut bytes = this.memory_mut().get_mut(ptr.alloc_id)?.bytes.clone();
            args.push(WrappedArg::Pointer(PointerData {
                pointer: ptr,
                data: bytes
            }));
        }
        _ => return err!(Unimplemented(format!("Don't know how represent type {:?} in FFI", arg.layout.ty)))
    })
}

#[derive(Debug)]
enum RetData {
    Small(u128),
    Large(Vec<u8>)
}

fn call_fn<'mir, 'tcx: 'mir>(this: &mut MiriEvalContext<'mir, 'tcx>, ptr: *const libc::c_void, args: &[OpTy<'tcx, Tag>], ret: PlaceTy<'tcx, Tag>) -> InterpResult<'tcx> {
    let mut builder = Builder::new(); 
    let mut actual_args = vec![];
    for arg in args {
        error!("miri: adding arg {:?}", arg);
        builder = builder.arg(ty_to_type(arg.layout.ty)?);
        convert_ty(this, *arg, &mut builder, &mut actual_args)?;
    }
    builder = builder.res(ty_to_type(ret.layout.ty)?);;
    let mut cif = builder.into_cif();

    let mut cif_args: Vec<*mut libc::c_void> = vec![];

    for arg in &mut actual_args {
        match arg {
            WrappedArg::Plain(box_wrapper) => {
                cif_args.push(box_wrapper.ptr)
            },
            WrappedArg::Pointer(pointer_data) => {
                cif_args.push(pointer_data.data.as_mut_ptr() as *mut libc::c_void)
            }
        }
    }

    let fn_ptr = unsafe { std::mem::transmute::<*const libc::c_void, Option<unsafe extern "C" fn()>>(ptr) };

    let mut ret_data: RetData;
    let mut ret_ptr: *mut libc::c_void;
    // It fits in a u128 - we can use an Immediate
    if ret.layout.size.bytes() <= 16 {
        ret_data = RetData::Small(0);
        match &mut ret_data {
            RetData::Small(ref mut s) => ret_ptr = s as *mut u128 as *mut libc::c_void,
            _ => unreachable!()
        }
    } else {
        ret_data = RetData::Large(vec![0; ret.layout.size.bytes() as usize]);
        match &mut ret_data {
            RetData::Large(ref mut l) => ret_ptr = l.as_mut_ptr() as *mut libc::c_void,
            _ => unreachable!()
        }
    }

    error!("miri: calling cif {:?}", cif);

    let res = unsafe {
        libffi::raw::ffi_call(cif.as_raw_ptr(),
                              fn_ptr,
                              ret_ptr,
                              cif_args.as_mut_ptr())
                              
    };

    error!("miri: Result: {:?}", ret_data);

    let tcx = &{this.tcx.tcx};

    for arg in &actual_args {
        match arg {
            WrappedArg::Pointer(pointer_data) => {
                error!("miri: Writing back data to pointer {:?}: {:?}", pointer_data.pointer,
                       pointer_data.data);
                this.memory_mut().get_mut(pointer_data.pointer.alloc_id)?.write_bytes(
                    tcx,
                    pointer_data.pointer,
                    &pointer_data.data
                )?;
            },
            _ => {}
        }
    }

    match ret_data {
        RetData::Small(s) => {
            this.write_scalar(Scalar::from_uint(s, ret.layout.size), ret)?;
        },
        RetData::Large(data) => {
            let ptr = ret.to_ptr()?;
            this.memory_mut().get_mut(ptr.alloc_id)?.write_bytes(tcx, ptr, &data)?;
        }
    }

    Ok(())
}

impl<'mir, 'tcx: 'mir> PlatformExt<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {
    fn eval_ffi(
        &mut self,
        def_id: DefId,
        args: &[OpTy<'tcx, Tag>],
        dest: PlaceTy<'tcx, Tag>,
        ret: mir::BasicBlock,
        link_name: &str
    ) -> InterpResult<'tcx, Option<&'mir mir::Body<'tcx>>> {
        error!("calling dlsym({})", link_name);

        let c_link_name = CString::new(link_name).unwrap();
        let ret = unsafe { dlsym(RTLD_DEFAULT, c_link_name.as_ptr()) };
        error!("dlsym({}): got symbol {:?}", link_name, ret);

        if ret.is_null() {
            return err!(Unimplemented(format!("Failed to find dynamic symbol {}", link_name)));
        }

        call_fn(self, ret, args, dest)?;

        Ok(None)
    }
}
