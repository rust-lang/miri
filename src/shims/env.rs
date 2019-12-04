use std::collections::HashMap;
use std::ffi::OsString;
use std::env;

use crate::stacked_borrows::Tag;
use crate::*;

use rustc::ty::layout::Size;
use rustc_mir::interpret::{Memory, Pointer};

#[derive(Default)]
pub struct EnvVars {
    /// Stores pointers to the environment variables. These variables must be stored as
    /// null-terminated C strings with the `"{name}={value}"` format.
    map: HashMap<Vec<u8>, Pointer<Tag>>,
}

impl EnvVars {
    pub(crate) fn init<'mir, 'tcx>(
        ecx: &mut InterpCx<'mir, 'tcx, Evaluator<'tcx>>,
        mut excluded_env_vars: Vec<String>,
    ) {
        // Exclude `TERM` var to avoid terminfo trying to open the termcap file.
        excluded_env_vars.push("TERM".to_owned());

        if ecx.machine.communicate {
            for (name, value) in env::vars() {
                if !excluded_env_vars.contains(&name) {
                    let var_ptr =
                        alloc_env_var(name.as_bytes(), value.as_bytes(), &mut ecx.memory);
                    ecx.machine.env_vars.map.insert(name.into_bytes(), var_ptr);
                }
            }
        }
    }
}

fn alloc_env_var<'mir, 'tcx>(
    name: &[u8],
    value: &[u8],
    memory: &mut Memory<'mir, 'tcx, Evaluator<'tcx>>,
) -> Pointer<Tag> {
    let mut bytes = name.to_vec();
    bytes.push(b'=');
    bytes.extend_from_slice(value);
    bytes.push(0);
    memory.allocate_static_bytes(bytes.as_slice(), MiriMemoryKind::Env.into())
}

impl<'mir, 'tcx> EvalContextExt<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {}
pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriEvalContextExt<'mir, 'tcx> {
    fn getenv(&mut self, name_op: OpTy<'tcx, Tag>) -> InterpResult<'tcx, Scalar<Tag>> {
        let this = self.eval_context_mut();

        let name_ptr = this.read_scalar(name_op)?.not_undef()?;
        let name = this.memory.read_c_str(name_ptr)?;
        Ok(match this.machine.env_vars.map.get(name) {
            // The offset is used to strip the "{name}=" part of the string.
            Some(var_ptr) => {
                Scalar::from(var_ptr.offset(Size::from_bytes(name.len() as u64 + 1), this)?)
            }
            None => Scalar::ptr_null(&*this.tcx),
        })
    }

    fn getenvironmentvariablew(
        &mut self, 
        name_op: OpTy<'tcx, Tag>,
        lpbuffer_op: OpTy<'tcx, Tag>,
    ) -> InterpResult<'tcx, u32> {
        let this = self.eval_context_mut();

        let name_ptr = this.read_scalar(name_op)?.not_undef()?;
        let name = this.memory.read_wide_str(name_ptr)?;
        let lpbuf_ptr = this.read_scalar(buffer_op)?.not_undef()?;
        Ok(match this.machine.env_vars.map.get(name) {
            Some(var_ptr) => {
                let var = this.memory.read_wide_str(var_ptr);
                // Write contents of env_var to lpBuffer
                this.memory.write_bytes(lpbuf_ptr, var);
                // `var` is a byte slice, but will be interpreted as unicode string.
                // one unicode character equals 2 bytes. 
                (var.len() >> 1) as u32
            }
            None => 0,
        })
    }

    fn setenv(
        &mut self,
        name_op: OpTy<'tcx, Tag>,
        value_op: OpTy<'tcx, Tag>,
    ) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let name_ptr = this.read_scalar(name_op)?.not_undef()?;
        let value_ptr = this.read_scalar(value_op)?.not_undef()?;
        let value = this.memory.read_c_str(value_ptr)?;
        let mut new = None;
        if !this.is_null(name_ptr)? {
            let name = this.memory.read_c_str(name_ptr)?;
            if !name.is_empty() && !name.contains(&b'=') {
                new = Some((name.to_owned(), value.to_owned()));
            }
        }
        if let Some((name, value)) = new {
            let var_ptr = alloc_env_var(&name, &value, &mut this.memory);
            if let Some(var) = this.machine.env_vars.map.insert(name.to_owned(), var_ptr) {
                this.memory
                    .deallocate(var, None, MiriMemoryKind::Env.into())?;
            }
            Ok(0)
        } else {
            Ok(-1)
        }
    }

    fn setenvironmentvariablew(
        &mut self,
        name_op: OpTy<'tcx, Tag>,
        value_op: OpTy<'tcx, Tag>,
    ) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let name_ptr = this.read_scalar(name_op)?.not_undef()?;
        let value_ptr = this.read_scalar(value_op)?.not_undef()?;
        let value = this.memory.read_wide_str(value_ptr)?;
        let mut new = None;
        if !this.is_null(name_ptr)? {
            let name = this.memory.read_wide_str(name_ptr)?;
            if !name.is_empty() && !name.contains(&b'=') {
                new = Some((name.to_owned(), value.to_owned()));
            }
        }
        if let Some((name, value)) = new {
            let var_ptr = alloc_env_var(&name, &value, &mut this.memory);
            if let Some(var) = this.machine.env_vars.map.insert(name.to_owned(), var_ptr) {
                this.memory
                    .deallocate(var, None, MiriMemoryKind::Env.into())?;
            }
            Ok(1) // return non-zero if success
        } else {
            Ok(0) // return upon failure
        }
    }

    fn unsetenv(&mut self, name_op: OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let name_ptr = this.read_scalar(name_op)?.not_undef()?;
        let mut success = None;
        if !this.is_null(name_ptr)? {
            let name = this.memory.read_c_str(name_ptr)?.to_owned();
            if !name.is_empty() && !name.contains(&b'=') {
                success = Some(this.machine.env_vars.map.remove(&name));
            }
        }
        if let Some(old) = success {
            if let Some(var) = old {
                this.memory
                    .deallocate(var, None, MiriMemoryKind::Env.into())?;
            }
            Ok(0)
        } else {
            Ok(-1)
        }
    }

    fn getcwd(
        &mut self,
        buf_op: OpTy<'tcx, Tag>,
        size_op: OpTy<'tcx, Tag>,
    ) -> InterpResult<'tcx, Scalar<Tag>> {
        let this = self.eval_context_mut();

        this.check_no_isolation("getcwd")?;

        let buf = this.read_scalar(buf_op)?.not_undef()?;
        let size = this.read_scalar(size_op)?.to_machine_usize(&*this.tcx)?;
        // If we cannot get the current directory, we return null
        match env::current_dir() {
            Ok(cwd) => {
                if this.write_os_str_to_c_string(&OsString::from(cwd), buf, size)? {
                    return Ok(buf);
                }
                let erange = this.eval_libc("ERANGE")?;
                this.set_last_error(erange)?;
            }
            Err(e) => this.set_last_error_from_io_error(e)?,
        }
        Ok(Scalar::ptr_null(&*this.tcx))
    }

    fn chdir(&mut self, path_op: OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        this.check_no_isolation("chdir")?;

        let path = this.read_os_string_from_c_string(this.read_scalar(path_op)?.not_undef()?)?;

        match env::set_current_dir(path) {
            Ok(()) => Ok(0),
            Err(e) => {
                this.set_last_error_from_io_error(e)?;
                Ok(-1)
            }
        }
    }
}
