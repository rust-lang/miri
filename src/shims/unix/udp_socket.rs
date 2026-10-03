use std::cell::Cell;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::time::Duration;

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
    /// Whether the socket is implicitly or explicitly bound to an address.
    is_bound: Cell<bool>,
    /// Read timeout of the socket. [`None`] means that reads can block indefinitely.
    /// The timeout is applied to the monotonic clock (the Unix specification doesn't
    /// specify which clock to use, but the monotonic clock is more common for
    /// relative timeouts).
    /// This is ignored when the socket is non-blocking.
    read_timeout: Cell<Option<Duration>>,
    /// Write timeout of the socket. [`None`] means that writes can block indefinitely.
    /// The timeout is applied to the monotonic clock (the Unix specification doesn't
    /// specify which clock to use, but the monotonic clock is more common
    /// for relative timeouts).
    /// This is ignored when the socket is non-blocking.
    write_timeout: Cell<Option<Duration>>,
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

        Ok(UdpSocket {
            family,
            socket,
            is_non_block: Cell::new(is_non_block),
            is_bound: Cell::new(false),
            read_timeout: Cell::new(None),
            write_timeout: Cell::new(None),
        })
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

        // The socket has been explicitly bound to a local address.
        self.is_bound.set(true);

        interp_ok(Ok(()))
    }

    fn connect<'tcx>(
        self: FileDescriptionRef<Self>,
        communicate_allowed: bool,
        address: SocketAddr,
        ecx: &mut MiriInterpCx<'tcx>,
        finish: DynMachineCallback<'tcx, Result<(), IoError>>,
    ) -> InterpResult<'tcx> {
        assert!(communicate_allowed, "cannot have `UdpSocket` with isolation enabled!");

        finish.call(ecx, self.socket.connect(address).map_err(IoError::HostError))
    }

    fn setsockopt<'tcx>(
        self: FileDescriptionRef<Self>,
        level: i32,
        option: i32,
        value_ptr: Pointer,
        value_len: u64,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, Result<(), IoError>> {
        if level == ecx.eval_libc_i32("SOL_SOCKET") {
            let opt_so_rcvtimeo = ecx.eval_libc_i32("SO_RCVTIMEO");
            let opt_so_sndtimeo = ecx.eval_libc_i32("SO_SNDTIMEO");

            if matches!(ecx.tcx.sess.target.os, Os::MacOs | Os::FreeBsd | Os::NetBsd) {
                // SO_NOSIGPIPE only exists on MacOS, FreeBSD, and NetBSD.
                let opt_so_nosigpipe = ecx.eval_libc_i32("SO_NOSIGPIPE");

                if option == opt_so_nosigpipe {
                    if value_len != 4 {
                        // Option value should be C-int which is usually 4 bytes.
                        return interp_ok(Err(LibcError("EINVAL")));
                    }
                    let option_value = ecx.ptr_to_mplace(value_ptr, ecx.machine.layouts.i32);
                    let _val = ecx.read_scalar(&option_value)?.to_i32()?;
                    // We entirely ignore this value since we do not support signals anyway.

                    return interp_ok(Ok(()));
                }
            }

            if option == opt_so_rcvtimeo || option == opt_so_sndtimeo {
                let timeval_layout = ecx.libc_ty_layout("timeval");
                let option_value = ecx.ptr_to_mplace(value_ptr, timeval_layout);

                let timeout = match ecx.read_timeval(&option_value)? {
                    None => return interp_ok(Err(LibcError("EINVAL"))),
                    Some(Duration::ZERO) => None,
                    Some(duration) => Some(duration),
                };

                if option == opt_so_rcvtimeo {
                    self.read_timeout.set(timeout);
                } else {
                    self.write_timeout.set(timeout);
                }

                return interp_ok(Ok(()));
            } else {
                throw_unsup_format!(
                    "setsockopt: option {option:#x} is unsupported for level SOL_SOCKET",
                );
            }
        } else if level == ecx.eval_libc_i32("IPPROTO_IP") {
            let opt_ip_ttl = ecx.eval_libc_i32("IP_TTL");

            if option == opt_ip_ttl {
                if value_len != 4 {
                    // Option value should be C-uint which is usually 4 bytes.
                    return interp_ok(Err(LibcError("EINVAL")));
                }
                let option_value = ecx.ptr_to_mplace(value_ptr, ecx.machine.layouts.u32);
                let ttl = ecx.read_scalar(&option_value)?.to_u32()?;

                return match self.socket.set_ttl(ttl) {
                    Ok(_) => interp_ok(Ok(())),
                    Err(e) => interp_ok(Err(IoError::HostError(e))),
                };
            } else {
                throw_unsup_format!(
                    "setsockopt: option {option:#x} is unsupported for level IPPROTO_IP",
                );
            }
        }

        throw_unsup_format!(
            "setsockopt: level {level:#x} is unsupported, only SOL_SOCKET and IPPROTO_IP \
           are allowed"
        );
    }

    fn getsockopt<'tcx>(
        self: FileDescriptionRef<Self>,
        level: i32,
        option: i32,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, Result<MPlaceTy<'tcx>, IoError>> {
        if level == ecx.eval_libc_i32("SOL_SOCKET") {
            let opt_so_rcvtimeo = ecx.eval_libc_i32("SO_RCVTIMEO");
            let opt_so_sndtimeo = ecx.eval_libc_i32("SO_SNDTIMEO");

            if option == opt_so_rcvtimeo || option == opt_so_sndtimeo {
                let timeout = if option == opt_so_rcvtimeo {
                    self.read_timeout.get()
                } else {
                    self.write_timeout.get()
                }
                .unwrap_or_default();

                let secs = timeout.as_secs();
                let usecs = timeout.subsec_micros();

                let timeval_layout = ecx.libc_ty_layout("timeval");
                // Allocate new buffer on the stack with the `timeval` layout.
                let timeval_buffer = ecx.allocate(timeval_layout, MemoryKind::Stack)?;

                let sec_field = ecx.project_field_named(&timeval_buffer, "tv_sec")?;
                ecx.write_int(secs, &sec_field)?;

                let usec_field = ecx.project_field_named(&timeval_buffer, "tv_usec")?;
                ecx.write_int(usecs, &usec_field)?;

                interp_ok(Ok(timeval_buffer))
            } else {
                throw_unsup_format!(
                    "getsockopt: option {option:#x} is unsupported for level SOL_SOCKET",
                );
            }
        } else if level == ecx.eval_libc_i32("IPPROTO_IP") {
            let opt_ip_ttl = ecx.eval_libc_i32("IP_TTL");

            if option == opt_ip_ttl {
                let ttl = match self.socket.ttl() {
                    Ok(ttl) => ttl,
                    Err(e) => return interp_ok(Err(IoError::HostError(e))),
                };

                // Allocate new buffer on the stack with the `u32` layout.
                let value_buffer = ecx.allocate(ecx.machine.layouts.u32, MemoryKind::Stack)?;
                ecx.write_int(ttl, &value_buffer)?;
                interp_ok(Ok(value_buffer))
            } else {
                throw_unsup_format!(
                    "getsockopt: option {option:#x} is unsupported for level IPPROTO_IP",
                );
            }
        } else {
            throw_unsup_format!(
                "getsockopt: level {level:#x} is unsupported, only SOL_SOCKET and IPPROTO_IP \
                are allowed"
            )
        }
    }

    fn getsockname<'tcx>(
        self: FileDescriptionRef<Self>,
        communicate_allowed: bool,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, Result<SocketAddr, IoError>> {
        assert!(communicate_allowed, "cannot have `UdpSocket` with isolation enabled!");

        if !self.is_bound.get() {
            // Since Windows returns EINVAL when invoking `getsockname` on
            // a socket which hasn't been bound yet, we need to manually
            // return an unspecified address here.

            let address = if self.family == socket2::Domain::IPV4 {
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, /* port */ 0))
            } else {
                SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::UNSPECIFIED,
                    /* port */ 0,
                    /* flowinfo */ 0,
                    /* scope_id */ 0,
                ))
            };
            return interp_ok(Ok(address));
        }

        match self.socket.local_addr() {
            Ok(address) => interp_ok(Ok(address)),
            Err(e) => interp_ok(Err(IoError::HostError(e))),
        }
    }
}
