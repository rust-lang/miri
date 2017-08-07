use rustc::mir;
use rustc::ty::{self, Ty};
use syntax::codemap::Span;

use interpret::{EvalResult, EvalContext, StackPopCleanup, Lvalue, LvalueExtra, PrimVal, Value,
                Machine};

impl<'a, 'tcx, M: Machine<'tcx>> EvalContext<'a, 'tcx, M> {
    pub(crate) fn drop_lvalue(
        &mut self,
        lval: Lvalue<'tcx>,
        instance: ty::Instance<'tcx>,
        ty: Ty<'tcx>,
        span: Span,
    ) -> EvalResult<'tcx> {
        trace!("drop_lvalue: {:#?}", lval);
        // We take the address of the object.  This may well be unaligned, which is fine for us here.
        // However, unaligned accesses will probably make the actual drop implementation fail -- a problem shared
        // by rustc.
        let val = match self.force_allocation(lval)? {
            Lvalue::Ptr {
                ptr,
                extra: LvalueExtra::Vtable(vtable),
                aligned: _,
            } => ptr.to_value_with_vtable(vtable),
            Lvalue::Ptr {
                ptr,
                extra: LvalueExtra::Length(len),
                aligned: _,
            } => ptr.to_value_with_len(len),
            Lvalue::Ptr {
                ptr,
                extra: LvalueExtra::None,
                aligned: _,
            } => ptr.to_value(),
            _ => bug!("force_allocation broken"),
        };
        self.drop(val, instance, ty, span)
    }
    pub(crate) fn drop(
        &mut self,
        arg: Value,
        mut instance: ty::Instance<'tcx>,
        ty: Ty<'tcx>,
        span: Span,
    ) -> EvalResult<'tcx> {
        trace!("drop: {:#?}, {:?}, {:?}", arg, ty.sty, instance.def);

        if let ty::InstanceDef::DropGlue(_, None) = instance.def {
            trace!("nothing to do, aborting");
            // we don't actually need to drop anything
            return Ok(());
        }
        let mir = match ty.sty {
            ty::TyDynamic(..) => {
                let vtable = match arg {
                    Value::ByValPair(_, PrimVal::Ptr(vtable)) => vtable,
                    _ => bug!("expected fat ptr, got {:?}", arg),
                };
                match self.read_drop_type_from_vtable(vtable)? {
                    Some(func) => {
                        instance = func;
                        self.load_mir(func.def)?
                    }
                    // no drop fn -> bail out
                    None => return Ok(()),
                }
            }
            _ => self.load_mir(instance.def)?,
        };

        self.push_stack_frame(
            instance,
            span,
            mir,
            Lvalue::undef(),
            StackPopCleanup::None,
        )?;

        let mut arg_locals = self.frame().mir.args_iter();
        assert_eq!(self.frame().mir.arg_count, 1);
        let arg_local = arg_locals.next().unwrap();
        let dest = self.eval_lvalue(&mir::Lvalue::Local(arg_local))?;
        let arg_ty = self.tcx.mk_mut_ptr(ty);
        self.write_value(arg, dest, arg_ty)
    }
}
