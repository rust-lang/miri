//@ignore-target: windows # No libc fstat on non-file FDs on Windows
//@compile-flags: -Zmiri-disable-isolation

use std::mem::MaybeUninit;

#[path = "../../utils/libc.rs"]
mod libc_utils;
use libc_utils::errno_check;

fn main() {
    test_fstat_socketpair();
    test_fstat_pipe();
    #[cfg(target_os = "linux")]
    test_fstat_eventfd();
    #[cfg(target_os = "linux")]
    test_fstat_epoll();
    test_fstat_stdin();
    test_fstat_stdout();
    test_fstat_stderr();
}

/// Test fstat on socketpair file descriptors.
fn test_fstat_socketpair() {
    let mut fds = [0i32; 2];
    errno_check(unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) });

    // Test fstat on both ends of the socketpair
    for fd in fds.iter() {
        let mut stat = MaybeUninit::<libc::stat>::uninit();
        let res = unsafe { libc::fstat(*fd, stat.as_mut_ptr()) };
        assert_eq!(res, 0, "fstat should succeed on socketpair");
        let stat = unsafe { stat.assume_init_ref() };

        // Check that it's a socket
        assert_eq!(
            stat.st_mode & libc::S_IFMT,
            libc::S_IFSOCK,
            "socketpair should have S_IFSOCK mode"
        );

        // Check that size is 0 (sockets don't have a meaningful size)
        assert_eq!(stat.st_size, 0, "socketpair should have size 0");

        // Check that all fields are initialized (at least accessible)
        let _st_nlink = stat.st_nlink;
        let _st_blksize = stat.st_blksize;
        let _st_blocks = stat.st_blocks;
        let _st_ino = stat.st_ino;
        let _st_dev = stat.st_dev;
        let _st_uid = stat.st_uid;
        let _st_gid = stat.st_gid;
        let _st_rdev = stat.st_rdev;
        let _st_atime = stat.st_atime;
        let _st_mtime = stat.st_mtime;
        let _st_ctime = stat.st_ctime;
        let _st_atime_nsec = stat.st_atime_nsec;
        let _st_mtime_nsec = stat.st_mtime_nsec;
        let _st_ctime_nsec = stat.st_ctime_nsec;
    }

    // Cleanup
    errno_check(unsafe { libc::close(fds[0]) });
    errno_check(unsafe { libc::close(fds[1]) });
}

/// Test fstat on pipe file descriptors.
fn test_fstat_pipe() {
    let mut fds = [0i32; 2];
    errno_check(unsafe { libc::pipe(fds.as_mut_ptr()) });

    // Test fstat on both ends of the pipe
    for fd in fds.iter() {
        let mut stat = MaybeUninit::<libc::stat>::uninit();
        let res = unsafe { libc::fstat(*fd, stat.as_mut_ptr()) };
        assert_eq!(res, 0, "fstat should succeed on pipe");
        let stat = unsafe { stat.assume_init_ref() };

        // Check that it's a FIFO (pipe)
        assert_eq!(stat.st_mode & libc::S_IFMT, libc::S_IFIFO, "pipe should have S_IFIFO mode");

        // Check that size is 0 (pipes don't have a meaningful size)
        assert_eq!(stat.st_size, 0, "pipe should have size 0");

        // Check that all fields are initialized (at least accessible)
        let _st_nlink = stat.st_nlink;
        let _st_blksize = stat.st_blksize;
        let _st_blocks = stat.st_blocks;
        let _st_ino = stat.st_ino;
        let _st_dev = stat.st_dev;
        let _st_uid = stat.st_uid;
        let _st_gid = stat.st_gid;
        let _st_rdev = stat.st_rdev;
        let _st_atime = stat.st_atime;
        let _st_mtime = stat.st_mtime;
        let _st_ctime = stat.st_ctime;
        let _st_atime_nsec = stat.st_atime_nsec;
        let _st_mtime_nsec = stat.st_mtime_nsec;
        let _st_ctime_nsec = stat.st_ctime_nsec;
    }

    // Cleanup
    errno_check(unsafe { libc::close(fds[0]) });
    errno_check(unsafe { libc::close(fds[1]) });
}

/// Test fstat on eventfd file descriptors (Linux only).
#[cfg(target_os = "linux")]
fn test_fstat_eventfd() {
    let flags = libc::EFD_CLOEXEC | libc::EFD_NONBLOCK;
    let fd = libc_utils::errno_result(unsafe { libc::eventfd(0, flags) }).unwrap();

    let mut stat = MaybeUninit::<libc::stat>::uninit();
    let res = unsafe { libc::fstat(fd, stat.as_mut_ptr()) };
    assert_eq!(res, 0, "fstat should succeed on eventfd");
    let stat = unsafe { stat.assume_init_ref() };

    // eventfd is typically reported as a regular file
    // (though the exact type may vary by kernel version)
    // We just check that it's not an error and has size 0
    assert_eq!(stat.st_size, 0, "eventfd should have size 0");

    // Check that all fields are initialized (at least accessible)
    let _st_mode = stat.st_mode;
    let _st_nlink = stat.st_nlink;
    let _st_blksize = stat.st_blksize;
    let _st_blocks = stat.st_blocks;
    let _st_ino = stat.st_ino;
    let _st_dev = stat.st_dev;
    let _st_uid = stat.st_uid;
    let _st_gid = stat.st_gid;
    let _st_rdev = stat.st_rdev;
    let _st_atime = stat.st_atime;
    let _st_mtime = stat.st_mtime;
    let _st_ctime = stat.st_ctime;
    let _st_atime_nsec = stat.st_atime_nsec;
    let _st_mtime_nsec = stat.st_mtime_nsec;
    let _st_ctime_nsec = stat.st_ctime_nsec;

    // Cleanup
    errno_check(unsafe { libc::close(fd) });
}

/// Test fstat on epoll file descriptors (Linux only).
#[cfg(target_os = "linux")]
fn test_fstat_epoll() {
    let fd = libc_utils::errno_result(unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) }).unwrap();

    let mut stat = MaybeUninit::<libc::stat>::uninit();
    let res = unsafe { libc::fstat(fd, stat.as_mut_ptr()) };
    assert_eq!(res, 0, "fstat should succeed on epoll");
    let stat = unsafe { stat.assume_init_ref() };

    // epoll is typically reported as a regular file
    // We just check that it's not an error and has size 0
    assert_eq!(stat.st_size, 0, "epoll should have size 0");

    // Check that all fields are initialized (at least accessible)
    let _st_mode = stat.st_mode;
    let _st_nlink = stat.st_nlink;
    let _st_blksize = stat.st_blksize;
    let _st_blocks = stat.st_blocks;
    let _st_ino = stat.st_ino;
    let _st_dev = stat.st_dev;
    let _st_uid = stat.st_uid;
    let _st_gid = stat.st_gid;
    let _st_rdev = stat.st_rdev;
    let _st_atime = stat.st_atime;
    let _st_mtime = stat.st_mtime;
    let _st_ctime = stat.st_ctime;
    let _st_atime_nsec = stat.st_atime_nsec;
    let _st_mtime_nsec = stat.st_mtime_nsec;
    let _st_ctime_nsec = stat.st_ctime_nsec;

    // Cleanup
    errno_check(unsafe { libc::close(fd) });
}

/// Test fstat on stdin.
fn test_fstat_stdin() {
    let mut stat = MaybeUninit::<libc::stat>::uninit();
    let res = unsafe { libc::fstat(libc::STDIN_FILENO, stat.as_mut_ptr()) };
    assert_eq!(res, 0, "fstat should succeed on stdin");
    let stat = unsafe { stat.assume_init_ref() };

    // stdin is typically a character device (S_IFCHR) or a regular file
    // We just check that it's not an error
    let file_type = stat.st_mode & libc::S_IFMT;
    assert!(
        file_type == libc::S_IFCHR || file_type == libc::S_IFREG,
        "stdin should be S_IFCHR or S_IFREG, got {:#o}",
        file_type
    );

    // Check that all fields are initialized (at least accessible)
    let _st_size = stat.st_size;
    let _st_nlink = stat.st_nlink;
    let _st_blksize = stat.st_blksize;
    let _st_blocks = stat.st_blocks;
    let _st_ino = stat.st_ino;
    let _st_dev = stat.st_dev;
    let _st_uid = stat.st_uid;
    let _st_gid = stat.st_gid;
    let _st_rdev = stat.st_rdev;
    let _st_atime = stat.st_atime;
    let _st_mtime = stat.st_mtime;
    let _st_ctime = stat.st_ctime;
    let _st_atime_nsec = stat.st_atime_nsec;
    let _st_mtime_nsec = stat.st_mtime_nsec;
    let _st_ctime_nsec = stat.st_ctime_nsec;
}

/// Test fstat on stdout.
fn test_fstat_stdout() {
    let mut stat = MaybeUninit::<libc::stat>::uninit();
    let res = unsafe { libc::fstat(libc::STDOUT_FILENO, stat.as_mut_ptr()) };
    assert_eq!(res, 0, "fstat should succeed on stdout");
    let stat = unsafe { stat.assume_init_ref() };

    // stdout is typically a character device (S_IFCHR) or a regular file
    // We just check that it's not an error
    let file_type = stat.st_mode & libc::S_IFMT;
    assert!(
        file_type == libc::S_IFCHR || file_type == libc::S_IFREG,
        "stdout should be S_IFCHR or S_IFREG, got {:#o}",
        file_type
    );

    // Check that all fields are initialized (at least accessible)
    let _st_size = stat.st_size;
    let _st_nlink = stat.st_nlink;
    let _st_blksize = stat.st_blksize;
    let _st_blocks = stat.st_blocks;
    let _st_ino = stat.st_ino;
    let _st_dev = stat.st_dev;
    let _st_uid = stat.st_uid;
    let _st_gid = stat.st_gid;
    let _st_rdev = stat.st_rdev;
    let _st_atime = stat.st_atime;
    let _st_mtime = stat.st_mtime;
    let _st_ctime = stat.st_ctime;
    let _st_atime_nsec = stat.st_atime_nsec;
    let _st_mtime_nsec = stat.st_mtime_nsec;
    let _st_ctime_nsec = stat.st_ctime_nsec;
}

/// Test fstat on stderr.
fn test_fstat_stderr() {
    let mut stat = MaybeUninit::<libc::stat>::uninit();
    let res = unsafe { libc::fstat(libc::STDERR_FILENO, stat.as_mut_ptr()) };
    assert_eq!(res, 0, "fstat should succeed on stderr");
    let stat = unsafe { stat.assume_init_ref() };

    // stderr is typically a character device (S_IFCHR) or a regular file
    // We just check that it's not an error
    let file_type = stat.st_mode & libc::S_IFMT;
    assert!(
        file_type == libc::S_IFCHR || file_type == libc::S_IFREG,
        "stderr should be S_IFCHR or S_IFREG, got {:#o}",
        file_type
    );

    // Check that all fields are initialized (at least accessible)
    let _st_size = stat.st_size;
    let _st_nlink = stat.st_nlink;
    let _st_blksize = stat.st_blksize;
    let _st_blocks = stat.st_blocks;
    let _st_ino = stat.st_ino;
    let _st_dev = stat.st_dev;
    let _st_uid = stat.st_uid;
    let _st_gid = stat.st_gid;
    let _st_rdev = stat.st_rdev;
    let _st_atime = stat.st_atime;
    let _st_mtime = stat.st_mtime;
    let _st_ctime = stat.st_ctime;
    let _st_atime_nsec = stat.st_atime_nsec;
    let _st_mtime_nsec = stat.st_mtime_nsec;
    let _st_ctime_nsec = stat.st_ctime_nsec;
}
