//@ignore-target: windows # No pthreads on Windows

use std::mem::MaybeUninit;
use std::ptr;

extern "C" fn thread_start(_arg: *mut libc::c_void) -> *mut libc::c_void {
    ptr::null_mut()
}

fn page_size() -> usize {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE).try_into().unwrap() }
}

/// Spawns a thread with the given attributes, and joins it if that succeeded.
fn spawn(attr: *const libc::pthread_attr_t) -> i32 {
    unsafe {
        let mut thread = MaybeUninit::<libc::pthread_t>::uninit();
        let ret = libc::pthread_create(thread.as_mut_ptr(), attr, thread_start, ptr::null_mut());
        if ret == 0 {
            assert_eq!(libc::pthread_join(thread.assume_init(), ptr::null_mut()), 0);
        }
        ret
    }
}

fn test_init_destroy() {
    unsafe {
        let mut attr = MaybeUninit::<libc::pthread_attr_t>::uninit();
        assert_eq!(libc::pthread_attr_init(attr.as_mut_ptr()), 0);
        assert_eq!(libc::pthread_attr_destroy(attr.as_mut_ptr()), 0);
        // It can be initialized again after being destroyed.
        assert_eq!(libc::pthread_attr_init(attr.as_mut_ptr()), 0);
        assert_eq!(libc::pthread_attr_destroy(attr.as_mut_ptr()), 0);
    }
}

fn test_setstacksize() {
    unsafe {
        let mut attr = MaybeUninit::<libc::pthread_attr_t>::uninit();
        assert_eq!(libc::pthread_attr_init(attr.as_mut_ptr()), 0);

        // Too small.
        assert_eq!(
            libc::pthread_attr_setstacksize(attr.as_mut_ptr(), libc::PTHREAD_STACK_MIN - 1),
            libc::EINVAL
        );
        // Not a multiple of the page size: only macOS rejects this.
        let ret = libc::pthread_attr_setstacksize(attr.as_mut_ptr(), 4 * page_size() + 1);
        if cfg!(target_os = "macos") {
            assert_eq!(ret, libc::EINVAL);
        } else {
            assert_eq!(ret, 0);
        }
        // A normal size.
        assert_eq!(libc::pthread_attr_setstacksize(attr.as_mut_ptr(), 4 * page_size()), 0);

        assert_eq!(libc::pthread_attr_destroy(attr.as_mut_ptr()), 0);
    }
}

fn test_create() {
    unsafe {
        // Default attributes.
        assert_eq!(spawn(ptr::null()), 0);

        let mut attr = MaybeUninit::<libc::pthread_attr_t>::uninit();
        assert_eq!(libc::pthread_attr_init(attr.as_mut_ptr()), 0);
        // Initialized attributes without a stack size.
        assert_eq!(spawn(attr.as_ptr()), 0);
        // A normal stack size.
        assert_eq!(libc::pthread_attr_setstacksize(attr.as_mut_ptr(), 4 * page_size()), 0);
        assert_eq!(spawn(attr.as_ptr()), 0);
        // A stack that is too big to reserve. Keep it page-aligned so macOS accepts it.
        let huge = usize::MAX & !(page_size() - 1);
        assert_eq!(libc::pthread_attr_setstacksize(attr.as_mut_ptr(), huge), 0);
        assert_eq!(spawn(attr.as_ptr()), libc::EAGAIN);
        assert_eq!(libc::pthread_attr_destroy(attr.as_mut_ptr()), 0);
    }
}

fn main() {
    test_init_destroy();
    test_setstacksize();
    test_create();
}
