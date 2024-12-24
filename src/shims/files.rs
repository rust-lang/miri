use std::any::Any;
use std::collections::BTreeMap;
use std::io::{IsTerminal, Read, SeekFrom, Write};
use std::ops::Deref;
use std::rc::{Rc, Weak};
use std::{fs, io};

use rustc_abi::Size;

use crate::shims::unix::UnixFileDescription;
use crate::*;

/// Represents an open file description.
pub trait FileDescription: std::fmt::Debug + Any {
    fn name(&self) -> &'static str;

    /// Reads as much as possible into the given buffer `ptr`.
    /// `len` indicates how many bytes we should try to read.
    /// `dest` is where the return value should be stored: number of bytes read, or `-1` in case of error.
    #[allow(dead_code)]
    fn read<'tcx>(
        &self,
        _self_ref: &FileDescriptionRef,
        _communicate_allowed: bool,
        _ptr: Pointer,
        _len: usize,
        _dest: &MPlaceTy<'tcx>,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx> {
        throw_unsup_format!("cannot read from {}", self.name());
    }

    /// Performs an atomic read operation on the file.
    ///
    /// # Arguments
    /// * `self_ref` - Strong reference to file description for lifetime management
    /// * `communicate_allowed` - Whether external communication is permitted
    /// * `op` - The I/O operation containing buffer and layout information
    /// * `dest` - Destination for storing operation results
    /// * `ecx` - Mutable reference to interpreter context
    ///
    /// # Returns
    /// * `Ok(())` on successful read
    /// * `Err(_)` if read fails or is unsupported
    fn read_atomic<'tcx>(
        &self,
        _self_ref: &FileDescriptionRef,
        _communicate_allowed: bool,
        _op: &mut IoTransferOperation<'tcx>,
        _dest: &MPlaceTy<'tcx>,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx> {
        throw_unsup_format!("cannot read from {}", self.name());
    }

    /// Writes as much as possible from the given buffer `ptr`.
    /// `len` indicates how many bytes we should try to write.
    /// `dest` is where the return value should be stored: number of bytes written, or `-1` in case of error.
    fn write<'tcx>(
        &self,
        _self_ref: &FileDescriptionRef,
        _communicate_allowed: bool,
        _ptr: Pointer,
        _len: usize,
        _dest: &MPlaceTy<'tcx>,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx> {
        throw_unsup_format!("cannot write to {}", self.name());
    }

    /// Seeks to the given offset (which can be relative to the beginning, end, or current position).
    /// Returns the new position from the start of the stream.
    fn seek<'tcx>(
        &self,
        _communicate_allowed: bool,
        _offset: SeekFrom,
    ) -> InterpResult<'tcx, io::Result<u64>> {
        throw_unsup_format!("cannot seek on {}", self.name());
    }

    fn close<'tcx>(
        self: Box<Self>,
        _communicate_allowed: bool,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<()>> {
        throw_unsup_format!("cannot close {}", self.name());
    }

    fn metadata<'tcx>(&self) -> InterpResult<'tcx, io::Result<fs::Metadata>> {
        throw_unsup_format!("obtaining metadata is only supported on file-backed file descriptors");
    }

    fn is_tty(&self, _communicate_allowed: bool) -> bool {
        // Most FDs are not tty's and the consequence of a wrong `false` are minor,
        // so we use a default impl here.
        false
    }

    fn as_unix(&self) -> &dyn UnixFileDescription {
        panic!("Not a unix file descriptor: {}", self.name());
    }
}

impl dyn FileDescription {
    #[inline(always)]
    pub fn downcast<T: Any>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}

impl FileDescription for io::Stdin {
    fn name(&self) -> &'static str {
        "stdin"
    }

    fn read<'tcx>(
        &self,
        _self_ref: &FileDescriptionRef,
        communicate_allowed: bool,
        ptr: Pointer,
        len: usize,
        dest: &MPlaceTy<'tcx>,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx> {
        let mut bytes = vec![0; len];
        if !communicate_allowed {
            // We want isolation mode to be deterministic, so we have to disallow all reads, even stdin.
            helpers::isolation_abort_error("`read` from stdin")?;
        }
        let result = Read::read(&mut { self }, &mut bytes);
        match result {
            Ok(read_size) => ecx.return_read_success(ptr, &bytes, read_size, dest),
            Err(e) => ecx.set_last_error_and_return(e, dest),
        }
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
        &self,
        _self_ref: &FileDescriptionRef,
        _communicate_allowed: bool,
        ptr: Pointer,
        len: usize,
        dest: &MPlaceTy<'tcx>,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx> {
        let bytes = ecx.read_bytes_ptr_strip_provenance(ptr, Size::from_bytes(len))?;
        // We allow writing to stderr even with isolation enabled.
        let result = Write::write(&mut { self }, bytes);
        // Stdout is buffered, flush to make sure it appears on the
        // screen.  This is the write() syscall of the interpreted
        // program, we want it to correspond to a write() syscall on
        // the host -- there is no good in adding extra buffering
        // here.
        io::stdout().flush().unwrap();
        match result {
            Ok(write_size) => ecx.return_write_success(write_size, dest),
            Err(e) => ecx.set_last_error_and_return(e, dest),
        }
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
        &self,
        _self_ref: &FileDescriptionRef,
        _communicate_allowed: bool,
        ptr: Pointer,
        len: usize,
        dest: &MPlaceTy<'tcx>,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx> {
        let bytes = ecx.read_bytes_ptr_strip_provenance(ptr, Size::from_bytes(len))?;
        // We allow writing to stderr even with isolation enabled.
        // No need to flush, stderr is not buffered.
        let result = Write::write(&mut { self }, bytes);
        match result {
            Ok(write_size) => ecx.return_write_success(write_size, dest),
            Err(e) => ecx.set_last_error_and_return(e, dest),
        }
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
        &self,
        _self_ref: &FileDescriptionRef,
        _communicate_allowed: bool,
        _ptr: Pointer,
        len: usize,
        dest: &MPlaceTy<'tcx>,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx> {
        // We just don't write anything, but report to the user that we did.
        ecx.return_write_success(len, dest)
    }
}

/// Structure contains both the file description and its unique identifier.
#[derive(Clone, Debug)]
pub struct FileDescWithId<T: FileDescription + ?Sized> {
    id: FdId,
    file_description: Box<T>,
}

#[derive(Clone, Debug)]
pub struct FileDescriptionRef(Rc<FileDescWithId<dyn FileDescription>>);

impl Deref for FileDescriptionRef {
    type Target = dyn FileDescription;

    fn deref(&self) -> &Self::Target {
        &*self.0.file_description
    }
}

impl FileDescriptionRef {
    fn new(fd: impl FileDescription, id: FdId) -> Self {
        FileDescriptionRef(Rc::new(FileDescWithId { id, file_description: Box::new(fd) }))
    }

    pub fn close<'tcx>(
        self,
        communicate_allowed: bool,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<()>> {
        // Destroy this `Rc` using `into_inner` so we can call `close` instead of
        // implicitly running the destructor of the file description.
        let id = self.get_id();
        match Rc::into_inner(self.0) {
            Some(fd) => {
                // Remove entry from the global epoll_event_interest table.
                ecx.machine.epoll_interests.remove(id);

                fd.file_description.close(communicate_allowed, ecx)
            }
            None => interp_ok(Ok(())),
        }
    }

    pub fn downgrade(&self) -> WeakFileDescriptionRef {
        WeakFileDescriptionRef { weak_ref: Rc::downgrade(&self.0) }
    }

    pub fn get_id(&self) -> FdId {
        self.0.id
    }
}

/// Holds a weak reference to the actual file description.
#[derive(Clone, Debug, Default)]
pub struct WeakFileDescriptionRef {
    weak_ref: Weak<FileDescWithId<dyn FileDescription>>,
}

impl WeakFileDescriptionRef {
    pub fn upgrade(&self) -> Option<FileDescriptionRef> {
        if let Some(file_desc_with_id) = self.weak_ref.upgrade() {
            return Some(FileDescriptionRef(file_desc_with_id));
        }
        None
    }
}

impl VisitProvenance for WeakFileDescriptionRef {
    fn visit_provenance(&self, _visit: &mut VisitWith<'_>) {
        // A weak reference can never be the only reference to some pointer or place.
        // Since the actual file description is tracked by strong ref somewhere,
        // it is ok to make this a NOP operation.
    }
}

/// A unique id for file descriptions. While we could use the address, considering that
/// is definitely unique, the address would expose interpreter internal state when used
/// for sorting things. So instead we generate a unique id per file description is the name
/// for all `dup`licates and is never reused.
#[derive(Debug, Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct FdId(usize);

/// The file descriptor table
#[derive(Debug)]
pub struct FdTable {
    pub fds: BTreeMap<i32, FileDescriptionRef>,
    /// Unique identifier for file description, used to differentiate between various file description.
    next_file_description_id: FdId,
}

impl VisitProvenance for FdTable {
    fn visit_provenance(&self, _visit: &mut VisitWith<'_>) {
        // All our FileDescription instances do not have any tags.
    }
}

impl FdTable {
    fn new() -> Self {
        FdTable { fds: BTreeMap::new(), next_file_description_id: FdId(0) }
    }
    pub(crate) fn init(mute_stdout_stderr: bool) -> FdTable {
        let mut fds = FdTable::new();
        fds.insert_new(io::stdin());
        if mute_stdout_stderr {
            assert_eq!(fds.insert_new(NullOutput), 1);
            assert_eq!(fds.insert_new(NullOutput), 2);
        } else {
            assert_eq!(fds.insert_new(io::stdout()), 1);
            assert_eq!(fds.insert_new(io::stderr()), 2);
        }
        fds
    }

    pub fn new_ref(&mut self, fd: impl FileDescription) -> FileDescriptionRef {
        let file_handle = FileDescriptionRef::new(fd, self.next_file_description_id);
        self.next_file_description_id = FdId(self.next_file_description_id.0.strict_add(1));
        file_handle
    }

    /// Insert a new file description to the FdTable.
    pub fn insert_new(&mut self, fd: impl FileDescription) -> i32 {
        let fd_ref = self.new_ref(fd);
        self.insert(fd_ref)
    }

    pub fn insert(&mut self, fd_ref: FileDescriptionRef) -> i32 {
        self.insert_with_min_num(fd_ref, 0)
    }

    /// Insert a file description, giving it a file descriptor that is at least `min_fd_num`.
    pub fn insert_with_min_num(&mut self, file_handle: FileDescriptionRef, min_fd_num: i32) -> i32 {
        // Find the lowest unused FD, starting from min_fd. If the first such unused FD is in
        // between used FDs, the find_map combinator will return it. If the first such unused FD
        // is after all other used FDs, the find_map combinator will return None, and we will use
        // the FD following the greatest FD thus far.
        let candidate_new_fd =
            self.fds.range(min_fd_num..).zip(min_fd_num..).find_map(|((fd_num, _fd), counter)| {
                if *fd_num != counter {
                    // There was a gap in the fds stored, return the first unused one
                    // (note that this relies on BTreeMap iterating in key order)
                    Some(counter)
                } else {
                    // This fd is used, keep going
                    None
                }
            });
        let new_fd_num = candidate_new_fd.unwrap_or_else(|| {
            // find_map ran out of BTreeMap entries before finding a free fd, use one plus the
            // maximum fd in the map
            self.fds.last_key_value().map(|(fd_num, _)| fd_num.strict_add(1)).unwrap_or(min_fd_num)
        });

        self.fds.try_insert(new_fd_num, file_handle).unwrap();
        new_fd_num
    }

    pub fn get(&self, fd_num: i32) -> Option<FileDescriptionRef> {
        let fd = self.fds.get(&fd_num)?;
        Some(fd.clone())
    }

    pub fn remove(&mut self, fd_num: i32) -> Option<FileDescriptionRef> {
        self.fds.remove(&fd_num)
    }

    pub fn is_fd_num(&self, fd_num: i32) -> bool {
        self.fds.contains_key(&fd_num)
    }
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    /// Helper to implement `FileDescription::read`:
    /// This is only used when `read` is successful.
    /// `actual_read_size` should be the return value of some underlying `read` call that used
    /// `bytes` as its output buffer.
    /// The length of `bytes` must not exceed either the host's or the target's `isize`.
    /// `bytes` is written to `buf` and the size is written to `dest`.
    fn return_read_success(
        &mut self,
        buf: Pointer,
        bytes: &[u8],
        actual_read_size: usize,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        // If reading to `bytes` did not fail, we write those bytes to the buffer.
        // Crucially, if fewer than `bytes.len()` bytes were read, only write
        // that much into the output buffer!
        this.write_bytes_ptr(buf, bytes[..actual_read_size].iter().copied())?;

        // The actual read size is always less than what got originally requested so this cannot fail.
        this.write_int(u64::try_from(actual_read_size).unwrap(), dest)?;
        interp_ok(())
    }

    /// Helper to implement `FileDescription::write`:
    /// This function is only used when `write` is successful, and writes `actual_write_size` to `dest`
    fn return_write_success(
        &mut self,
        actual_write_size: usize,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        // The actual write size is always less than what got originally requested so this cannot fail.
        this.write_int(u64::try_from(actual_write_size).unwrap(), dest)?;
        interp_ok(())
    }
}

/// Represents an atomic I/O operation that handles data transfer between memory regions.
/// Supports both contiguous and scattered memory layouts for efficient I/O operations.
#[derive(Clone)]
pub struct IoTransferOperation<'tcx> {
    /// Intermediate buffer for atomic transfer operations.
    /// For reads: Temporary storage before distribution to destinations
    /// For writes: Aggregation point before writing to file
    transfer_buffer: Vec<u8>,

    /// Memory layout specification for the transfer operation.
    layout: IoBufferLayout,

    /// Total number of bytes to be processed in this operation.
    total_size: usize,

    /// Interpreter context lifetime marker.
    _phantom: std::marker::PhantomData<&'tcx ()>,
}

/// Specifies how memory regions are organized for I/O operations
#[derive(Clone)]
enum IoBufferLayout {
    /// Single continuous memory region for transfer.
    Contiguous { address: Pointer },
    /// Multiple discontinuous memory regions.
    Scattered { regions: Vec<(Pointer, usize)> },
}

impl VisitProvenance for IoTransferOperation<'_> {
    fn visit_provenance(&self, _visit: &mut VisitWith<'_>) {
        // Visits any references that need provenance tracking.
        // Currently a no-op as IoTransferOperation contains no such references.
    }
}

impl<'tcx> IoTransferOperation<'tcx> {
    /// Creates a new I/O operation for a contiguous memory region.
    pub fn new_contiguous(ptr: Pointer, len: usize) -> Self {
        IoTransferOperation {
            transfer_buffer: vec![0; len],
            layout: IoBufferLayout::Contiguous { address: ptr },
            total_size: len,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Creates a new I/O operation for scattered memory regions.
    pub fn new_scattered(buffers: Vec<(Pointer, usize)>) -> Self {
        let total_size = buffers.iter().map(|(_, len)| len).sum();
        IoTransferOperation {
            transfer_buffer: vec![0; total_size],
            layout: IoBufferLayout::Scattered { regions: buffers },
            total_size,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Provides mutable access to the transfer buffer.
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        &mut self.transfer_buffer
    }

    /// Distributes data from the transfer buffer to final destinations.
    pub fn distribute_data(
        &mut self,
        ecx: &mut MiriInterpCx<'tcx>,
        dest: &MPlaceTy<'tcx>,
        bytes_processed: usize,
    ) -> InterpResult<'tcx> {
        if bytes_processed > self.total_size {
            return ecx.set_last_error_and_return(LibcError("EINVAL"), dest);
        }

        match &self.layout {
            IoBufferLayout::Contiguous { address } => {
                // POSIX Compliance: Verify buffer accessibility before writing
                if ecx
                    .check_ptr_access(
                        *address,
                        Size::from_bytes(bytes_processed),
                        CheckInAllocMsg::MemoryAccessTest,
                    )
                    .report_err()
                    .is_err()
                {
                    return ecx.set_last_error_and_return(LibcError("EFAULT"), dest);
                }

                // Attempt the write operation
                if ecx
                    .write_bytes_ptr(
                        *address,
                        self.transfer_buffer[..bytes_processed].iter().copied(),
                    )
                    .report_err()
                    .is_err()
                {
                    return ecx.set_last_error_and_return(LibcError("EIO"), dest);
                }
            }

            IoBufferLayout::Scattered { regions } => {
                let mut current_pos = 0;

                for (ptr, len) in regions {
                    if current_pos >= bytes_processed {
                        break;
                    }

                    // Calculate copy size with safe arithmetic
                    let remaining_bytes = bytes_processed
                        .checked_sub(current_pos)
                        .expect("current_pos should never exceed bytes_read");
                    let copy_size = (*len).min(remaining_bytes);

                    // POSIX Compliance: Verify each buffer's accessibility
                    if ecx
                        .check_ptr_access(
                            *ptr,
                            Size::from_bytes(copy_size),
                            CheckInAllocMsg::MemoryAccessTest,
                        )
                        .report_err()
                        .is_err()
                    {
                        return ecx.set_last_error_and_return(LibcError("EFAULT"), dest);
                    }

                    let end_pos = current_pos
                        .checked_add(copy_size)
                        .expect("end position calculation should not overflow");

                    // Attempt the write operation with proper error handling
                    if ecx
                        .write_bytes_ptr(
                            *ptr,
                            self.transfer_buffer[current_pos..end_pos].iter().copied(),
                        )
                        .report_err()
                        .is_err()
                    {
                        return ecx.set_last_error_and_return(LibcError("EIO"), dest);
                    }

                    current_pos = end_pos;
                }
            }
        }

        interp_ok(())
    }
}
