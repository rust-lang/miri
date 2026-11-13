//@only-target: windows # Uses win32 api functions
// Make every blocked futex waiter wake up spuriously at the next scheduling point.
//@compile-flags: -Zmiri-spurious-wakeup-rate=1.0

use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_TIMEOUT, FALSE, GetLastError, TRUE};
use windows_sys::Win32::System::Threading::{INFINITE, WaitOnAddress, WakeByAddressSingle};

fn spurious_wait_returns_without_wake() {
    let futex: u32 = 0;
    let compare: u32 = 0;

    // Nobody will ever wake us up, but with a spurious wakeup rate of 1.0 we
    // still return successfully, as if we had been woken up.
    unsafe {
        assert_eq!(
            WaitOnAddress(
                ptr::from_ref(&futex).cast(),
                ptr::from_ref(&compare).cast(),
                size_of::<u32>(),
                INFINITE,
            ),
            TRUE,
        );
    }
}

fn spurious_expired_deadline_times_out() {
    let futex: u32 = 0;
    let compare: u32 = 0;

    // A timeout of 0ms means the deadline has passed by the time we wait.
    // Expired deadlines are handled before spurious wakeups, so we must time out.
    unsafe {
        assert_eq!(
            WaitOnAddress(
                ptr::from_ref(&futex).cast(),
                ptr::from_ref(&compare).cast(),
                size_of::<u32>(),
                0,
            ),
            FALSE,
        );
        assert_eq!(GetLastError(), ERROR_TIMEOUT);
    }
}

fn spurious_wakeup_removes_waiter() {
    let futex: u32 = 0;
    let compare: u32 = 0;

    unsafe {
        assert_eq!(
            WaitOnAddress(
                ptr::from_ref(&futex).cast(),
                ptr::from_ref(&compare).cast(),
                size_of::<u32>(),
                INFINITE,
            ),
            TRUE,
        );
    }

    // `WakeByAddressSingle` does not report how many threads it woke. But if we were still in
    // the waiter queue, waking us up again would make Miri panic, as we are no longer blocked.
    unsafe {
        WakeByAddressSingle(ptr::from_ref(&futex).cast());
    }
}

fn main() {
    spurious_wait_returns_without_wake();
    spurious_expired_deadline_times_out();
    spurious_wakeup_removes_waiter();
}
