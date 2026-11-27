use std::ffi::OsString;
use std::process::Command;
use std::vec::Vec;

use crate::shims::os_str::bytes_to_os_str;
use crate::*;

impl<'tcx> EvalContextExtPrivate<'tcx> for crate::MiriInterpCx<'tcx> {}
trait EvalContextExtPrivate<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn read_c_str_array(&self, array: Pointer) -> InterpResult<'tcx, Vec<OsString>> {
        let ecx = self.eval_context_ref();
        let mut strings = Vec::new();

        if ecx.ptr_is_null(array)? {
            return interp_ok(strings);
        }

        let el_layout = ecx.machine.layouts.unit_ptr_const;
        let el_size = el_layout.size;

        for idx in 0u64.. {
            let el = array.wrapping_offset(el_size * idx, ecx);
            let el_place = ecx.ptr_to_mplace(el, el_layout);
            let str_ptr = ecx.read_pointer(&el_place)?;

            if ecx.ptr_is_null(str_ptr)? {
                break;
            }

            let str = ecx.read_os_str_from_c_str(str_ptr)?;
            strings.push(str.to_owned());
        }

        interp_ok(strings)
    }

    fn read_envp(&self, envp: Pointer) -> InterpResult<'tcx, Vec<(OsString, OsString)>> {
        let ecx = self.eval_context_ref();
        let mut envps = Vec::new();

        if ecx.ptr_is_null(envp)? {
            return interp_ok(envps);
        }

        let envp_strings = ecx.read_c_str_array(envp)?;

        for osstr in envp_strings {
            let bytes = osstr.as_encoded_bytes();
            let Some(eq) = bytes.iter().position(|&b| b == b'=') else {
                throw_unsup_format!(
                    "posix_spawn: currently supports environment variables of the form 'key=value'"
                );
            };
            let (k, v) = (&bytes[..eq], &bytes[eq.strict_add(1)..]);
            let k = bytes_to_os_str(k)?;
            let v = bytes_to_os_str(v)?;
            envps.push((k.to_owned(), v.to_owned()));
        }

        interp_ok(envps)
    }
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn posix_spawn(
        &mut self,
        pid: &OpTy<'tcx>,
        path: &OpTy<'tcx>,
        file_actions: &OpTy<'tcx>,
        attrp: &OpTy<'tcx>,
        argv: &OpTy<'tcx>,
        envp: &OpTy<'tcx>,
    ) -> InterpResult<'tcx, Scalar> {
        let ecx = self.eval_context_mut();

        // Read Arguments
        let pid_place = ecx.deref_pointer(pid)?;
        let path = ecx.read_pointer(path)?;
        let file_actions = ecx.read_pointer(file_actions)?;
        let attrp = ecx.read_pointer(attrp)?;
        let argv = ecx.read_pointer(argv)?;
        let envp = ecx.read_pointer(envp)?;

        // validate arguments
        if !ecx.ptr_is_null(file_actions)? || !ecx.ptr_is_null(attrp)? {
            throw_unsup_format!(
                "posix_spawn: currently only supported with file_actions and attrp set to NULL"
            );
        }
        if ecx.ptr_is_null(path)? {
            return interp_ok(ecx.eval_libc("EFAULT"));
        }

        // Read Strings: path, argv, envp
        let path = ecx.read_path_from_c_str(path)?;
        let argv = ecx.read_c_str_array(argv)?;
        let envp = ecx.read_envp(envp)?;

        // Reject: if isolation enabled
        if let IsolatedOp::Reject(reject_with) = ecx.machine.isolated_op {
            ecx.reject_in_isolation("posix_spawn:", reject_with)?;
            return interp_ok(ecx.eval_libc("EPERM"));
        }

        // Create Command
        let mut command = Command::new(&*path);
        // FIXME:: file descriptor modifications done in Miri's environment not seen by the spawned process
        if let Some((arg0, args)) = argv.split_first() {
            cfg_select! {
                unix => {
                    use std::os::unix::process::CommandExt;
                    command.arg0(arg0);
                }
                _ =>
                    if *arg0 != path {
                        todo!("emit warning arg0 doesn't match path");
                    },
            }
            command.args(args);
        } else {
            throw_unsup_format!("posix_spawn: argv shouldn't be empty");
        }
        command.envs(envp.iter().map(|(k, v)| (k, v)));

        // Spawn
        let child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                return ecx.io_error_to_errnum(err);
            }
        };

        // Write pid
        if !ecx.ptr_is_null(pid_place.ptr())? {
            if let Ok(child_id_i32) = i32::try_from(child.id()) {
                let child_id = Scalar::from_i32(child_id_i32);
                ecx.write_scalar(child_id, &pid_place)?;
            } else {
                throw_unsup_format!("child pid doesn't fit into i32");
            };
        }

        interp_ok(Scalar::from_i32(0))
    }
}
