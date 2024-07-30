//! General management of file descriptors, and support for
//! standard file descriptors (stdin/stdout/stderr).

use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
use std::collections::BTreeMap;
use std::io::{self, ErrorKind, IsTerminal, Read, SeekFrom, Write};
use std::rc::Rc;

use rustc_target::abi::Size;

use crate::shims::unix::*;
use crate::*;

/// Represents an open file descriptor.
pub trait FileDescription: std::fmt::Debug + Any {
    fn name(&self) -> &'static str;

    /// Reads as much as possible into the given buffer, and returns the number of bytes read.
    fn read<'tcx>(
        &mut self,
        _communicate_allowed: bool,
        _bytes: &mut [u8],
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<usize>> {
        throw_unsup_format!("cannot read from {}", self.name());
    }

    /// Writes as much as possible from the given buffer, and returns the number of bytes written.
    fn write<'tcx>(
        &mut self,
        _communicate_allowed: bool,
        _bytes: &[u8],
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<usize>> {
        throw_unsup_format!("cannot write to {}", self.name());
    }

    /// Reads as much as possible into the given buffer from a given offset,
    /// and returns the number of bytes read.
    fn pread<'tcx>(
        &mut self,
        _communicate_allowed: bool,
        _bytes: &mut [u8],
        _offset: u64,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<usize>> {
        throw_unsup_format!("cannot pread from {}", self.name());
    }

    /// Writes as much as possible from the given buffer starting at a given offset,
    /// and returns the number of bytes written.
    fn pwrite<'tcx>(
        &mut self,
        _communicate_allowed: bool,
        _bytes: &[u8],
        _offset: u64,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<usize>> {
        throw_unsup_format!("cannot pwrite to {}", self.name());
    }

    /// Seeks to the given offset (which can be relative to the beginning, end, or current position).
    /// Returns the new position from the start of the stream.
    fn seek<'tcx>(
        &mut self,
        _communicate_allowed: bool,
        _offset: SeekFrom,
    ) -> InterpResult<'tcx, io::Result<u64>> {
        throw_unsup_format!("cannot seek on {}", self.name());
    }

    fn close<'tcx>(
        self: Box<Self>,
        _communicate_allowed: bool,
    ) -> InterpResult<'tcx, io::Result<()>> {
        throw_unsup_format!("cannot close {}", self.name());
    }

    fn flock<'tcx>(
        &self,
        _communicate_allowed: bool,
        _op: FlockOp,
    ) -> InterpResult<'tcx, io::Result<()>> {
        throw_unsup_format!("cannot flock {}", self.name());
    }

    fn is_tty(&self, _communicate_allowed: bool) -> bool {
        // Most FDs are not tty's and the consequence of a wrong `false` are minor,
        // so we use a default impl here.
        false
    }
}

impl dyn FileDescription {
    #[inline(always)]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }

    #[inline(always)]
    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        (self as &mut dyn Any).downcast_mut()
    }
}

impl FileDescription for io::Stdin {
    fn name(&self) -> &'static str {
        "stdin"
    }

    fn read<'tcx>(
        &mut self,
        communicate_allowed: bool,
        bytes: &mut [u8],
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<usize>> {
        if !communicate_allowed {
            // We want isolation mode to be deterministic, so we have to disallow all reads, even stdin.
            helpers::isolation_abort_error("`read` from stdin")?;
        }
        Ok(Read::read(self, bytes))
    }

    fn is_tty(&self, communicate_allowed: bool) -> bool {
        communicate_allowed && self.is_terminal()
    }
}

impl FileDescription for io::Stdout {
    fn name(&self) -> &'static str {
        "stdout"
    }

    fn write<'tcx>(
        &mut self,
        _communicate_allowed: bool,
        bytes: &[u8],
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<usize>> {
        // We allow writing to stderr even with isolation enabled.
        let result = Write::write(self, bytes);
        // Stdout is buffered, flush to make sure it appears on the
        // screen.  This is the write() syscall of the interpreted
        // program, we want it to correspond to a write() syscall on
        // the host -- there is no good in adding extra buffering
        // here.
        io::stdout().flush().unwrap();

        Ok(result)
    }

    fn is_tty(&self, communicate_allowed: bool) -> bool {
        communicate_allowed && self.is_terminal()
    }
}

impl FileDescription for io::Stderr {
    fn name(&self) -> &'static str {
        "stderr"
    }

    fn write<'tcx>(
        &mut self,
        _communicate_allowed: bool,
        bytes: &[u8],
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<usize>> {
        // We allow writing to stderr even with isolation enabled.
        // No need to flush, stderr is not buffered.
        Ok(Write::write(&mut { self }, bytes))
    }

    fn is_tty(&self, communicate_allowed: bool) -> bool {
        communicate_allowed && self.is_terminal()
    }
}

/// Like /dev/null
#[derive(Debug)]
pub struct NullOutput;

impl FileDescription for NullOutput {
    fn name(&self) -> &'static str {
        "stderr and stdout"
    }

    fn write<'tcx>(
        &mut self,
        _communicate_allowed: bool,
        bytes: &[u8],
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<usize>> {
        // We just don't write anything, but report to the user that we did.
        Ok(Ok(bytes.len()))
    }
}

#[derive(Clone, Debug)]
pub struct FileDescriptionRef(Rc<RefCell<Box<dyn FileDescription>>>);

impl FileDescriptionRef {
    fn new(fd: impl FileDescription) -> Self {
        FileDescriptionRef(Rc::new(RefCell::new(Box::new(fd))))
    }

    pub fn borrow(&self) -> Ref<'_, dyn FileDescription> {
        Ref::map(self.0.borrow(), |fd| fd.as_ref())
    }

    pub fn borrow_mut(&self) -> RefMut<'_, dyn FileDescription> {
        RefMut::map(self.0.borrow_mut(), |fd| fd.as_mut())
    }

    pub fn close<'ctx>(self, communicate_allowed: bool) -> InterpResult<'ctx, io::Result<()>> {
        // Destroy this `Rc` using `into_inner` so we can call `close` instead of
        // implicitly running the destructor of the file description.
        match Rc::into_inner(self.0) {
            Some(fd) => RefCell::into_inner(fd).close(communicate_allowed),
            None => Ok(Ok(())),
        }
    }
}

/// The file descriptor table
#[derive(Debug)]
pub struct FdTable {
    fds: BTreeMap<i32, FileDescriptionRef>,
}

impl VisitProvenance for FdTable {
    fn visit_provenance(&self, _visit: &mut VisitWith<'_>) {
        // All our FileDescriptor do not have any tags.
    }
}

impl FdTable {
    fn new() -> Self {
        FdTable { fds: BTreeMap::new() }
    }
    pub(crate) fn init(mute_stdout_stderr: bool) -> FdTable {
        let mut fds = FdTable::new();
        fds.insert_fd(io::stdin());
        if mute_stdout_stderr {
            assert_eq!(fds.insert_fd(NullOutput), 1);
            assert_eq!(fds.insert_fd(NullOutput), 2);
        } else {
            assert_eq!(fds.insert_fd(io::stdout()), 1);
            assert_eq!(fds.insert_fd(io::stderr()), 2);
        }
        fds
    }

    /// Insert a new file description to the FdTable.
    pub fn insert_fd(&mut self, fd: impl FileDescription) -> i32 {
        let file_handle = FileDescriptionRef::new(fd);
        self.insert_fd_with_min_fd(file_handle, 0)
    }

    /// Insert a new FD that is at least `min_fd`.
    fn insert_fd_with_min_fd(&mut self, file_handle: FileDescriptionRef, min_fd: i32) -> i32 {
        // Find the lowest unused FD, starting from min_fd. If the first such unused FD is in
        // between used FDs, the find_map combinator will return it. If the first such unused FD
        // is after all other used FDs, the find_map combinator will return None, and we will use
        // the FD following the greatest FD thus far.
        let candidate_new_fd =
            self.fds.range(min_fd..).zip(min_fd..).find_map(|((fd, _fh), counter)| {
                if *fd != counter {
                    // There was a gap in the fds stored, return the first unused one
                    // (note that this relies on BTreeMap iterating in key order)
                    Some(counter)
                } else {
                    // This fd is used, keep going
                    None
                }
            });
        let new_fd = candidate_new_fd.unwrap_or_else(|| {
            // find_map ran out of BTreeMap entries before finding a free fd, use one plus the
            // maximum fd in the map
            self.fds.last_key_value().map(|(fd, _)| fd.strict_add(1)).unwrap_or(min_fd)
        });

        self.fds.try_insert(new_fd, file_handle).unwrap();
        new_fd
    }

    pub fn get(&self, fd: i32) -> Option<Ref<'_, dyn FileDescription>> {
        let fd = self.fds.get(&fd)?;
        Some(fd.borrow())
    }

    pub fn get_mut(&self, fd: i32) -> Option<RefMut<'_, dyn FileDescription>> {
        let fd = self.fds.get(&fd)?;
        Some(fd.borrow_mut())
    }

    pub fn dup(&self, fd: i32) -> Option<FileDescriptionRef> {
        let fd = self.fds.get(&fd)?;
        Some(fd.clone())
    }

    pub fn remove(&mut self, fd: i32) -> Option<FileDescriptionRef> {
        self.fds.remove(&fd)
    }

    pub fn is_fd(&self, fd: i32) -> bool {
        self.fds.contains_key(&fd)
    }
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn dup(&mut self, old_fd: i32) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        let Some(dup_fd) = this.machine.fds.dup(old_fd) else {
            return Ok(Scalar::from_i32(this.fd_not_found()?));
        };
        Ok(Scalar::from_i32(this.machine.fds.insert_fd_with_min_fd(dup_fd, 0)))
    }

    fn dup2(&mut self, old_fd: i32, new_fd: i32) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        let Some(dup_fd) = this.machine.fds.dup(old_fd) else {
            return Ok(Scalar::from_i32(this.fd_not_found()?));
        };
        if new_fd != old_fd {
            // Close new_fd if it is previously opened.
            // If old_fd and new_fd point to the same description, then `dup_fd` ensures we keep the underlying file description alive.
            if let Some(file_description) = this.machine.fds.fds.insert(new_fd, dup_fd) {
                // Ignore close error (not interpreter's) according to dup2() doc.
                file_description.close(this.machine.communicate())?.ok();
            }
        }
        Ok(Scalar::from_i32(new_fd))
    }

    fn flock(&mut self, fd: i32, op: i32) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();
        let Some(file_descriptor) = this.machine.fds.get(fd) else {
            return Ok(Scalar::from_i32(this.fd_not_found()?));
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

        let result = file_descriptor.flock(this.machine.communicate(), parsed_op)?;
        drop(file_descriptor);
        // return `0` if flock is successful
        this.try_unwrap_io_result(result)
    }

    fn fcntl(&mut self, args: &[OpTy<'tcx>]) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        if args.len() < 2 {
            throw_ub_format!(
                "incorrect number of arguments for fcntl: got {}, expected at least 2",
                args.len()
            );
        }
        let fd = this.read_scalar(&args[0])?.to_i32()?;
        let cmd = this.read_scalar(&args[1])?.to_i32()?;

        // We only support getting the flags for a descriptor.
        if cmd == this.eval_libc_i32("F_GETFD") {
            // Currently this is the only flag that `F_GETFD` returns. It is OK to just return the
            // `FD_CLOEXEC` value without checking if the flag is set for the file because `std`
            // always sets this flag when opening a file. However we still need to check that the
            // file itself is open.
            Ok(Scalar::from_i32(if this.machine.fds.is_fd(fd) {
                this.eval_libc_i32("FD_CLOEXEC")
            } else {
                this.fd_not_found()?
            }))
        } else if cmd == this.eval_libc_i32("F_DUPFD")
            || cmd == this.eval_libc_i32("F_DUPFD_CLOEXEC")
        {
            // Note that we always assume the FD_CLOEXEC flag is set for every open file, in part
            // because exec() isn't supported. The F_DUPFD and F_DUPFD_CLOEXEC commands only
            // differ in whether the FD_CLOEXEC flag is pre-set on the new file descriptor,
            // thus they can share the same implementation here.
            if args.len() < 3 {
                throw_ub_format!(
                    "incorrect number of arguments for fcntl with cmd=`F_DUPFD`/`F_DUPFD_CLOEXEC`: got {}, expected at least 3",
                    args.len()
                );
            }
            let start = this.read_scalar(&args[2])?.to_i32()?;

            match this.machine.fds.dup(fd) {
                Some(dup_fd) =>
                    Ok(Scalar::from_i32(this.machine.fds.insert_fd_with_min_fd(dup_fd, start))),
                None => Ok(Scalar::from_i32(this.fd_not_found()?)),
            }
        } else if this.tcx.sess.target.os == "macos" && cmd == this.eval_libc_i32("F_FULLFSYNC") {
            // Reject if isolation is enabled.
            if let IsolatedOp::Reject(reject_with) = this.machine.isolated_op {
                this.reject_in_isolation("`fcntl`", reject_with)?;
                this.set_last_error_from_io_error(ErrorKind::PermissionDenied.into())?;
                return Ok(Scalar::from_i32(-1));
            }

            this.ffullsync_fd(fd)
        } else {
            throw_unsup_format!("the {:#x} command is not supported for `fcntl`)", cmd);
        }
    }

    fn close(&mut self, fd_op: &OpTy<'tcx>) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        let fd = this.read_scalar(fd_op)?.to_i32()?;

        let Some(file_description) = this.machine.fds.remove(fd) else {
            return Ok(Scalar::from_i32(this.fd_not_found()?));
        };
        let result = file_description.close(this.machine.communicate())?;
        // return `0` if close is successful
        let result = result.map(|()| 0i32);
        this.try_unwrap_io_result(result)
    }

    /// Function used when a file descriptor does not exist. It returns `Ok(-1)`and sets
    /// the last OS error to `libc::EBADF` (invalid file descriptor). This function uses
    /// `T: From<i32>` instead of `i32` directly because some fs functions return different integer
    /// types (like `read`, that returns an `i64`).
    fn fd_not_found<T: From<i32>>(&mut self) -> InterpResult<'tcx, T> {
        let this = self.eval_context_mut();
        let ebadf = this.eval_libc("EBADF");
        this.set_last_error(ebadf)?;
        Ok((-1).into())
    }

    /// Read data from `fd` into buffer specified by `buf` and `count`.
    ///
    /// If `offset` is `None`, reads data from current cursor position associated with `fd`
    /// and updates cursor position on completion. Otherwise, reads from the specified offset
    /// and keeps the cursor unchanged.
    fn read(
        &mut self,
        fd: i32,
        buf: Pointer,
        count: u64,
        offset: Option<i128>,
    ) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        // Isolation check is done via `FileDescriptor` trait.

        trace!("Reading from FD {}, size {}", fd, count);

        // Check that the *entire* buffer is actually valid memory.
        this.check_ptr_access(buf, Size::from_bytes(count), CheckInAllocMsg::MemoryAccessTest)?;

        // We cap the number of read bytes to the largest value that we are able to fit in both the
        // host's and target's `isize`. This saves us from having to handle overflows later.
        let count = count
            .min(u64::try_from(this.target_isize_max()).unwrap())
            .min(u64::try_from(isize::MAX).unwrap());
        let communicate = this.machine.communicate();

        // We temporarily dup the FD to be able to retain mutable access to `this`.
        let Some(fd) = this.machine.fds.dup(fd) else {
            trace!("read: FD not found");
            return Ok(Scalar::from_target_isize(this.fd_not_found()?, this));
        };

        trace!("read: FD mapped to {fd:?}");
        // We want to read at most `count` bytes. We are sure that `count` is not negative
        // because it was a target's `usize`. Also we are sure that its smaller than
        // `usize::MAX` because it is bounded by the host's `isize`.
        let mut bytes = vec![0; usize::try_from(count).unwrap()];
        let result = match offset {
            None => fd.borrow_mut().read(communicate, &mut bytes, this),
            Some(offset) => {
                let Ok(offset) = u64::try_from(offset) else {
                    let einval = this.eval_libc("EINVAL");
                    this.set_last_error(einval)?;
                    return Ok(Scalar::from_target_isize(-1, this));
                };
                fd.borrow_mut().pread(communicate, &mut bytes, offset, this)
            }
        };
        drop(fd);

        // `File::read` never returns a value larger than `count`, so this cannot fail.
        match result?.map(|c| i64::try_from(c).unwrap()) {
            Ok(read_bytes) => {
                // If reading to `bytes` did not fail, we write those bytes to the buffer.
                // Crucially, if fewer than `bytes.len()` bytes were read, only write
                // that much into the output buffer!
                this.write_bytes_ptr(
                    buf,
                    bytes[..usize::try_from(read_bytes).unwrap()].iter().copied(),
                )?;
                Ok(Scalar::from_target_isize(read_bytes, this))
            }
            Err(e) => {
                this.set_last_error_from_io_error(e)?;
                Ok(Scalar::from_target_isize(-1, this))
            }
        }
    }

    fn write(
        &mut self,
        fd: i32,
        buf: Pointer,
        count: u64,
        offset: Option<i128>,
    ) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        // Isolation check is done via `FileDescriptor` trait.

        // Check that the *entire* buffer is actually valid memory.
        this.check_ptr_access(buf, Size::from_bytes(count), CheckInAllocMsg::MemoryAccessTest)?;

        // We cap the number of written bytes to the largest value that we are able to fit in both the
        // host's and target's `isize`. This saves us from having to handle overflows later.
        let count = count
            .min(u64::try_from(this.target_isize_max()).unwrap())
            .min(u64::try_from(isize::MAX).unwrap());
        let communicate = this.machine.communicate();

        let bytes = this.read_bytes_ptr_strip_provenance(buf, Size::from_bytes(count))?.to_owned();
        // We temporarily dup the FD to be able to retain mutable access to `this`.
        let Some(fd) = this.machine.fds.dup(fd) else {
            return Ok(Scalar::from_target_isize(this.fd_not_found()?, this));
        };

        let result = match offset {
            None => fd.borrow_mut().write(communicate, &bytes, this),
            Some(offset) => {
                let Ok(offset) = u64::try_from(offset) else {
                    let einval = this.eval_libc("EINVAL");
                    this.set_last_error(einval)?;
                    return Ok(Scalar::from_target_isize(-1, this));
                };
                fd.borrow_mut().pwrite(communicate, &bytes, offset, this)
            }
        };
        drop(fd);

        let result = result?.map(|c| i64::try_from(c).unwrap());
        match result {
            Ok(written_bytes) => Ok(Scalar::from_target_isize(written_bytes, this)),
            Err(e) => {
                this.set_last_error_from_io_error(e)?;
                Ok(Scalar::from_target_isize(-1, this))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum FlockOp {
    SharedLock { nonblocking: bool },
    ExclusiveLock { nonblocking: bool },
    Unlock,
}
