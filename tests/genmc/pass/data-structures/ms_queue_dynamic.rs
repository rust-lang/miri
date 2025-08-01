//@compile-flags: -Zmiri-ignore-leaks -Zmiri-genmc -Zmiri-disable-stacked-borrows

// This test is a translations of the GenMC test `ms-queue-dynamic`, but with all code related to GenMC's hazard pointer API removed.
// The test leaks memory, so leak checks are disabled.

#![no_main]
#![allow(static_mut_refs)]

use std::alloc::{Layout, alloc, dealloc};
use std::ffi::c_void;
use std::sync::atomic::Ordering::*;
use std::sync::atomic::{AtomicPtr};

use libc::{self, pthread_attr_t, pthread_t};

const MAX_THREADS: usize = 32;

static mut QUEUE: MyStack = MyStack::new();
static mut PARAMS: [u64; MAX_THREADS] = [0; MAX_THREADS];
static mut INPUT: [u64; MAX_THREADS] = [0; MAX_THREADS];
static mut OUTPUT: [Option<u64>; MAX_THREADS] = [None; MAX_THREADS];

#[repr(C)]
struct Node {
    value: u64,
    next: AtomicPtr<Node>,
}

struct MyStack {
    head: AtomicPtr<Node>,
    tail: AtomicPtr<Node>,
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
        let head = AtomicPtr::new(std::ptr::null_mut());
        let tail = AtomicPtr::new(std::ptr::null_mut());
        Self { head, tail }
    }

    pub unsafe fn init_queue(&mut self, _num_threads: usize) {
        let dummy = Node::new_alloc();

        (*dummy).next = AtomicPtr::new(std::ptr::null_mut());
        self.head = AtomicPtr::new(dummy);
        self.tail = AtomicPtr::new(dummy);
    }

    pub unsafe fn clear_queue(&mut self, _num_threads: usize) {
        let mut next;
        let mut head = *self.head.get_mut();
        while !head.is_null() {
            next = *(*head).next.get_mut();
            Node::free(head);
            head = next;
        }
    }

    pub unsafe fn enqueue(&self, value: u64) {
        let mut tail;
        let node = Node::new_alloc();
        (*node).value = value;
        (*node).next = AtomicPtr::new(std::ptr::null_mut());

        loop {
            tail = self.tail.load(Acquire);
            let next = (*tail).next.load(Acquire);
            if tail != self.tail.load(Acquire) {
                continue;
            }

            if next.is_null() {
                if (*tail).next.compare_exchange(next, node, Release, Relaxed).is_ok() {
                    break;
                }
            } else {
                let _ = self.tail.compare_exchange(tail, next, Release, Relaxed);
            }
        }

        let _ = self.tail.compare_exchange(tail, node, Release, Relaxed);
    }

    pub unsafe fn dequeue(&self) -> Option<u64> {
        loop {
            let head = self.head.load(Acquire);
            let tail = self.tail.load(Acquire);

            let next_ref = &(*head).next;
            let next = next_ref.load(Acquire);
            if self.head.load(Acquire) != head {
                continue;
            }
            if head == tail {
                if next.is_null() {
                    return None;
                }
                let _ = self.tail.compare_exchange(tail, next, Release, Relaxed);
            } else {
                let ret_val = (*next).value;
                if self.head.compare_exchange(head, next, Release, Relaxed).is_ok() {
                    // NOTE: The popped `Node` is leaked.
                    return Some(ret_val);
                }
            }
        }
    }
}

extern "C" fn thread_w(value: *mut c_void) -> *mut c_void {
    unsafe {
        let pid = *(value as *mut u64);

        INPUT[pid as usize] = pid * 10;
        QUEUE.enqueue(INPUT[pid as usize]);

        std::ptr::null_mut()
    }
}

extern "C" fn thread_r(value: *mut c_void) -> *mut c_void {
    unsafe {
        let pid = *(value as *mut u64);

        OUTPUT[pid as usize] = QUEUE.dequeue();

        std::ptr::null_mut()
    }
}

extern "C" fn thread_rw(value: *mut c_void) -> *mut c_void {
    unsafe {
        let pid = *(value as *mut u64);

        INPUT[pid as usize] = pid * 10;
        QUEUE.enqueue(INPUT[pid as usize]);

        OUTPUT[pid as usize] = QUEUE.dequeue();

        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let mut thread_ids: [pthread_t; MAX_THREADS] = [0; MAX_THREADS];

    let attr: *const pthread_attr_t = std::ptr::null();

    // TODO GENMC (TESTS): make different tests:
    let readers = 0;
    let writers = 0;
    let rdwr = 2;

    let num_threads = readers + writers + rdwr;

    if num_threads > MAX_THREADS {
        std::process::abort();
    }

    let mut i = 0;
    unsafe {
        MyStack::init_queue(&mut QUEUE, num_threads);

        for j in 0..num_threads {
            PARAMS[j] = j as u64;
        }

        /* Spawn threads */
        for _ in 0..writers {
            let value: *mut c_void = (&raw mut PARAMS[i]) as *mut c_void;
            if libc::pthread_create(&raw mut thread_ids[i], attr, thread_w, value) != 0 {
                std::process::abort();
            }
            i += 1;
        }
        for _ in 0..readers {
            let value: *mut c_void = (&raw mut PARAMS[i]) as *mut c_void;
            if libc::pthread_create(&raw mut thread_ids[i], attr, thread_r, value) != 0 {
                std::process::abort();
            }
            i += 1;
        }
        for _ in 0..rdwr {
            let value: *mut c_void = (&raw mut PARAMS[i]) as *mut c_void;
            if libc::pthread_create(&raw mut thread_ids[i], attr, thread_rw, value) != 0 {
                std::process::abort();
            }
            i += 1;
        }

        for i in 0..num_threads {
            if libc::pthread_join(thread_ids[i], std::ptr::null_mut()) != 0 {
                std::process::abort();
            }
        }

        MyStack::clear_queue(&mut QUEUE, num_threads);
    }

    0
}
