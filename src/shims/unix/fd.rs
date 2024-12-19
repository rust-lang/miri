//! General management of file descriptors, and support for
//! standard file descriptors (stdin/stdout/stderr).

use std::io;
use std::io::ErrorKind;

use rustc_abi::Size;

use crate::helpers::check_min_arg_count;
use crate::shims::files::FileDescription;
use crate::shims::unix::linux_like::epoll::EpollReadyEvents;
use crate::shims::unix::*;
use crate::*;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum FlockOp {
    SharedLock { nonblocking: bool },
    ExclusiveLock { nonblocking: bool },
    Unlock,
}

/// Represents unix-specific file descriptions.
pub trait UnixFileDescription: FileDescription {
    /// Reads as much as possible into the given buffer `ptr` from a given offset.
    /// `len` indicates how many bytes we should try to read.
    /// `dest` is where the return value should be stored: number of bytes read, or `-1` in case of error.
    fn pread<'tcx>(
        &self,
        _communicate_allowed: bool,
        _offset: u64,
        _ptr: Pointer,
        _len: usize,
        _dest: &MPlaceTy<'tcx>,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx> {
        throw_unsup_format!("cannot pread from {}", self.name());
    }

    /// Writes as much as possible from the given buffer `ptr` starting at a given offset.
    /// `ptr` is the pointer to the user supplied read buffer.
    /// `len` indicates how many bytes we should try to write.
    /// `dest` is where the return value should be stored: number of bytes written, or `-1` in case of error.
    fn pwrite<'tcx>(
        &self,
        _communicate_allowed: bool,
        _ptr: Pointer,
        _len: usize,
        _offset: u64,
        _dest: &MPlaceTy<'tcx>,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx> {
        throw_unsup_format!("cannot pwrite to {}", self.name());
    }

    fn flock<'tcx>(
        &self,
        _communicate_allowed: bool,
        _op: FlockOp,
    ) -> InterpResult<'tcx, io::Result<()>> {
        throw_unsup_format!("cannot flock {}", self.name());
    }

    /// Check the readiness of file description.
    fn get_epoll_ready_events<'tcx>(&self) -> InterpResult<'tcx, EpollReadyEvents> {
        throw_unsup_format!("{}: epoll does not support this file description", self.name());
    }
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn dup(&mut self, old_fd_num: i32) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        let Some(fd) = this.machine.fds.get(old_fd_num) else {
            return this.set_last_error_and_return_i32(LibcError("EBADF"));
        };
        interp_ok(Scalar::from_i32(this.machine.fds.insert(fd)))
    }

    fn dup2(&mut self, old_fd_num: i32, new_fd_num: i32) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        let Some(fd) = this.machine.fds.get(old_fd_num) else {
            return this.set_last_error_and_return_i32(LibcError("EBADF"));
        };
        if new_fd_num != old_fd_num {
            // Close new_fd if it is previously opened.
            // If old_fd and new_fd point to the same description, then `dup_fd` ensures we keep the underlying file description alive.
            if let Some(old_new_fd) = this.machine.fds.fds.insert(new_fd_num, fd) {
                // Ignore close error (not interpreter's) according to dup2() doc.
                old_new_fd.close(this.machine.communicate(), this)?.ok();
            }
        }
        interp_ok(Scalar::from_i32(new_fd_num))
    }

    fn flock(&mut self, fd_num: i32, op: i32) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();
        let Some(fd) = this.machine.fds.get(fd_num) else {
            return this.set_last_error_and_return_i32(LibcError("EBADF"));
        };

        // We need to check that there aren't unsupported options in `op`.
        let lock_sh = this.eval_libc_i32("LOCK_SH");
        let lock_ex = this.eval_libc_i32("LOCK_EX");
        let lock_nb = this.eval_libc_i32("LOCK_NB");
        let lock_un = this.eval_libc_i32("LOCK_UN");

        use FlockOp::*;
        let parsed_op = if op == lock_sh {
            SharedLock { nonblocking: false }
        } else if op == lock_sh | lock_nb {
            SharedLock { nonblocking: true }
        } else if op == lock_ex {
            ExclusiveLock { nonblocking: false }
        } else if op == lock_ex | lock_nb {
            ExclusiveLock { nonblocking: true }
        } else if op == lock_un {
            Unlock
        } else {
            throw_unsup_format!("unsupported flags {:#x}", op);
        };

        let result = fd.as_unix().flock(this.machine.communicate(), parsed_op)?;
        drop(fd);
        // return `0` if flock is successful
        let result = result.map(|()| 0i32);
        interp_ok(Scalar::from_i32(this.try_unwrap_io_result(result)?))
    }

    fn fcntl(&mut self, args: &[OpTy<'tcx>]) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        let [fd_num, cmd] = check_min_arg_count("fcntl", args)?;

        let fd_num = this.read_scalar(fd_num)?.to_i32()?;
        let cmd = this.read_scalar(cmd)?.to_i32()?;

        let f_getfd = this.eval_libc_i32("F_GETFD");
        let f_dupfd = this.eval_libc_i32("F_DUPFD");
        let f_dupfd_cloexec = this.eval_libc_i32("F_DUPFD_CLOEXEC");

        // We only support getting the flags for a descriptor.
        match cmd {
            cmd if cmd == f_getfd => {
                // Currently this is the only flag that `F_GETFD` returns. It is OK to just return the
                // `FD_CLOEXEC` value without checking if the flag is set for the file because `std`
                // always sets this flag when opening a file. However we still need to check that the
                // file itself is open.
                if !this.machine.fds.is_fd_num(fd_num) {
                    this.set_last_error_and_return_i32(LibcError("EBADF"))
                } else {
                    interp_ok(this.eval_libc("FD_CLOEXEC"))
                }
            }
            cmd if cmd == f_dupfd || cmd == f_dupfd_cloexec => {
                // Note that we always assume the FD_CLOEXEC flag is set for every open file, in part
                // because exec() isn't supported. The F_DUPFD and F_DUPFD_CLOEXEC commands only
                // differ in whether the FD_CLOEXEC flag is pre-set on the new file descriptor,
                // thus they can share the same implementation here.
                let cmd_name = if cmd == f_dupfd {
                    "fcntl(fd, F_DUPFD, ...)"
                } else {
                    "fcntl(fd, F_DUPFD_CLOEXEC, ...)"
                };

                let [_, _, start] = check_min_arg_count(cmd_name, args)?;
                let start = this.read_scalar(start)?.to_i32()?;

                if let Some(fd) = this.machine.fds.get(fd_num) {
                    interp_ok(Scalar::from_i32(this.machine.fds.insert_with_min_num(fd, start)))
                } else {
                    this.set_last_error_and_return_i32(LibcError("EBADF"))
                }
            }
            cmd if this.tcx.sess.target.os == "macos"
                && cmd == this.eval_libc_i32("F_FULLFSYNC") =>
            {
                // Reject if isolation is enabled.
                if let IsolatedOp::Reject(reject_with) = this.machine.isolated_op {
                    this.reject_in_isolation("`fcntl`", reject_with)?;
                    return this.set_last_error_and_return_i32(ErrorKind::PermissionDenied);
                }

                this.ffullsync_fd(fd_num)
            }
            cmd => {
                throw_unsup_format!("fcntl: unsupported command {cmd:#x}");
            }
        }
    }

    fn close(&mut self, fd_op: &OpTy<'tcx>) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        let fd_num = this.read_scalar(fd_op)?.to_i32()?;

        let Some(fd) = this.machine.fds.remove(fd_num) else {
            return this.set_last_error_and_return_i32(LibcError("EBADF"));
        };
        let result = fd.close(this.machine.communicate(), this)?;
        // return `0` if close is successful
        let result = result.map(|()| 0i32);
        interp_ok(Scalar::from_i32(this.try_unwrap_io_result(result)?))
    }

    /// Read data from `fd` into buffer specified by `buf` and `count`.
    ///
    /// If `offset` is `None`, reads data from current cursor position associated with `fd`
    /// and updates cursor position on completion. Otherwise, reads from the specified offset
    /// and keeps the cursor unchanged.
    fn read(
        &mut self,
        fd_num: i32,
        buf: Pointer,
        count: u64,
        offset: Option<i128>,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();

        // Isolation check is done via `FileDescription` trait.

        trace!("Reading from FD {}, size {}", fd_num, count);

        // Check that the *entire* buffer is actually valid memory.
        this.check_ptr_access(buf, Size::from_bytes(count), CheckInAllocMsg::MemoryAccessTest)?;

        // We cap the number of read bytes to the largest value that we are able to fit in both the
        // host's and target's `isize`. This saves us from having to handle overflows later.
        let count = count
            .min(u64::try_from(this.target_isize_max()).unwrap())
            .min(u64::try_from(isize::MAX).unwrap());
        let count = usize::try_from(count).unwrap(); // now it fits in a `usize`
        let communicate = this.machine.communicate();

        // We temporarily dup the FD to be able to retain mutable access to `this`.
        let Some(fd) = this.machine.fds.get(fd_num) else {
            trace!("read: FD not found");
            return this.set_last_error_and_return(LibcError("EBADF"), dest);
        };

        trace!("read: FD mapped to {fd:?}");
        // We want to read at most `count` bytes. We are sure that `count` is not negative
        // because it was a target's `usize`. Also we are sure that its smaller than
        // `usize::MAX` because it is bounded by the host's `isize`.

        match offset {
            None => fd.read(&fd, communicate, buf, count, dest, this)?,
            Some(offset) => {
                let Ok(offset) = u64::try_from(offset) else {
                    return this.set_last_error_and_return(LibcError("EINVAL"), dest);
                };
                fd.as_unix().pread(communicate, offset, buf, count, dest, this)?
            }
        };
        interp_ok(())
    }

    fn readv(
        &mut self,
        fd_num: i32,
        iov_ptr: &OpTy<'tcx>,
        iovcnt: i32,
        offset: Option<i128>,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();

        // Early returns for empty or invalid cases
        if iovcnt == 0 {
            trace!("readv: iovcnt is 0, returning 0 bytes read.");
            return this.write_scalar(Scalar::from_i32(0), dest);
        }

        let Some(fd) = this.machine.fds.get(fd_num) else {
            trace!("readv: FD not found");
            return this.set_last_error_and_return(LibcError("EBADF"), dest);
        };
        trace!("readv: FD mapped to {fd:?}");

        // Convert count only once at the start
        let iovcnt = usize::try_from(iovcnt).expect("iovcnt conversion to usize failed");

        // Get iovec layout information
        let iovec_layout = this.libc_ty_layout("iovec");

        // Create temporary storage for read results
        // We need temporary storage for each individual read operation's result
        // Using an intermediate buffer helps handle error conditions cleanly
        // We use i128 to safely handle both success (positive) and error (-1) cases
        let read_dest = this.allocate(this.machine.layouts.i128, MiriMemoryKind::Machine.into())?;

        // Use usize to match ssize_t semantics while staying platform-independent
        let mut total_bytes_read: usize = 0;

        let mut current_offset = offset;

        // Process each iovec structure
        for i in 0..iovcnt {
            // Access the current iovec structure
            let offset_bytes = iovec_layout
                .size
                .bytes()
                .checked_mul(i as u64)
                .expect("iovec array index calculation overflow");

            let current_iov = this.deref_pointer_and_offset_vectored(
                iov_ptr,
                offset_bytes,
                iovec_layout,
                iovcnt,
                iovec_layout,
            )?;

            // Extract buffer information
            let iov_base = this.project_field_named(&current_iov, "iov_base")?;
            let iov_base_ptr = this.read_pointer(&iov_base)?;

            let iov_len = this.project_field_named(&current_iov, "iov_len")?;
            let iov_len = usize::try_from(this.read_target_usize(&iov_len)?)
                .expect("iovec length exceeds platform size");

            if iov_len == 0 {
                continue;
            }

            // Validate buffer access
            let buffer_size = Size::from_bytes(iov_len);
            this.check_ptr_access(iov_base_ptr, buffer_size, CheckInAllocMsg::MemoryAccessTest)?;

            // Perform the read operation
            let read_result = if let Some(off) = current_offset {
                // Handle pread case
                let Ok(off) = u64::try_from(off) else {
                    return this.set_last_error_and_return(LibcError("EINVAL"), dest);
                };

                fd.as_unix().pread(
                    this.machine.communicate(),
                    off,
                    iov_base_ptr,
                    iov_len,
                    &read_dest,
                    this,
                )?;
                this.read_scalar(&read_dest)?.to_i128()?
            } else {
                // Handle regular read case
                fd.read(&fd, this.machine.communicate(), iov_base_ptr, iov_len, &read_dest, this)?;
                this.read_scalar(&read_dest)?.to_i128()?
            };

            // Handle read result
            if read_result < 0 {
                this.write_int(-1, dest)?;
                return interp_ok(());
            }

            // Update offset for next read if preadv
            if let Some(off) = current_offset.as_mut() {
                // Safe addition with overflow check for offset
                *off = off.checked_add(read_result).expect("file offset calculation overflow");
            }

            let read_result = usize::try_from(read_result).unwrap();

            // Safe addition with overflow check
            total_bytes_read = total_bytes_read
                .checked_add(read_result)
                .expect("total bytes read calculation overflow");

            // Break if we hit EOF (partial read)
            // Convert read_result to unsigned safely for comparison
            if read_result < iov_len {
                break;
            }
        }

        trace!("readv: Total bytes read: {}", total_bytes_read);
        this.write_int(
            u64::try_from(total_bytes_read).expect("total bytes read exceeds u64 capacity"),
            dest,
        )?;

        interp_ok(())
    }

    fn write(
        &mut self,
        fd_num: i32,
        buf: Pointer,
        count: u64,
        offset: Option<i128>,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();

        // Isolation check is done via `FileDescription` trait.

        // Check that the *entire* buffer is actually valid memory.
        this.check_ptr_access(buf, Size::from_bytes(count), CheckInAllocMsg::MemoryAccessTest)?;

        // We cap the number of written bytes to the largest value that we are able to fit in both the
        // host's and target's `isize`. This saves us from having to handle overflows later.
        let count = count
            .min(u64::try_from(this.target_isize_max()).unwrap())
            .min(u64::try_from(isize::MAX).unwrap());
        let count = usize::try_from(count).unwrap(); // now it fits in a `usize`
        let communicate = this.machine.communicate();

        // We temporarily dup the FD to be able to retain mutable access to `this`.
        let Some(fd) = this.machine.fds.get(fd_num) else {
            return this.set_last_error_and_return(LibcError("EBADF"), dest);
        };

        match offset {
            None => fd.write(&fd, communicate, buf, count, dest, this)?,
            Some(offset) => {
                let Ok(offset) = u64::try_from(offset) else {
                    return this.set_last_error_and_return(LibcError("EINVAL"), dest);
                };
                fd.as_unix().pwrite(communicate, buf, count, offset, dest, this)?
            }
        };
        interp_ok(())
    }
}
