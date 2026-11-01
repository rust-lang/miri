use std::ffi::CString;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

use rustc_middle::ty::Ty;
use rustc_span::Symbol;
use rustc_target::callconv::FnAbi;

use super::sync::EvalContextExt as _;
use crate::shims::unix::*;
use crate::*;

pub fn is_dyn_sym(_name: &str) -> bool {
    false
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn emulate_foreign_item_inner(
        &mut self,
        link_name: Symbol,
        abi: &FnAbi<'tcx, Ty<'tcx>>,
        args: &[OpTy<'tcx>],
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx, EmulateItemResult> {
        let this = self.eval_context_mut();
        match link_name.as_str() {
            // Threading
            "pthread_setname_np" => {
                let [thread, name] = &this.check_shim_sig(
                    shim_sig!(extern "C" fn(libc::pthread_t, *_) -> i32),
                    (link_name, abi, args),
                )?;
                let max_len = u64::MAX; // FreeBSD does not seem to have a limit.
                let res = match this.pthread_setname_np(
                    this.read_scalar(thread)?,
                    this.read_scalar(name)?,
                    max_len,
                    /* truncate */ false,
                )? {
                    ThreadNameResult::Ok => Scalar::from_u32(0),
                    ThreadNameResult::NameTooLong => unreachable!(),
                };
                this.write_scalar(res, dest)?;
            }
            "pthread_getname_np" => {
                let [thread, name, len] = &this.check_shim_sig(
                    shim_sig!(extern "C" fn(libc::pthread_t, *_, usize) -> i32),
                    (link_name, abi, args),
                )?;
                // FreeBSD's pthread_getname_np uses strlcpy, which truncates the resulting value,
                // but always adds a null terminator (except for zero-sized buffers).
                // https://github.com/freebsd/freebsd-src/blob/c2d93a803acef634bd0eede6673aeea59e90c277/lib/libthr/thread/thr_info.c#L119-L144
                let res = match this.pthread_getname_np(
                    this.read_scalar(thread)?,
                    this.read_scalar(name)?,
                    this.read_scalar(len)?,
                    /* truncate */ true,
                )? {
                    ThreadNameResult::Ok => Scalar::from_u32(0),
                    // `NameTooLong` is possible when the buffer is zero sized,
                    ThreadNameResult::NameTooLong => Scalar::from_u32(0),
                };
                this.write_scalar(res, dest)?;
            }
            "pthread_getthreadid_np" => {
                let [] = &this
                    .check_shim_sig(shim_sig!(extern "C" fn() -> i32), (link_name, abi, args))?;
                let result = this.unix_gettid(link_name.as_str())?;
                this.write_scalar(result, dest)?;
            }

            "cpuset_getaffinity" => {
                // The "same" kind of api as `sched_getaffinity` but more fine grained control for FreeBSD specifically.
                let [level, which, id, set_size, mask] = &this.check_shim_sig(
                    shim_sig!(extern "C" fn(i32, i32, i64, usize, *_) -> i32),
                    (link_name, abi, args),
                )?;

                let level = this.read_scalar(level)?.to_i32()?;
                let which = this.read_scalar(which)?.to_i32()?;
                let id = this.read_scalar(id)?.to_i64()?;
                let set_size = this.read_target_usize(set_size)?; // measured in bytes
                let mask = this.read_pointer(mask)?;

                if this.machine.thread_cpu_affinity.is_none() {
                    throw_unsup_format!(
                        "`cpuset_getaffinity` is not supported on #![no_core] programs"
                    )
                }

                let _level_root = this.eval_libc_i32("CPU_LEVEL_ROOT");
                let _level_cpuset = this.eval_libc_i32("CPU_LEVEL_CPUSET");
                let level_which = this.eval_libc_i32("CPU_LEVEL_WHICH");

                let _which_tid = this.eval_libc_i32("CPU_WHICH_TID");
                let which_pid = this.eval_libc_i32("CPU_WHICH_PID");
                let _which_jail = this.eval_libc_i32("CPU_WHICH_JAIL");
                let _which_cpuset = this.eval_libc_i32("CPU_WHICH_CPUSET");
                let _which_irq = this.eval_libc_i32("CPU_WHICH_IRQ");

                // For sched_getaffinity, the current process is identified by -1.
                // TODO: Use gettid? I'm (LorrensP-2158466) not that familiar with this api .
                let id = match id {
                    -1 => this.active_thread(),
                    _ =>
                        throw_unsup_format!(
                            "`cpuset_getaffinity` is only supported with a pid of -1 (indicating the current thread)"
                        ),
                };

                if this.ptr_is_null(mask)? {
                    this.set_errno_and_return_neg1(LibcError("EFAULT"), dest)?;
                }
                // We only support CPU_LEVEL_WHICH and CPU_WHICH_PID for now.
                // This is the bare minimum to make the tests pass.
                else if level != level_which || which != which_pid {
                    throw_unsup_format!(
                        "`cpuset_getaffinity` is only supported with `level` set to CPU_LEVEL_WHICH and `which` set to CPU_WHICH_PID."
                    );
                } else if let Some(cpuset) =
                    this.machine.thread_cpu_affinity.as_ref().unwrap().get(&id)
                {
                    // `cpusetsize` must be large enough to contain the entire CPU mask.
                    // FreeBSD only uses `cpusetsize` to verify that it's sufficient for the kernel's CPU mask.
                    // If it's too small, the syscall returns ERANGE.
                    // If it's large enough, copying the kernel mask to user space is safe, regardless of the actual size.
                    // See https://github.com/freebsd/freebsd-src/blob/909aa6781340f8c0b4ae01c6366bf1556ee2d1be/sys/kern/kern_cpuset.c#L1985
                    if set_size < u64::from(this.machine.num_cpus).div_ceil(8) {
                        this.set_errno_and_return_neg1(LibcError("ERANGE"), dest)?;
                    } else {
                        let cpuset = cpuset.clone();
                        let byte_count =
                            Ord::min(cpuset.as_slice().len(), set_size.try_into().unwrap());
                        this.write_bytes_ptr(
                            mask,
                            cpuset.as_slice()[..byte_count].iter().copied(),
                        )?;
                        this.write_null(dest)?;
                    }
                } else {
                    // `id` is always that of the active thread, so this is currently unreachable.
                    unreachable!();
                }
            }

            // We only implement `sysctl` to support `std::env::current_exe` for now.
            "sysctl" => 'sysctl: {
                let [names, name_len, old_p, old_len_p, new_p, new_p_len] = this.check_shim_sig(
                    shim_sig!(extern "C" fn(*i32, u32, *_, *u32, *_, u32) -> i32),
                    (link_name, abi, args),
                )?;

                // Reject if isolation is enabled.
                if let IsolatedOp::Reject(reject_with) = this.machine.isolated_op {
                    this.reject_in_isolation("`sysctl`", reject_with)?;
                    this.set_errno_and_return_neg1(ErrorKind::PermissionDenied, dest)?;
                    break 'sysctl;
                }

                // Man page: If a new value is not to be set, newp should be set to NULL and
                // newlen set to 0
                // So we make sure the user never wants to update a name.
                if !this.ptr_is_null(this.read_pointer(&new_p)?)?
                    && this.read_scalar(&new_p_len)?.to_u32()? != 0
                {
                    throw_unsup_format!("`sysctl` does not support setting new values");
                }

                let old_p = this.read_pointer(&old_p)?;

                // Man page: If	the old value is not desired, oldp and oldlenp should be set to NULL
                // So nothing to do for us here.
                if this.ptr_is_null(old_p)? && this.ptr_is_null(this.read_pointer(&old_len_p)?)? {
                    break 'sysctl;
                }

                let ctl_maxname = u32::try_from(this.eval_libc_i32("CTL_MAXNAME"))
                    .expect("CTL_MAXNAME is always positive");
                let name_len = this.read_scalar(&name_len)?.to_u32()?;
                if name_len < 2 || name_len > ctl_maxname {
                    this.set_errno_and_return_neg1(LibcError("EINVAL"), dest)?;
                    break 'sysctl;
                }

                let layout = this.machine.layouts.i32;
                let names_place = this.ptr_to_mplace(this.read_pointer(&names)?, layout);
                let mut names = this.project_array_fields(&names_place)?;

                let ctl_kern = this.eval_libc_i32("CTL_KERN");
                let kern_proc = this.eval_libc_i32("KERN_PROC");
                let kern_proc_path = this.eval_libc_i32("KERN_PROC_PATHNAME");

                let Some((_, first_name)) = names.next(this)? else {
                    // The given length of the `names` array is >= 2 but the array itself is empty.
                    throw_ub_format!("Out of bounds read");
                };
                if ctl_kern != this.read_scalar(&first_name)?.to_i32()? {
                    throw_unsup_format!(
                        "`sysctl` only accepts `CTL_KERN` as top-level name to support `std::env::current_exe`"
                    )
                }

                let Some((_, second_name)) = names.next(this)? else {
                    // The given length of the `names` array is >= 2 but the array itself is empty
                    throw_ub_format!("Out of bounds read");
                };
                if kern_proc != this.read_scalar(&second_name)?.to_i32()? {
                    throw_unsup_format!(
                        "`sysctl` only accepts `KERN_PROC` as a second-level name to support `std::env::current_exe`"
                    );
                }

                let Some((_, third_name)) = names.next(this)? else {
                    // Man page: The name array specifies  an  intermediate  rather  than
                    // terminal name.
                    this.set_errno_and_return_neg1(LibcError("ENOTDIR"), dest)?;
                    break 'sysctl;
                };
                if kern_proc_path != this.read_scalar(&third_name)?.to_i32()? {
                    throw_unsup_format!(
                        "`sysctl` only accepts `KERN_PROC_PATHNAME` as a third-level name to support `std::env::current_exe`"
                    );
                }

                let Some((_, last_name)) = names.next(this)? else {
                    // Man page: The name array specifies  an  intermediate  rather  than
                    // terminal name.
                    this.set_errno_and_return_neg1(LibcError("ENOTDIR"), dest)?;
                    break 'sysctl;
                };

                // we only support the current process due to `std::env::current_exe`
                if -1 != this.read_scalar(&last_name)?.to_i32()? {
                    throw_unsup_format!(
                        "`sysctl` only supports returning the current process' path"
                    );
                }

                match std::env::current_exe() {
                    Ok(path) => {
                        let path_name = CString::new(path.as_path().as_os_str().as_bytes())
                            .expect("we did not insert a null-byte ourselves");
                        let path_name_bytes = path_name.as_bytes_with_nul(); // write as a c_str

                        let path_name_len = u32::try_from(path_name_bytes.len())
                            .expect("the path name can't be that large");
                        let old_len_place = this.deref_pointer(&old_len_p)?;

                        if this.ptr_is_null(old_p)? {
                            // Man page: The size of the available data can be determined by calling
                            // sysctl() with the NULL argument for oldp.
                            this.write_int(path_name_len, &old_len_place)?;
                            this.write_int(0, dest)?
                        } else {
                            let old_len = this.read_scalar(&old_len_place)?.to_u32()?;

                            if old_len >= path_name_len {
                                // We can freely write the path name.
                                this.write_bytes_ptr(old_p, path_name_bytes.iter().copied())?;
                                this.write_int(path_name_len, &old_len_place)?;
                            } else {
                                // Man page: If the amount of data available is greater than the
                                // size of the buffer supplied, the call supplies as much data as
                                // fits in the buffer provided and returns with the error
                                // code ENOMEM
                                let old_len = usize::try_from(old_len).unwrap();
                                this.write_bytes_ptr(
                                    old_p,
                                    path_name_bytes[..old_len].iter().copied(),
                                )?;
                                this.set_errno_and_return_neg1(LibcError("ENOMEM"), dest)?;
                            }
                        }
                    }
                    Err(err) => {
                        // convert the host error and return
                        this.set_errno_and_return_neg1(err, dest)?;
                    }
                }
            }

            // Synchronization primitives
            "_umtx_op" => {
                let [obj, op, val, uaddr, uaddr2] = &this.check_shim_sig(
                    shim_sig!(extern "C" fn(*_, i32, usize, *_, *_) -> i32),
                    (link_name, abi, args),
                )?;
                this._umtx_op(obj, op, val, uaddr, uaddr2, dest)?;
            }

            // Miscellaneous
            "__error" => {
                let [] = &this
                    .check_shim_sig(shim_sig!(extern "C" fn() -> *_), (link_name, abi, args))?;
                let errno_place = this.last_error_place()?;
                this.write_scalar(errno_place.to_ref(this).to_scalar(), dest)?;
            }
            "__xuname" => {
                // FreeBSD uses __xuname under the hood to implement uname, see:
                // https://github.com/freebsd/freebsd-src/blob/3542d60fb8042474f66fbf2d779ed8c5a80d0f78/sys/sys/utsname.h#L64
                // https://github.com/freebsd/freebsd-src/blob/3542d60fb8042474f66fbf2d779ed8c5a80d0f78/lib/libc/gen/uname.c#L44
                let [size, uname] = &this.check_shim_sig(
                    shim_sig!(extern "C" fn(i32, *libc::utsname) -> i32),
                    (link_name, abi, args),
                )?;
                let result = this.uname(uname, Some(size))?;
                this.write_scalar(result, dest)?;
            }

            // Incomplete shims that we "stub out" just to get pre-main initialization code to work.
            // These shims are enabled only when the caller is in the standard library.
            "pthread_attr_get_np" if this.frame_in_std() => {
                let [_thread, _attr] = &this.check_shim_sig(
                    shim_sig!(extern "C" fn(libc::pthread_t, *_) -> i32),
                    (link_name, abi, args),
                )?;
                this.write_null(dest)?;
            }

            _ => return interp_ok(EmulateItemResult::NotSupported),
        }
        interp_ok(EmulateItemResult::NeedsReturn)
    }
}
