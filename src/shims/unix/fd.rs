//! General management of file descriptors, and support for
//! standard file descriptors (stdin/stdout/stderr).

use std::io;
use std::io::ErrorKind;

use rustc_abi::Size;
use rustc_middle::ty::layout::TyAndLayout;

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
                old_new_fd.close_ref(this.machine.communicate(), this)?.ok();
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
        let result = fd.close_ref(this.machine.communicate(), this)?;
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
            None => fd.read(communicate, buf, count, dest, this)?,
            Some(offset) => {
                let Ok(offset) = u64::try_from(offset) else {
                    return this.set_last_error_and_return(LibcError("EINVAL"), dest);
                };
                fd.as_unix().pread(communicate, offset, buf, count, dest, this)?
            }
        };
        interp_ok(())
    }

    /// Reads data from a file descriptor into multiple buffers atomically (vectored I/O).
    ///
    /// This implementation follows POSIX readv() semantics, reading data from the file descriptor
    /// specified by `fd_num` into the buffers described by the array of iovec structures pointed
    /// to by `iov_ptr`. The `iovcnt` argument specifies the number of iovec structures in the array.
    ///
    /// # Arguments
    /// * `fd_num` - The file descriptor to read from
    /// * `iov_ptr` - Pointer to an array of iovec structures
    /// * `iovcnt` - Number of iovec structures in the array
    /// * `dest` - Destination for storing the number of bytes read
    ///
    /// # Returns
    /// * `Ok(())` - Operation completed successfully, with total bytes read stored in `dest`
    /// * `Err(_)` - Operation failed with appropriate errno set
    ///
    /// # Errors
    /// * `EBADF` - `fd_num` is not a valid file descriptor
    /// * `EFAULT` - Part of iovec array or buffers point outside accessible address space
    /// * `EINVAL` - `iovcnt` is negative or exceeds system limit
    /// * `EIO` - I/O error occurred while reading from the file descriptor
    ///
    /// # POSIX Compliance
    /// Implements standard POSIX readv() behavior:
    /// * Performs reads atomically with respect to other threads
    /// * Returns exact number of bytes read or -1 on error
    /// * Handles partial reads and end-of-file conditions
    /// * Respects system-imposed limits on total transfer size
    fn readv(
        &mut self,
        fd_num: i32,
        iov_ptr: &OpTy<'tcx>,
        iovcnt: i32,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();

        // POSIX Compliance: Must handle negative values (EINVAL).
        if iovcnt < 0 {
            return this.set_last_error_and_return(LibcError("EINVAL"), dest);
        }

        // POSIX Compliance: Must handle zero properly.
        if iovcnt == 0 {
            return this.write_scalar(Scalar::from_i32(0), dest);
        }

        // POSIX Compliance: Check if iovcnt exceeds system limits.
        // Most implementations limit this to IOV_MAX
        // Common system default
        // https://github.com/turbolent/w2c2/blob/d94227c22f8d78a04fbad70fa744481ca4a1912e/examples/clang/sys/include/limits.h#L50
        const IOV_MAX: i32 = 1024;
        if iovcnt > IOV_MAX {
            trace!("readv: iovcnt exceeds IOV_MAX");
            return this.set_last_error_and_return(LibcError("EINVAL"), dest);
        }

        // POSIX Compliance: Validate file descriptor.
        let Some(fd) = this.machine.fds.get(fd_num) else {
            return this.set_last_error_and_return(LibcError("EBADF"), dest);
        };

        // Convert iovcnt to usize for array indexing.
        let iovcnt = usize::try_from(iovcnt).expect("iovcnt exceeds platform size");
        let iovec_layout = this.libc_ty_layout("iovec");

        // Gather iovec information.
        // Pre-allocate vectors for iovec information
        let mut iov_info = Vec::with_capacity(iovcnt);
        let mut total_size: u64 = 0;

        // POSIX Compliance: Validate each iovec structure.
        // Must check for EFAULT (invalid buffer addresses) and EINVAL (invalid length).
        for i in 0..iovcnt {
            // Calculate offset to current iovec structure.
            let offset = iovec_layout
                .size
                .bytes()
                .checked_mul(i as u64)
                .expect("iovec array index overflow");

            // Access current iovec structure.
            let current_iov = match this
                .deref_pointer_and_offset_vectored(
                    iov_ptr,
                    offset,
                    iovec_layout,
                    iovcnt,
                    iovec_layout,
                )
                .report_err()
            {
                Ok(iov) => iov,
                Err(_) => {
                    return this.set_last_error_and_return(LibcError("EFAULT"), dest);
                }
            };

            // Extract and validate buffer pointer and length.
            let base_field = this.project_field_named(&current_iov, "iov_base")?;
            let base = this.read_pointer(&base_field)?;

            let len_field = this.project_field_named(&current_iov, "iov_len")?;
            let len = this.read_target_usize(&len_field)?;

            // Validate buffer alignment and accessibility.
            if this
                .check_ptr_access(base, Size::from_bytes(len), CheckInAllocMsg::MemoryAccessTest)
                .report_err()
                .is_err()
            {
                return this.set_last_error_and_return(LibcError("EFAULT"), dest);
            }

            // Update total size safely.
            total_size = match total_size.checked_add(len) {
                Some(new_size) => new_size,
                None => {
                    return this.set_last_error_and_return(LibcError("EINVAL"), dest);
                }
            };

            iov_info.push((base, len));
        }

        // Cap total size to platform limits.
        let total_size =
            total_size.min(u64::try_from(this.target_isize_max()).unwrap()).min(isize::MAX as u64);

        // Early return for zero total size
        if total_size == 0 {
            return this.write_scalar(Scalar::from_i32(0), dest);
        }

        let mut temp_buf: Vec<u8> = vec![0; total_size.try_into().unwrap()];

        // Perform single atomic read operation
        let read_result = fd.read_buffer(this.machine.communicate(), &mut temp_buf[..], dest, this);

        // Handle read result
        if read_result.report_err().is_err() {
            return this.set_last_error_and_return(LibcError("EIO"), dest);
        }

        // Get bytes read from dest and convert to usize for slice operations.
        let read_bytes: usize = this
            .read_target_usize(dest)?
            .try_into()
            .expect("read bytes count exceeds platform usize");

        if read_bytes > 0 {
            // Copy data to individual iovec buffers.
            let mut current_pos = 0usize;

            for (base, len) in iov_info {
                // Early exit if no more bytes to copy.
                if current_pos >= read_bytes {
                    break;
                }

                // Convert len to usize safely and handle potential overflow.
                let buffer_len = usize::try_from(len).expect("buffer length exceeds platform size");

                // Calculate remaining bytes safely.
                let bytes_remaining =
                    read_bytes.checked_sub(current_pos).expect("position calculation underflow");

                // Calculate copy size as minimum of buffer length and remaining bytes.
                let copy_size = buffer_len.min(bytes_remaining);

                // Calculate end position safely.
                let slice_end =
                    current_pos.checked_add(copy_size).expect("slice end calculation overflow");

                // Verify slice bounds.
                if slice_end > temp_buf.len() {
                    return this.set_last_error_and_return(LibcError("EFAULT"), dest);
                }

                // Write the slice to the destination buffer.
                if this
                    .write_bytes_ptr(base, temp_buf[current_pos..slice_end].iter().copied())
                    .report_err()
                    .is_err()
                {
                    return this.set_last_error_and_return(LibcError("EIO"), dest);
                }

                // Update position safely.
                current_pos = slice_end;
            }
        }

        interp_ok(())
    }

    /// Dereferences a pointer to access an element within a source array, with specialized bounds checking
    /// for vectored I/O operations like readv().
    ///
    /// This function provides array-aware bounds checking that is specifically designed for situations
    /// where we need to access multiple independent memory regions, such as when processing an array
    /// of iovec structures. Unlike simple pointer arithmetic bounds checking, this implementation
    /// understands and validates array-based access patterns.
    fn deref_pointer_and_offset_vectored(
        &self,
        op: &impl Projectable<'tcx, Provenance>,
        offset_bytes: u64,
        base_layout: TyAndLayout<'tcx>,
        count: usize,
        value_layout: TyAndLayout<'tcx>,
    ) -> InterpResult<'tcx, MPlaceTy<'tcx>> {
        // 1. Validate the iovec array bounds.
        let array_size = base_layout
            .size
            .bytes()
            .checked_mul(count as u64)
            .ok_or_else(|| err_ub_format!("iovec array size overflow"))?;

        // 2. Check if our offset is within the array.
        if offset_bytes >= array_size {
            throw_ub_format!(
                "{}",
                format!(
                    "iovec array access out of bounds: offset {} in array of size {}",
                    offset_bytes, array_size
                )
            );
        }

        // 3. Ensure the iovec structure we're accessing is fully contained.
        if offset_bytes.checked_add(base_layout.size.bytes()).is_none_or(|end| end > array_size) {
            throw_ub_format!("iovec structure would extend past array bounds");
        }

        // 4. Proceed with the dereferencing.
        let this = self.eval_context_ref();
        let op_place = this.deref_pointer_as(op, base_layout)?;
        let offset = Size::from_bytes(offset_bytes);

        let value_place = op_place.offset(offset, value_layout, this)?;
        interp_ok(value_place)
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
            None => fd.write(communicate, buf, count, dest, this)?,
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
