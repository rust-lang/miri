use crate::*;

use crate::shims::unix::fs::FileDescriptor;

use std::io;

/// Socket
///
/// We currently don't allow sending any data through this socket, so this can be just a dummy.
#[derive(Debug)]
pub struct Socket;

impl FileDescriptor for Socket {
    fn name(&self) -> &'static str {
        "socket"
    }

    fn dup(&mut self) -> io::Result<Box<dyn FileDescriptor>> {
        Ok(Box::new(Socket))
    }

    fn close<'tcx>(
        self: Box<Self>,
        _communicate_allowed: bool,
    ) -> InterpResult<'tcx, io::Result<i32>> {
        Ok(Ok(0))
    }
}
