use rustc_middle::mir;
use rustc_target::spec::abi::Abi;

use crate::helpers::{check_abi, check_arg_count};
use crate::*;
use shims::posix::fs::EvalContextExt as _;
use shims::posix::linux::sync::futex;
use shims::posix::sync::EvalContextExt as _;
use shims::posix::thread::EvalContextExt as _;

impl<'mir, 'tcx: 'mir> EvalContextExt<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {}
pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriEvalContextExt<'mir, 'tcx> {
    fn emulate_foreign_item_by_name(
        &mut self,
        link_name: &str,
        abi: Abi,
        args: &[OpTy<'tcx, Tag>],
        dest: &PlaceTy<'tcx, Tag>,
        _ret: mir::BasicBlock,
    ) -> InterpResult<'tcx, bool> {
        let this = self.eval_context_mut();

        match link_name {
            // errno
            "__errno_location" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[] = check_arg_count(args)?;
                let errno_place = this.last_error_place()?;
                this.write_scalar(errno_place.to_ref().to_scalar()?, dest)?;
            }

            // File related shims (but also see "syscall" below for statx)
            // These symbols have different names on Linux and macOS, which is the only reason they are not
            // in the `posix` module.
            "close" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref fd] = check_arg_count(args)?;
                let result = this.close(fd)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }
            "opendir" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref name] = check_arg_count(args)?;
                let result = this.opendir(name)?;
                this.write_scalar(result, dest)?;
            }
            "readdir64_r" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref dirp, ref entry, ref result] = check_arg_count(args)?;
                let result = this.linux_readdir64_r(dirp, entry, result)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }
            "ftruncate64" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref fd, ref length] = check_arg_count(args)?;
                let result = this.ftruncate64(fd, length)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }
            // Linux-only
            "posix_fadvise" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref fd, ref offset, ref len, ref advice] = check_arg_count(args)?;
                this.read_scalar(fd)?.to_i32()?;
                this.read_scalar(offset)?.to_machine_isize(this)?;
                this.read_scalar(len)?.to_machine_isize(this)?;
                this.read_scalar(advice)?.to_i32()?;
                // fadvise is only informational, we can ignore it.
                this.write_null(dest)?;
            }
            "sync_file_range" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref fd, ref offset, ref nbytes, ref flags] = check_arg_count(args)?;
                let result = this.sync_file_range(fd, offset, nbytes, flags)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }

            // Time related shims
            "clock_gettime" => {
                check_abi(abi, Abi::C { unwind: false })?;
                // This is a POSIX function but it has only been tested on linux.
                let &[ref clk_id, ref tp] = check_arg_count(args)?;
                let result = this.clock_gettime(clk_id, tp)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }

            // Querying system information
            "pthread_attr_getstack" => {
                check_abi(abi, Abi::C { unwind: false })?;
                // We don't support "pthread_attr_setstack", so we just pretend all stacks have the same values here.
                let &[ref attr_place, ref addr_place, ref size_place] = check_arg_count(args)?;
                this.deref_operand(attr_place)?;
                let addr_place = this.deref_operand(addr_place)?;
                let size_place = this.deref_operand(size_place)?;

                this.write_scalar(
                    Scalar::from_uint(STACK_ADDR, this.pointer_size()),
                    &addr_place.into(),
                )?;
                this.write_scalar(
                    Scalar::from_uint(STACK_SIZE, this.pointer_size()),
                    &size_place.into(),
                )?;

                // Return success (`0`).
                this.write_null(dest)?;
            }

            // Threading
            "prctl" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref option, ref arg2, ref arg3, ref arg4, ref arg5] = check_arg_count(args)?;
                let result = this.prctl(option, arg2, arg3, arg4, arg5)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }
            "pthread_condattr_setclock" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref attr, ref clock_id] = check_arg_count(args)?;
                let result = this.pthread_condattr_setclock(attr, clock_id)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }
            "pthread_condattr_getclock" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref attr, ref clock_id] = check_arg_count(args)?;
                let result = this.pthread_condattr_getclock(attr, clock_id)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }

            // Dynamically invoked syscalls
            "syscall" => {
                check_abi(abi, Abi::C { unwind: false })?;
                // The syscall variadic function is legal to call with more arguments than needed,
                // extra arguments are simply ignored. However, all arguments need to be scalars;
                // other types might be treated differently by the calling convention.
                for arg in args {
                    if !matches!(arg.layout.abi, rustc_target::abi::Abi::Scalar(_)) {
                        throw_ub_format!(
                            "`syscall` arguments must all have scalar layout, but {} does not",
                            arg.layout.ty
                        );
                    }
                }

                let sys_getrandom = this.eval_libc("SYS_getrandom")?.to_machine_usize(this)?;

                let sys_statx = this.eval_libc("SYS_statx")?.to_machine_usize(this)?;

                let sys_futex = this.eval_libc("SYS_futex")?.to_machine_usize(this)?;

                if args.is_empty() {
                    throw_ub_format!(
                        "incorrect number of arguments for syscall: got 0, expected at least 1"
                    );
                }
                match this.read_scalar(&args[0])?.to_machine_usize(this)? {
                    // `libc::syscall(NR_GETRANDOM, buf.as_mut_ptr(), buf.len(), GRND_NONBLOCK)`
                    // is called if a `HashMap` is created the regular way (e.g. HashMap<K, V>).
                    id if id == sys_getrandom => {
                        // The first argument is the syscall id, so skip over it.
                        if args.len() < 4 {
                            throw_ub_format!(
                                "incorrect number of arguments for `getrandom` syscall: got {}, expected at least 4",
                                args.len()
                            );
                        }
                        getrandom(this, &args[1], &args[2], &args[3], dest)?;
                    }
                    // `statx` is used by `libstd` to retrieve metadata information on `linux`
                    // instead of using `stat`,`lstat` or `fstat` as on `macos`.
                    id if id == sys_statx => {
                        // The first argument is the syscall id, so skip over it.
                        if args.len() < 6 {
                            throw_ub_format!(
                                "incorrect number of arguments for `statx` syscall: got {}, expected at least 6",
                                args.len()
                            );
                        }
                        let result =
                            this.linux_statx(&args[1], &args[2], &args[3], &args[4], &args[5])?;
                        this.write_scalar(Scalar::from_machine_isize(result.into(), this), dest)?;
                    }
                    // `futex` is used by some synchonization primitives.
                    id if id == sys_futex => {
                        futex(this, args, dest)?;
                    }
                    id => throw_unsup_format!("Miri does not support syscall ID {}", id),
                }
            }

            // Miscelanneous
            "getrandom" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref ptr, ref len, ref flags] = check_arg_count(args)?;
                getrandom(this, ptr, len, flags, dest)?;
            }
            "sched_getaffinity" => {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref pid, ref cpusetsize, ref mask] = check_arg_count(args)?;
                this.read_scalar(pid)?.to_i32()?;
                this.read_scalar(cpusetsize)?.to_machine_usize(this)?;
                this.deref_operand(mask)?;
                // FIXME: we just return an error; `num_cpus` then falls back to `sysconf`.
                let einval = this.eval_libc("EINVAL")?;
                this.set_last_error(einval)?;
                this.write_scalar(Scalar::from_i32(-1), dest)?;
            }

            // Incomplete shims that we "stub out" just to get pre-main initialization code to work.
            // These shims are enabled only when the caller is in the standard library.
            "pthread_getattr_np"
                if this.frame().instance.to_string().starts_with("std::sys::unix::") =>
            {
                check_abi(abi, Abi::C { unwind: false })?;
                let &[ref _thread, ref _attr] = check_arg_count(args)?;
                this.write_null(dest)?;
            }

            _ => throw_unsup_format!("can't call foreign function: {}", link_name),
        };

        Ok(true)
    }
}

// Shims the linux `getrandom` syscall.
fn getrandom<'tcx>(
    this: &mut MiriEvalContext<'_, 'tcx>,
    ptr: &OpTy<'tcx, Tag>,
    len: &OpTy<'tcx, Tag>,
    flags: &OpTy<'tcx, Tag>,
    dest: &PlaceTy<'tcx, Tag>,
) -> InterpResult<'tcx> {
    let ptr = this.read_scalar(ptr)?.check_init()?;
    let len = this.read_scalar(len)?.to_machine_usize(this)?;

    // The only supported flags are GRND_RANDOM and GRND_NONBLOCK,
    // neither of which have any effect on our current PRNG.
    // See <https://github.com/rust-lang/rust/pull/79196> for a discussion of argument sizes.
    let _flags = this.read_scalar(flags)?.to_i32();

    this.gen_random(ptr, len)?;
    this.write_scalar(Scalar::from_machine_usize(len, this), dest)?;
    Ok(())
}
