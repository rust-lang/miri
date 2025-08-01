//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc -Zmiri-disable-stacked-borrows

// This test is a translations of the GenMC test `treiber-stack-dynamic`, but with all code related to GenMC's hazard pointer API removed.
// The test leaks memory, so leak checks are disabled.

#![no_main]
#![allow(static_mut_refs)]

use std::alloc::{Layout, alloc, dealloc};
use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};

use libc::{self, pthread_attr_t, pthread_t};

const MAX_THREADS: usize = 32;

static mut STACK: MyStack = MyStack::new();
static mut THREADS: [pthread_t; MAX_THREADS] = [0; MAX_THREADS];
static mut PARAMS: [u64; MAX_THREADS] = [0; MAX_THREADS];

#[repr(C)]
struct Node {
    value: u64,
    next: AtomicPtr<Node>,
}

struct MyStack {
    top: AtomicPtr<Node>,
}

impl Node {
    pub unsafe fn new_alloc() -> *mut Self {
        alloc(Layout::new::<Self>()) as *mut Self
    }

    pub unsafe fn free(node: *mut Self) {
        dealloc(node as *mut u8, Layout::new::<Self>())
    }
}

impl MyStack {
    pub const fn new() -> Self {
        Self { top: AtomicPtr::new(std::ptr::null_mut()) }
    }

    pub unsafe fn init_stack(&mut self, _num_threads: usize) {
        self.top = AtomicPtr::new(std::ptr::null_mut());
    }

    pub unsafe fn clear_stack(&mut self, _num_threads: usize) {
        let mut next;
        let mut top = *self.top.get_mut();
        while !top.is_null() {
            next = *(*top).next.get_mut();
            Node::free(top);
            top = next;
        }
    }

    pub unsafe fn push(&self, value: u64) {
        let node = Node::new_alloc();
        (*node).value = value;

        loop {
            let top = self.top.load(Ordering::Acquire);
            (*node).next.store(top, Ordering::Relaxed);
            if self.top.compare_exchange(top, node, Ordering::Release, Ordering::Relaxed).is_ok() {
                break;
            }
        }
    }

    pub unsafe fn pop(&self) -> u64 {
        loop {
            let top = self.top.load(Ordering::Acquire);
            if top.is_null() {
                return 0;
            }

            let next = (*top).next.load(Ordering::Relaxed);
            if self.top.compare_exchange(top, next, Ordering::Release, Ordering::Relaxed).is_ok() {
                // NOTE: The popped `Node` is leaked.
                return (*top).value;
            }
        }
    }
}

extern "C" fn thread_w(value: *mut c_void) -> *mut c_void {
    unsafe {
        let pid = *(value as *mut u64);

        STACK.push(pid);
    }
    std::ptr::null_mut()
}

extern "C" fn thread_r(_value: *mut c_void) -> *mut c_void {
    let _idx = unsafe { STACK.pop() };
    std::ptr::null_mut()
}

extern "C" fn thread_rw(value: *mut c_void) -> *mut c_void {
    unsafe {
        let pid = *(value as *mut u64);
        STACK.push(pid);
        let _idx = STACK.pop();
    }
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let attr: *const pthread_attr_t = std::ptr::null();

    // TODO GENMC: make different tests:
    let readers = 1;
    let writers = 2;
    let rdwr = 0;

    let num_threads = readers + writers + rdwr;

    if num_threads > MAX_THREADS {
        std::process::abort();
    }

    let mut i = 0;
    unsafe {
        MyStack::init_stack(&mut STACK, num_threads);

        for j in 0..num_threads {
            PARAMS[j] = j as u64;
        }
        for _ in 0..readers {
            let value: *mut c_void = (&raw mut PARAMS[i]) as *mut c_void;
            if libc::pthread_create(&raw mut THREADS[i], attr, thread_r, value) != 0 {
                std::process::abort();
            }
            i += 1;
        }
        for _ in 0..writers {
            let value: *mut c_void = (&raw mut PARAMS[i]) as *mut c_void;
            if libc::pthread_create(&raw mut THREADS[i], attr, thread_w, value) != 0 {
                std::process::abort();
            }
            i += 1;
        }
        for _ in 0..rdwr {
            let value: *mut c_void = (&raw mut PARAMS[i]) as *mut c_void;
            if libc::pthread_create(&raw mut THREADS[i], attr, thread_rw, value) != 0 {
                std::process::abort();
            }
            i += 1;
        }

        for i in 0..num_threads {
            if libc::pthread_join(THREADS[i], std::ptr::null_mut()) != 0 {
                std::process::abort();
            }
        }

        MyStack::clear_stack(&mut STACK, num_threads);
    }

    0
}
