//@ignore-target: windows # No pthreads and prctl on Windows
use std::ffi::CStr;
use std::thread;

fn main() {
    let short_name = "test_named".to_owned();
    let long_name = std::iter::once("test_named_thread_truncation")
        .chain(std::iter::repeat(" yada").take(100))
        .collect::<String>();

    fn set_thread_name(name: &CStr) -> i32 {
        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "illumos", target_os = "solaris"))] {
                unsafe { libc::pthread_setname_np(libc::pthread_self(), name.as_ptr().cast()) }
            } else if #[cfg(target_os = "freebsd")] {
                // pthread_set_name_np does not return anything
                unsafe { libc::pthread_set_name_np(libc::pthread_self(), name.as_ptr().cast()) };
                0
            } else if #[cfg(target_os = "macos")] {
                unsafe { libc::pthread_setname_np(name.as_ptr().cast()) }
            } else if #[cfg(target_os = "android")] {
                // FIXME: Use PR_SET_NAME constant when https://github.com/rust-lang/libc/pull/3941 lands.
                const PR_SET_NAME: i32 = 15;
                unsafe { libc::prctl(PR_SET_NAME, name.as_ptr().cast::<libc::c_char>()) }
            } else {
                compile_error!("set_thread_name not supported for this OS")
            }
        }
    }

    fn get_thread_name(name: &mut [u8]) -> i32 {
        cfg_if::cfg_if! {
            if #[cfg(any(
                target_os = "linux",
                target_os = "illumos",
                target_os = "solaris",
                target_os = "macos"
            ))] {
                unsafe {
                    libc::pthread_getname_np(libc::pthread_self(), name.as_mut_ptr().cast(), name.len())
                }
            } else if #[cfg(target_os = "freebsd")] {
                // pthread_get_name_np does not return anything
                unsafe {
                    libc::pthread_get_name_np(libc::pthread_self(), name.as_mut_ptr().cast(), name.len())
                };
                0
            } else if #[cfg(target_os = "android")] {
                // FIXME: Use PR_GET_NAME constant when https://github.com/rust-lang/libc/pull/3941 lands.
                const PR_GET_NAME: i32 = 16;
                unsafe { libc::prctl(PR_GET_NAME, name.as_mut_ptr().cast::<libc::c_char>()) }
            } else {
                compile_error!("get_thread_name not supported for this OS")
            }
        }
    }

    fn test_using(name: String) {
        let result = thread::Builder::new().name(name.clone()).spawn(move || {
            assert_eq!(thread::current().name(), Some(name.as_str()));

            let mut buf = vec![0u8; name.len() + 1];
            assert_eq!(get_thread_name(&mut buf), 0);
            let cstr = CStr::from_bytes_until_nul(&buf).unwrap();
            if name.len() >= 15 {
                assert!(
                    cstr.to_bytes().len() >= 15,
                    "name is too short: len={}",
                    cstr.to_bytes().len()
                ); // POSIX seems to promise at least 15 chars
                assert!(name.as_bytes().starts_with(cstr.to_bytes()));
            } else {
                assert_eq!(name.as_bytes(), cstr.to_bytes());
            }

            // Also test directly calling pthread_setname to check its return value.
            assert_eq!(set_thread_name(&cstr), 0);
            // But with a too long name it should fail except:
            // * on FreeBSD where the function has no return, hence cannot indicate failure,
            // * on Android where prctl silently truncates the string.
            #[cfg(not(any(target_os = "freebsd", target_os = "android")))]
            assert_ne!(set_thread_name(&std::ffi::CString::new(name).unwrap()), 0);
        });
        result.unwrap().join().unwrap();
    }

    test_using(short_name);
    // Rust remembers the full thread name itself.
    // But the system is limited -- make sure we successfully set a truncation.
    test_using(long_name);
}
