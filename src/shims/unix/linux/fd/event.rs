use crate::shims::unix::fs::FileDescriptor;

use rustc_const_eval::interpret::InterpResult;
use rustc_target::abi::Endian;

use std::cell::Cell;
use std::io;

/// A kind of file descriptor created by `eventfd`.
/// The `Event` type isn't currently written to by `eventfd`.
/// The interface is meant to keep track of objects associated
/// with a file descriptor. For more information see the man
/// page below:
///
/// <https://man.netbsd.org/eventfd.2>
#[derive(Debug)]
pub struct Event {
    /// The object contains an unsigned 64-bit integer (uint64_t) counter that is maintained by the
    /// kernel. This counter is initialized with the value specified in the argument initval.
    pub val: Cell<u64>,
    /// We don't have access to interpcx in the file descriptor method, so we use this for passing
    /// the machine's context.
    pub endianness: Endian,
}

impl FileDescriptor for Event {
    fn name(&self) -> &'static str {
        "event"
    }

    fn dup(&mut self) -> io::Result<Box<dyn FileDescriptor>> {
        Ok(Box::new(Event { val: self.val.clone(), endianness: self.endianness }))
    }

    fn is_tty(&self) -> bool {
        false
    }

    fn close<'tcx>(
        self: Box<Self>,
        _communicate_allowed: bool,
    ) -> InterpResult<'tcx, io::Result<i32>> {
        Ok(Ok(0))
    }

    /// A write call adds the 8-byte integer value supplied in
    /// its buffer to the counter.  The maximum value that may be
    /// stored in the counter is the largest unsigned 64-bit value
    /// minus 1 (i.e., 0xfffffffffffffffe).
    ///
    /// When write is supported in eventfd, if the addition would
    /// cause the counter's value to exceed the maximum, then the
    /// write should either block until a read is performed on the
    /// file descriptor, or fail with the error EAGAIN if the
    /// file descriptor has been made nonblocking.

    /// A write fails with the error EINVAL if the size of the
    /// supplied buffer is less than 8 bytes, or if an attempt is
    /// made to write the value 0xffffffffffffffff.
    fn write<'tcx>(
        &self,
        _communicate_allowed: bool,
        bytes: &[u8],
    ) -> InterpResult<'tcx, io::Result<usize>> {
        let v1 = self.val.get();
        let bytes: [u8; 8] = bytes
            .try_into()
            .map_err(|_| err_unsup_format!("we expected 8 bytes and got {}", bytes.len()))?;
        let step = match self.endianness {
            Endian::Little => u64::from_le_bytes(bytes),
            Endian::Big => u64::from_be_bytes(bytes),
        };

        let v2 = v1.checked_add(step).ok_or_else(|| {
            err_unsup_format!(
                "Miri currently has an incomplete epoll implementation. \
                This operation would overflow if all the numbers written \
                to it would overflow a 64-bit integer. The correct \
                behavior is to block or retry, which is not yet implemented."
            )
        })?;
        self.val.set(v2);
        Ok(Ok(8))
    }
}
