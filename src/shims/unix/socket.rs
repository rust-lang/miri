use std::cell::{Cell, RefCell};
use std::net::{TcpListener, TcpStream};

use rustc_const_eval::interpret::{InterpResult, interp_ok};
use rustc_middle::throw_unsup_format;
use rustc_target::spec::Os;

use crate::shims::files::{FdId, FileDescription};
use crate::{OpTy, Scalar, *};

#[derive(Debug, PartialEq)]
enum SocketFamily {
    // IPv4 internet protocols
    IPv4,
    // IPv6 internet protocols
    IPv6,
}

#[derive(Debug)]
enum SocketType {
    // Reliable full-duplex communication, based on connections
    Stream,
}

#[allow(unused)]
#[derive(Debug)]
enum SocketKind {
    TcpListener(TcpListener),
    TcpStream(TcpStream),
}

#[allow(unused)]
#[derive(Debug)]
struct Socket {
    /// Family of the socket, used to ensure socket only binds/connects to address of
    /// same family.
    family: SocketFamily,
    /// Type of the socket, either datagram or stream.
    /// Only stream is supported at the moment!
    socket_type: SocketType,
    /// Inner standard library socket used for shimming.
    /// Depending on whether `bind` or `connect` is called, we use a listener or a stream.
    /// This is `None` until either of those methods is called.
    socket: RefCell<Option<SocketKind>>,
    /// Whether this fd is non-blocking or not.
    is_non_block: Cell<bool>,
}

impl FileDescription for Socket {
    fn name(&self) -> &'static str {
        "socket"
    }

    fn destroy<'tcx>(
        self,
        _self_id: FdId,
        _communicate_allowed: bool,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, std::io::Result<()>>
    where
        Self: Sized,
    {
        interp_ok(Ok(()))
    }

    fn get_flags<'tcx>(&self, ecx: &mut MiriInterpCx<'tcx>) -> InterpResult<'tcx, Scalar> {
        let mut flags = ecx.eval_libc_i32("O_RDWR");

        if self.is_non_block.get() {
            flags |= ecx.eval_libc_i32("O_NONBLOCK");
        }

        interp_ok(Scalar::from_i32(flags))
    }

    fn set_flags<'tcx>(
        &self,
        mut flag: i32,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, Scalar> {
        // FIXME: File creation flags should be ignored.
        // TODO: Should it be done here, when it's not done in socketpair
        //       or should it be done at both places?

        let o_nonblock = ecx.eval_libc_i32("O_NONBLOCK");
        let o_rdonly = ecx.eval_libc_i32("O_RDONLY");
        let o_wronly = ecx.eval_libc_i32("O_WRONLY");
        let o_rdwr = ecx.eval_libc_i32("O_RDWR");
        // TODO: What about O_ASYNC? Man page explicitly states socket and pipe support
        //       but socketpair doesn't have it either?

        // O_NONBLOCK flag can be set / unset by user.
        if flag & o_nonblock == o_nonblock {
            self.is_non_block.set(true);
            flag &= !o_nonblock;
        } else {
            self.is_non_block.set(false);
        }

        // Ignore all file access mode flags
        flag &= !(o_rdonly | o_wronly | o_rdwr);

        if flag != 0 {
            throw_unsup_format!("fcntl: only O_NONBLOCK is supported for F_SETFL on sockets");
        }

        interp_ok(Scalar::from_i32(0))
    }
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    /// For more information on the arguments see the socket manpage:
    /// <https://linux.die.net/man/2/socket>
    fn socket(
        &mut self,
        domain: &OpTy<'tcx>,
        type_: &OpTy<'tcx>,
        protocol: &OpTy<'tcx>,
    ) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        let domain = this.read_scalar(domain)?.to_i32()?;
        let mut flags = this.read_scalar(type_)?.to_i32()?;
        let protocol = this.read_scalar(protocol)?.to_i32()?;

        // Reject if isolation is enabled
        if let IsolatedOp::Reject(reject_with) = this.machine.isolated_op {
            this.reject_in_isolation("`socket`", reject_with)?;
            this.set_last_error(LibcError("EACCES"))?;
            return interp_ok(Scalar::from_i32(-1));
        }

        let mut is_sock_nonblock = false;

        // Interpret the flag. Every flag we recognize is "subtracted" from `flags`, so
        // if there is anything left at the end, that's an unsupported flag.
        if matches!(this.tcx.sess.target.os, Os::Linux | Os::Android) {
            // SOCK_NONBLOCK only exists on Linux.
            let sock_nonblock = this.eval_libc_i32("SOCK_NONBLOCK");
            let sock_cloexec = this.eval_libc_i32("SOCK_CLOEXEC");
            if flags & sock_nonblock == sock_nonblock {
                is_sock_nonblock = true;
                flags &= !sock_nonblock;
            }
            if flags & sock_cloexec == sock_cloexec {
                flags &= !sock_cloexec;
            }
        }

        let family = if domain == this.eval_libc_i32("AF_INET") {
            SocketFamily::IPv4
        } else if domain == this.eval_libc_i32("AF_INET6") {
            SocketFamily::IPv6
        } else {
            throw_unsup_format!(
                "socket: domain {:#x} is unsupported, only AF_INET and \
                                AF_INET6 are allowed.",
                domain
            );
        };

        if flags != this.eval_libc_i32("SOCK_STREAM") {
            throw_unsup_format!(
                "socket: type {:#x} is unsupported, only SOCK_STREAM, \
                                SOCK_CLOEXEC and SOCK_NONBLOCK are allowed",
                flags
            );
        } else if protocol != 0 {
            throw_unsup_format!(
                "socket: socket protocol {protocol} is unsupported, \
                                only 0 is allowed"
            );
        }

        let fds = &mut this.machine.fds;
        let fd = fds.new_ref(Socket {
            family,
            is_non_block: Cell::new(is_sock_nonblock),
            socket: RefCell::new(None),
            socket_type: SocketType::Stream,
        });

        interp_ok(Scalar::from_i32(fds.insert(fd)))
    }
}
