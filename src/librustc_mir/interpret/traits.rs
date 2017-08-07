use rustc::traits::{self, Reveal};
use rustc::hir::def_id::DefId;
use rustc::ty::subst::Substs;
use rustc::ty::{self, Ty};
use syntax::codemap::DUMMY_SP;
use syntax::ast::{self, Mutability};

use super::{EvalResult, EvalContext, eval_context, MemoryPointer, Kind, Value, PrimVal, Machine};

impl<'a, 'tcx, M: Machine<'tcx>> EvalContext<'a, 'tcx, M> {
    pub(crate) fn fulfill_obligation(
        &self,
        trait_ref: ty::PolyTraitRef<'tcx>,
    ) -> traits::Vtable<'tcx, ()> {
        // Do the initial selection for the obligation. This yields the shallow result we are
        // looking for -- that is, what specific impl.
        self.tcx.infer_ctxt().enter(|infcx| {
            let mut selcx = traits::SelectionContext::new(&infcx);

            let obligation = traits::Obligation::new(
                traits::ObligationCause::misc(DUMMY_SP, ast::DUMMY_NODE_ID),
                ty::ParamEnv::empty(Reveal::All),
                trait_ref.to_poly_trait_predicate(),
            );
            let selection = selcx.select(&obligation).unwrap().unwrap();

            // Currently, we use a fulfillment context to completely resolve all nested obligations.
            // This is because they can inform the inference of the impl's type parameters.
            let mut fulfill_cx = traits::FulfillmentContext::new();
            let vtable = selection.map(|predicate| {
                fulfill_cx.register_predicate_obligation(&infcx, predicate);
            });
            infcx.drain_fulfillment_cx_or_panic(DUMMY_SP, &mut fulfill_cx, &vtable)
        })
    }

    /// Creates a dynamic vtable for the given type and vtable origin. This is used only for
    /// objects.
    ///
    /// The `trait_ref` encodes the erased self type. Hence if we are
    /// making an object `Foo<Trait>` from a value of type `Foo<T>`, then
    /// `trait_ref` would map `T:Trait`.
    pub fn get_vtable(
        &mut self,
        ty: Ty<'tcx>,
        trait_ref: ty::PolyTraitRef<'tcx>,
    ) -> EvalResult<'tcx, MemoryPointer> {
        debug!("get_vtable(trait_ref={:?})", trait_ref);

        let size = self.type_size(trait_ref.self_ty())?.expect(
            "can't create a vtable for an unsized type",
        );
        let align = self.type_align(trait_ref.self_ty())?;

        let ptr_size = self.memory.pointer_size();
        let methods = ::rustc::traits::get_vtable_methods(self.tcx, trait_ref);
        let vtable = self.memory.allocate(
            ptr_size * (3 + methods.count() as u64),
            ptr_size,
            Kind::UninitializedStatic,
        )?;

        let drop = eval_context::resolve_drop_in_place(self.tcx, ty);
        let drop = self.memory.create_fn_alloc(drop);
        self.memory.write_ptr(vtable, drop)?;

        let size_ptr = vtable.offset(ptr_size, &self)?;
        self.memory.write_usize(size_ptr, size)?;
        let align_ptr = vtable.offset(ptr_size * 2, &self)?;
        self.memory.write_usize(align_ptr, align)?;

        for (i, method) in ::rustc::traits::get_vtable_methods(self.tcx, trait_ref).enumerate() {
            if let Some((def_id, substs)) = method {
                let instance = eval_context::resolve(self.tcx, def_id, substs);
                let fn_ptr = self.memory.create_fn_alloc(instance);
                let method_ptr = vtable.offset(ptr_size * (3 + i as u64), &self)?;
                self.memory.write_ptr(method_ptr, fn_ptr)?;
            }
        }

        self.memory.mark_static_initalized(
            vtable.alloc_id,
            Mutability::Mutable,
        )?;

        Ok(vtable)
    }

    pub fn read_drop_type_from_vtable(
        &self,
        vtable: MemoryPointer,
    ) -> EvalResult<'tcx, Option<ty::Instance<'tcx>>> {
        // we don't care about the pointee type, we just want a pointer
        match self.read_ptr(vtable, self.tcx.mk_nil_ptr())? {
            // some values don't need to call a drop impl, so the value is null
            Value::ByVal(PrimVal::Bytes(0)) => Ok(None),
            Value::ByVal(PrimVal::Ptr(drop_fn)) => self.memory.get_fn(drop_fn).map(Some),
            _ => err!(ReadBytesAsPointer),
        }
    }

    pub fn read_size_and_align_from_vtable(
        &self,
        vtable: MemoryPointer,
    ) -> EvalResult<'tcx, (u64, u64)> {
        let pointer_size = self.memory.pointer_size();
        let size = self.memory.read_usize(vtable.offset(pointer_size, self)?)?;
        let align = self.memory.read_usize(
            vtable.offset(pointer_size * 2, self)?,
        )?;
        Ok((size, align))
    }

    pub(crate) fn resolve_associated_const(
        &self,
        def_id: DefId,
        substs: &'tcx Substs<'tcx>,
    ) -> ty::Instance<'tcx> {
        if let Some(trait_id) = self.tcx.trait_of_item(def_id) {
            let trait_ref = ty::Binder(ty::TraitRef::new(trait_id, substs));
            let vtable = self.fulfill_obligation(trait_ref);
            if let traits::VtableImpl(vtable_impl) = vtable {
                let name = self.tcx.item_name(def_id);
                let assoc_const_opt = self.tcx.associated_items(vtable_impl.impl_def_id).find(
                    |item| {
                        item.kind == ty::AssociatedKind::Const && item.name == name
                    },
                );
                if let Some(assoc_const) = assoc_const_opt {
                    return ty::Instance::new(assoc_const.def_id, vtable_impl.substs);
                }
            }
        }
        ty::Instance::new(def_id, substs)
    }
}
