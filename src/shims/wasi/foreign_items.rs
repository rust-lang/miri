use rustc_abi::CanonAbi;
use rustc_middle::ty::{Instance, Ty};
use rustc_span::Symbol;
use rustc_target::callconv::FnAbi;

use crate::shims::alloc::EvalContextExt as _;
use crate::*;

pub fn is_dyn_sym(_name: &str) -> bool {
    false
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn emulate_foreign_item_inner(
        &mut self,
        instance: Option<Instance<'tcx>>,
        link_name: Symbol,
        abi: &FnAbi<'tcx, Ty<'tcx>>,
        args: &[OpTy<'tcx>],
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx, EmulateItemResult> {
        let this = self.eval_context_mut();

        let (interface, name) = if let Some(instance) = instance
            && let Some(module) =
                this.tcx.wasm_import_module_map(instance.def_id().krate).get(&instance.def_id())
        {
            // Adapted from https://github.com/rust-lang/rust/blob/90b65889799733f21ebdf59d96411aa531c5900a/compiler/rustc_codegen_llvm/src/attributes.rs#L549-L562
            let codegen_fn_attrs = this.tcx.codegen_instance_attrs(instance.def);
            let name = codegen_fn_attrs
                .symbol_name
                .unwrap_or_else(|| this.tcx.item_name(instance.def_id()));

            // According to the component model, the version should be matched as semver, but for
            // simplicity we strip the version entirely for now. Once we support wasm-wasip3 it may
            // become actually important to match on the version, but for now it shouldn't matter.
            let (interface, _version) = module
                .split_once('@')
                .ok_or_else(|| err_unsup_format!("module name {module} must contain a version"))?;

            (Some(interface), name)
        } else {
            // This item is provided by wasi-libc, not imported from the wasi runtime
            (None, link_name)
        };

        match (interface, name.as_str()) {
            // Allocation
            (None, "posix_memalign") => {
                let [memptr, align, size] =
                    this.check_shim_sig_lenient(abi, CanonAbi::C, link_name, args)?;
                let result = this.posix_memalign(memptr, align, size)?;
                this.write_scalar(result, dest)?;
            }
            (None, "aligned_alloc") => {
                let [align, size] =
                    this.check_shim_sig_lenient(abi, CanonAbi::C, link_name, args)?;
                let res = this.aligned_alloc(align, size)?;
                this.write_pointer(res, dest)?;
            }

            // Standard input/output
            // FIXME: These shims are hacks that just get basic stdout/stderr working. We can't
            // constrain them to "std" since std itself uses the wasi crate for this.
            (Some("wasi:cli/stdout"), "get-stdout") => {
                let [] =
                    this.check_shim_sig(shim_sig!(extern "C" fn() -> i32), link_name, abi, args)?;
                this.write_scalar(Scalar::from_i32(1), dest)?; // POSIX FD number for stdout
            }
            (Some("wasi:cli/stderr"), "get-stderr") => {
                let [] =
                    this.check_shim_sig(shim_sig!(extern "C" fn() -> i32), link_name, abi, args)?;
                this.write_scalar(Scalar::from_i32(2), dest)?; // POSIX FD number for stderr
            }
            (Some("wasi:io/streams"), "[resource-drop]output-stream") => {
                let [handle] =
                    this.check_shim_sig(shim_sig!(extern "C" fn(i32) -> ()), link_name, abi, args)?;
                let handle = this.read_scalar(handle)?.to_i32()?;

                if !(handle == 1 || handle == 2) {
                    throw_unsup_format!("wasm output-stream: unsupported handle");
                }
                // We don't actually close these FDs, so this is a NOP.
            }
            (Some("wasi:io/streams"), "[method]output-stream.blocking-write-and-flush") => {
                let [handle, buf, len, ret_area] = this.check_shim_sig(
                    shim_sig!(extern "C" fn(i32, *mut _, usize, *mut _) -> ()),
                    link_name,
                    abi,
                    args,
                )?;
                let handle = this.read_scalar(handle)?.to_i32()?;
                let buf = this.read_pointer(buf)?;
                let len = this.read_target_usize(len)?;
                let ret_area = this.read_pointer(ret_area)?;

                if len > 4096 {
                    throw_unsup_format!(
                        "wasm output-stream.blocking-write-and-flush: buffer too big"
                    );
                }
                let len = usize::try_from(len).unwrap();
                let Some(fd) = this.machine.fds.get(handle) else {
                    throw_unsup_format!(
                        "wasm output-stream.blocking-write-and-flush: unsupported handle"
                    );
                };
                fd.write(
                    this.machine.communicate(),
                    buf,
                    len,
                    this,
                    callback!(
                        @capture<'tcx> {
                            len: usize,
                            ret_area: Pointer,
                        }
                        |this, result: Result<usize, IoError>| {
                            if !matches!(result, Ok(l) if l == len) {
                                throw_unsup_format!("wasm output-stream.blocking-write-and-flush: returning errors is not supported");
                            }
                            // 0 in the first byte of the ret_area indicates success.
                            let ret = this.ptr_to_mplace(ret_area, this.machine.layouts.u8);
                            this.write_null(&ret)?;
                            interp_ok(())
                    }),
                )?;
            }

            _ => return interp_ok(EmulateItemResult::NotSupported),
        }
        interp_ok(EmulateItemResult::NeedsReturn)
    }
}
