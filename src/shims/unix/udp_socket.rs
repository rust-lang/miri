use std::cell::Cell;
use std::io;
use std::net::SocketAddr;

use rustc_target::spec::Os;

use crate::shims::files::{FileDescription, FileDescriptionRef};
use crate::shims::unix::UnixFileDescription;
use crate::shims::unix::socket::UnixSocketFileDescription;
use crate::*;

#[derive(Debug)]
pub(super) struct UdpSocket {
    /// Family of the socket, used to ensure that the socket only binds/connects to addresses
    /// of the same family.
    family: socket2::Domain,
    /// The underlying host socket.
    socket: mio::net::UdpSocket,
    /// Whether this fd is non-blocking or not.
    is_non_block: Cell<bool>,
}

impl UdpSocket {
    pub fn new(family: socket2::Domain, is_non_block: bool) -> io::Result<Self> {
        let socket =
            socket2::Socket::new(family, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))?;
        // The underlying host socket needs to be non-blocking. The actual
        // blocking mode of the socket is stored in `is_non_block`.
        socket.set_nonblocking(true)?;

        // FIXME: Rustfmt has a bug where it incorrectly removes the outer braces of {{ .. }} inside
        // a `cfg_select!` block. See <https://github.com/rust-lang/rustfmt/issues/7045>.
        #[rustfmt::skip]
        let socket = cfg_select! {
            // Turn the `socket2::Socket` into a `mio::UdpSocket`. We cannot construct the
            // mio socket directly, since its only constructor is `UdpSocket::bind`.
            // SAFETY: mio specifies that it is safe to use `from_raw_fd` and
            // `from_raw_socket` as long as it's ensured that the socket is non-blocking.
            // Because we just made the socket non-blocking, this is uphold.

            unix => {{
                use std::os::fd::{IntoRawFd, FromRawFd};
                let raw_fd = socket.into_raw_fd();
                unsafe { mio::net::UdpSocket::from_raw_fd(raw_fd) }
            }},
            windows => {{
                use std::os::windows::io::{IntoRawSocket, FromRawSocket};
                let raw_socket = socket.into_raw_socket();
                unsafe { mio::net::UdpSocket::from_raw_socket(raw_socket) }
            }},
            _ => unreachable!("unsupported host platform")
        };

        // TODO: Add the underlying host socket to the blocking I/O manager.

        Ok(UdpSocket { family, socket, is_non_block: Cell::new(is_non_block) })
    }

    /// View the underlying host socket as a [`socket2::SockRef`].
    ///
    /// **Note**: Potentially blocking operations need to be performed on the
    /// underlying [`mio::net::UdpSocket`] as it would break the mio poll on
    /// Windows hosts when performed on the [`socket2::SockRef`].
    fn as_socket_ref<'a>(&'a self) -> socket2::SockRef<'a> {
        (&self.socket).into()
    }
}

impl FileDescription for UdpSocket {
    fn name(&self) -> &'static str {
        "udp socket"
    }

    fn as_unix(self: FileDescriptionRef<Self>) -> FileDescriptionRef<dyn UnixFileDescription> {
        self
    }
}

impl UnixFileDescription for UdpSocket {
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
        let o_nonblock = ecx.eval_libc_i32("O_NONBLOCK");

        // O_NONBLOCK flag can be set / unset by user.
        if flag & o_nonblock == o_nonblock {
            self.is_non_block.set(true);
            flag &= !o_nonblock;
        } else {
            self.is_non_block.set(false);
        }

        // Throw error if there is any unsupported flag.
        if flag != 0 {
            throw_unsup_format!("fcntl: only O_NONBLOCK is supported for sockets")
        }

        interp_ok(Scalar::from_i32(0))
    }

    fn as_socket(
        self: FileDescriptionRef<Self>,
    ) -> Option<FileDescriptionRef<dyn UnixSocketFileDescription>> {
        Some(self)
    }
}

impl UnixSocketFileDescription for UdpSocket {
    fn bind<'tcx>(
        self: FileDescriptionRef<Self>,
        communicate_allowed: bool,
        address: SocketAddr,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, Result<(), IoError>> {
        assert!(communicate_allowed, "cannot have `UdpSocket` with isolation enabled!");

        let address_family = match &address {
            SocketAddr::V4(_) => socket2::Domain::IPV4,
            SocketAddr::V6(_) => socket2::Domain::IPV6,
        };

        if self.family != address_family {
            // Attempted to bind an address from a family that doesn't match
            // the family of the socket.
            let err = if matches!(ecx.tcx.sess.target.os, Os::Linux | Os::Android | Os::MacOs) {
                // Linux man page states that `EINVAL` is used when there is an address family mismatch.
                // See <https://man7.org/linux/man-pages/man2/bind.2.html>
                // macOS also returns `EINVAL` but their man page does not specify this.
                LibcError("EINVAL")
            } else {
                // POSIX man page states that `EAFNOSUPPORT` should be used when there is an address
                // family mismatch.
                // See <https://man7.org/linux/man-pages/man3/bind.3p.html>
                LibcError("EAFNOSUPPORT")
            };
            return interp_ok(Err(err));
        }

        // `bind` is a non-blocking operation for UDP sockets.
        if let Err(e) = self.as_socket_ref().bind(&socket2::SockAddr::from(address)) {
            return interp_ok(Err(IoError::HostError(e)));
        }

        interp_ok(Ok(()))
    }
}
