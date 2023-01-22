use std::ffi::OsString;

use rustc_span::Symbol;
use rustc_target::abi::Size;
use rustc_target::spec::abi::Abi;

use crate::*;
use shims::foreign_items::EmulateByNameResult;

use rustc_middle::ty::layout::LayoutOf as _;
use rustc_middle::ty::Ty;

impl<'mir, 'tcx: 'mir> EvalContextExt<'mir, 'tcx> for crate::MiriInterpCx<'mir, 'tcx> {}
pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriInterpCxExt<'mir, 'tcx> {
    fn emulate_foreign_item_by_name(
        &mut self,
        link_name: Symbol,
        abi: Abi,
        args: &[OpTy<'tcx, Provenance>],
        dest: &PlaceTy<'tcx, Provenance>,
    ) -> InterpResult<'tcx, EmulateByNameResult<'mir, 'tcx>> {
        let this = self.eval_context_mut();

        // See `fn emulate_foreign_item_by_name` in `shims/foreign_items.rs` for the general pattern.
        match link_name.as_str() {
            "args_sizes_get" => {
                let [num_args_ptr, cumulative_size_ptr] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                let ptr_usize = this.tcx.mk_mut_ptr(this.tcx.types.usize);

                let num_args_ptr = this.addr_to_mplace_ty(num_args_ptr, ptr_usize)?;
                let cumulative_size_ptr = this.addr_to_mplace_ty(cumulative_size_ptr, ptr_usize)?;

                let argc: u32 = this.machine.args.len().try_into().unwrap();
                let cumulative_size: u32 = this
                    .machine
                    .args
                    .iter()
                    .map(|e| {
                        // Include null terminator
                        e.as_bytes().len().checked_add(1).unwrap()
                    })
                    .sum::<usize>()
                    .try_into()
                    .unwrap();

                // Store argc and cumulative size into the pointers passed in as arguments.
                this.write_scalar(Scalar::from_u32(argc), &num_args_ptr.into())?;
                this.write_scalar(Scalar::from_u32(cumulative_size), &cumulative_size_ptr.into())?;

                this.write_scalar(Scalar::from_u32(0), dest)?;
            }

            "args_get" => {
                let [argv, argv_buf] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                this.args_get(argv, argv_buf)?;

                this.write_scalar(Scalar::from_u32(0), dest)?;
            }

            "fd_write" => {
                let [fd, iovs, iovs_len, rp0] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                let iov_layout = this.libc_ty_layout("iovec");
                let iov_ptr_ty = this.tcx.mk_mut_ptr(iov_layout.ty);
                let iov_ptr_layout = this.layout_of(iov_ptr_ty).unwrap();

                let fd = this.read_scalar(fd)?.to_i32()?;
                let iovs = this.addr_to_mplace_ty(iovs, iov_ptr_ty)?;
                let iovs_len: u64 = this.read_scalar(iovs_len)?.to_u32()?.into();

                let mut total: i64 = 0;

                // Use the unix implementation of 'write' to process our iovv buffers
                // one at a time.
                for i in 0..iovs_len {
                    let iov_elem = iovs.offset(i * iov_ptr_layout.size, iov_layout, this)?;

                    let buf = this.mplace_field(&iov_elem, 0)?;
                    let buf = this.read_pointer(&buf.into())?;

                    let count = this.mplace_field(&iov_elem, 1)?;
                    let count = this.read_scalar(&count.into())?.to_u32()?;

                    use crate::shims::unix::fs::EvalContextExt;

                    let res = this.write(fd, buf, count.try_into().unwrap()).unwrap();
                    if res == -1 {
                        this.write_scalar(Scalar::from_i32(-1), dest)?;
                        return Ok(EmulateByNameResult::NeedsJumping);
                    }
                    total = total.checked_add(res).unwrap();
                }

                let rp0 = this.addr_to_mplace_ty(rp0, this.tcx.mk_mut_ptr(this.tcx.types.usize))?;
                this.write_scalar(Scalar::from_i32(total.try_into().unwrap()), &rp0.into())?;
                this.write_scalar(Scalar::from_u32(0), dest)?;
            }

            "random_get" => {
                let [ptr, len] = this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                let buf = this.addr_to_mplace_ty(ptr, this.tcx.types.u32)?;
                let len = this.read_machine_usize(len)?;

                this.gen_random(buf.ptr, len)?;

                // Newer versions of the `wasi` crate return a u32, while
                // older versions return a i16. We support both.
                let res = match dest.layout.ty {
                    ty if ty == this.tcx.types.i32 => Scalar::from_i32(0),
                    ty if ty == this.tcx.types.u16 => Scalar::from_u16(0),
                    ty => panic!("Unsupported random_get return type: {ty:?}"),
                };
                this.write_scalar(res, dest)?;
            }

            _ => return Ok(EmulateByNameResult::NotSupported),
        }

        Ok(EmulateByNameResult::NeedsJumping)
    }

    fn args_get(
        &mut self,
        element_heads: &OpTy<'tcx, Provenance>,
        buffer: &OpTy<'tcx, Provenance>,
    ) -> InterpResult<'tcx, ()> {
        let this = self.eval_context_mut();

        let ptr_u8 = this.tcx.mk_mut_ptr(this.tcx.types.u8);
        let ptr_ptr_u8 = this.tcx.mk_mut_ptr(ptr_u8);

        let ptr_u8_layout = this.layout_of(ptr_u8).unwrap();

        let element_heads = this.addr_to_mplace_ty(element_heads, ptr_ptr_u8)?;
        let buffer = this.addr_to_mplace_ty(buffer, ptr_u8)?;

        // Based on
        // https://github.com/bytecodealliance/wasmtime/blob/f85e3f85170bd2c8378977879f4fdf2c6b64de55/crates/wasi-common/src/string_array.rs#L48
        let mut cursor: u32 = 0;
        // Can't iterative directly over it to due borrowcheck conflict with 'this'
        for i in 0..this.machine.args.len() {
            let arg = &this.machine.args[i];
            let bytes = arg.as_bytes();
            let len: u32 = bytes.len().try_into().unwrap();
            // Make space for `0` terminator.
            let size = u64::try_from(arg.len()).unwrap().checked_add(1).unwrap();

            let buffer_loc = buffer.ptr.offset(Size::from_bytes(cursor), this)?;

            let arg = OsString::from(arg.to_owned());

            this.write_os_str_to_c_str(&arg, buffer_loc, size)?;

            let head_ptr = element_heads.ptr.offset((i as u64) * ptr_u8_layout.size, this)?;
            let head_ptr_mplace = MPlaceTy::from_aligned_ptr(head_ptr, ptr_u8_layout);

            this.write_pointer(buffer_loc, &head_ptr_mplace.into())?;
            cursor = cursor.checked_add(len).unwrap().checked_add(1).unwrap();
        }
        Ok(())
    }

    fn addr_to_mplace_ty(
        &mut self,
        arg: &OpTy<'tcx, Provenance>,
        ptr_ty: Ty<'tcx>,
    ) -> InterpResult<'tcx, MPlaceTy<'tcx, Provenance>> {
        let this = self.eval_context_mut();
        let arg = this.read_scalar(arg)?;
        //  Hack: New versions of the `wasi` crate perform an int-to-ptr cast before making a syscall,
        //  while older versions pass a pointer directly. We need to handle both cases.
        let ptr = match arg {
            Scalar::Int(_) => {
                #[allow(clippy::cast_sign_loss)] // We want to lose the sign.
                crate::intptrcast::GlobalStateInner::ptr_from_addr_cast(this, arg.to_i32()? as u64)?
            }
            Scalar::Ptr(ptr, _) => ptr.into(),
        };
        Ok(MPlaceTy::from_aligned_ptr(ptr, this.layout_of(ptr_ty).unwrap()))
    }
}
