//@ignore-target-windows: No pthreads on Windows
//@revisions: lock trylock unlock init

fn main() {
    check();
}

#[cfg(init)]
fn check() {
    unsafe {
        let mut m: libc::pthread_mutex_t = std::mem::zeroed();
        assert_eq!(libc::pthread_mutex_init(&mut m as *mut _, std::ptr::null()), 0);

        let mut m2 = m;
        libc::pthread_mutex_lock(&mut m2 as *mut _); //~[init] ERROR: pthread_mutex_t can't be moved after first use
    }
}

#[cfg(lock)]
fn check() {
    unsafe {
        let mut m: libc::pthread_mutex_t = libc::PTHREAD_MUTEX_INITIALIZER;
        libc::pthread_mutex_lock(&mut m as *mut _);
        // libc::pthread_mutex_unlock(&mut m as *mut _);

        let mut m2 = m;
        libc::pthread_mutex_lock(&mut m2 as *mut _); //~[lock] ERROR: pthread_mutex_t can't be moved after first use
    }
}

#[cfg(trylock)]
fn check() {
    unsafe {
        let mut m: libc::pthread_mutex_t = libc::PTHREAD_MUTEX_INITIALIZER;
        libc::pthread_mutex_trylock(&mut m as *mut _);
        // libc::pthread_mutex_unlock(&mut m as *mut _);

        let mut m2 = m;
        libc::pthread_mutex_trylock(&mut m2 as *mut _); //~[trylock] ERROR: pthread_mutex_t can't be moved after first use
    }
}

#[cfg(unlock)]
fn check() {
    unsafe {
        let mut m: libc::pthread_mutex_t = libc::PTHREAD_RECURSIVE_MUTEX_INITIALIZER_NP;
        libc::pthread_mutex_unlock(&mut m as *mut _);

        let mut m2 = m;
        libc::pthread_mutex_unlock(&mut m2 as *mut _); //~[unlock] ERROR: pthread_mutex_t can't be moved after first use
    }
}
