use std::ptr::NonNull;
use std::sync::atomic::Ordering;

use ipc_channel::ipc;
use nix::sys::{mman, ptrace, signal};
use nix::unistd;
use rustc_abi::{Align, Size};
use rustc_const_eval::interpret::InterpResult;

use super::CALLBACK_STACK_SIZE;
use super::messages::{Confirmation, StartFfiInfo, TraceRequest};
use super::parent::{ChildListener, sv_loop};
use crate::alloc_addresses::EvalContextExt as _;
use crate::helpers::ToU64;
use crate::shims::alloc::EvalContextExt as _;
use crate::shims::native_lib::trace::parent::{
    DEL_PAGES_ADDR, DEL_PAGES_COUNT, EVT_RX, MACHINE_PTR, NEW_PAGES_ADDR, NEW_PAGES_COUNT,
    PAGE_ADDR, PAGE_COUNT, PAGE_SIZE, RET_IS_LIBC,
};
use crate::shims::native_lib::{EvalContextExtPriv, MemEvents};
use crate::*;

/// A handle to the single, shared supervisor process across all `MiriMachine`s.
/// Since it would be very difficult to trace multiple FFI calls in parallel, we
/// need to ensure that either (a) only one `MiriMachine` is performing an FFI call
/// at any given time, or (b) there are distinct supervisor and child processes for
/// each machine. The former was chosen here.
///
/// This should only contain a `None` if the supervisor has not (yet) been initialised;
/// otherwise, if `init_sv` was called and did not error, this will always be nonempty.
static SUPERVISOR: std::sync::Mutex<Option<Supervisor>> = std::sync::Mutex::new(None);

/// The main means of communication between the child and parent process,
/// allowing the former to send requests and get info from the latter.
pub struct Supervisor {
    /// Sender for FFI-mode-related requests.
    message_tx: ipc::IpcSender<TraceRequest>,
    /// Used for synchronisation, allowing us to receive confirmation that the
    /// parent process has handled the request from `message_tx`.
    confirm_rx: ipc::IpcReceiver<Confirmation>,
    /// Receiver for memory acceses that ocurred during the FFI call.
    event_rx: Option<ipc::IpcReceiver<MemEvents>>,
}

/// Marker representing that an error occurred during creation of the supervisor.
#[derive(Debug)]
pub struct SvInitError;

impl Supervisor {
    /// Returns `true` if the supervisor process exists, and `false` otherwise.
    pub fn is_enabled() -> bool {
        SUPERVISOR.lock().unwrap().is_some()
    }

    pub unsafe fn protect_pages(
        pages: impl Iterator<Item = (NonNull<u8>, usize)>,
        prot: mman::ProtFlags,
    ) -> Result<(), nix::errno::Errno> {
        for (pg, sz) in pages {
            unsafe { mman::mprotect(pg.cast(), sz, prot)? };
        }
        Ok(())
    }
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    /// Performs an arbitrary FFI call, enabling tracing from the supervisor.
    /// As this locks the supervisor via a mutex, no other threads may enter FFI
    /// until this function returns.
    fn do_ffi(
        &mut self,
        f: impl FnOnce() -> InterpResult<'tcx, crate::ImmTy<'tcx>>,
    ) -> InterpResult<'tcx, (crate::ImmTy<'tcx>, Option<MemEvents>)> {
        let this = self.eval_context_mut();
        let machine_ptr = &raw mut *this;
        let mut sv_guard = SUPERVISOR.lock().unwrap();
        // If the supervisor is not initialised for whatever reason, fast-return.
        // As a side-effect, even on platforms where ptracing
        // is not implemented, we enforce that only one FFI call
        // happens at a time.
        let Some(sv) = sv_guard.as_mut() else { return f().map(|v| (v, None)) };

        // Save the machine pointer to a location where the libc interceptors can use it,
        // since we can't pass in arguments.
        super::parent::MACHINE_PTR.store(machine_ptr.cast(), std::sync::atomic::Ordering::Relaxed);
        // Give the libc interceptors the event channel.
        let mut e_rx = super::parent::EVT_RX.lock().unwrap();
        e_rx.replace(sv.event_rx.take().unwrap());
        drop(e_rx);

        // Get pointers to all the pages the supervisor must allow accesses in
        // and prepare the callback stack.
        let alloc = this.machine.allocator.as_ref().unwrap().borrow();
        let page_size = alloc.page_size();
        let page_ptrs = alloc
            .pages()
            .flat_map(|(pg, sz)| {
                // Convert (page, size) pair into list of pages.
                let start = pg.expose_provenance().get();
                (0..sz.strict_div(alloc.page_size()))
                    .map(move |i| start.strict_add(i.strict_mul(page_size)))
            })
            .collect();
        let raw_stack_ptr: *mut [u8; CALLBACK_STACK_SIZE] =
            Box::leak(Box::new([0u8; CALLBACK_STACK_SIZE])).as_mut_ptr().cast();
        let stack_ptr = raw_stack_ptr.expose_provenance();
        let start_info = StartFfiInfo { page_ptrs, stack_ptr };

        let pages: Vec<_> = alloc.pages().collect();
        // If native code allocates, we'll need to get access to the machine's allocator.
        drop(alloc);
        // Unwinding might be messed up due to partly protected memory, so let's abort if something
        // breaks inside here.
        let res = std::panic::abort_unwind(|| {
            // Send over the info.
            // NB: if we do not wait to receive a blank confirmation response, it is
            // possible that the supervisor is alerted of the SIGSTOP *before* it has
            // actually received the start_info, thus deadlocking! This way, we can
            // enforce an ordering for these events.
            sv.message_tx.send(TraceRequest::StartFfi(start_info)).unwrap();
            sv.confirm_rx.recv().unwrap();
            // We need to be stopped for the supervisor to be able to make certain
            // modifications to our memory - simply waiting on the recv() doesn't
            // count.
            signal::raise(signal::SIGSTOP).unwrap();

            // SAFETY: We have coordinated with the supervisor to ensure that this memory will keep
            // working as normal, just with extra tracing. So even if the compiler moves memory
            // accesses down to after the `mprotect`, they won't actually segfault.
            unsafe {
                Supervisor::protect_pages(pages.into_iter(), mman::ProtFlags::PROT_NONE).unwrap();
            }

            let res = f();

            // The original `this` was used during the FFI call, so
            // acquire a new mutable reference from the used pointer.
            let this = unsafe { &mut *machine_ptr };

            let alloc = this.machine.allocator.as_ref().unwrap().borrow();
            // SAFETY: We set memory back to normal, so this is safe.
            unsafe {
                Supervisor::protect_pages(
                    alloc.pages(),
                    mman::ProtFlags::PROT_READ | mman::ProtFlags::PROT_WRITE,
                )
                .unwrap();
            }

            // Signal the supervisor that we are done. Will block until the supervisor continues us.
            // This will also shut down the segfault handler, so it's important that all memory is
            // reset back to normal above. There must not be a window in time where accessing the
            // pages we protected above actually causes the program to abort.
            signal::raise(signal::SIGUSR1).unwrap();

            res
        });

        // SAFETY: We know that this pointer was allocated as a box with this type.
        unsafe {
            drop(Box::from_raw(raw_stack_ptr));
        }

        // Now get the event channel back.
        let mut e_rx = super::parent::EVT_RX.lock().unwrap();
        sv.event_rx = Some(e_rx.take().unwrap());
        drop(e_rx);

        // On the off-chance something really weird happens, don't block forever.
        let events = sv
            .event_rx
            .as_ref()
            .unwrap()
            .try_recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| {
                match e {
                    ipc::TryRecvError::IpcError(_) => (),
                    ipc::TryRecvError::Empty =>
                        panic!("Waiting for accesses from supervisor timed out!"),
                }
            })
            .ok();

        res.map(|v| (v, events))
    }
}

/// Initialises the supervisor process. If this function errors, then the
/// supervisor process could not be created successfully; else, the caller
/// is now the child process and can communicate via `do_ffi`, receiving back
/// events at the end.
///
/// # Safety
/// The invariants for `fork()` must be upheld by the caller, namely either:
/// - Other threads do not exist, or;
/// - If they do exist, either those threads or the resulting child process
///   only ever act in [async-signal-safe](https://www.man7.org/linux/man-pages/man7/signal-safety.7.html) ways.
pub unsafe fn init_sv() -> Result<(), SvInitError> {
    // FIXME: Much of this could be reimplemented via the mitosis crate if we upstream the
    // relevant missing bits.

    // On Linux, this will check whether ptrace is fully disabled by the Yama module.
    // If Yama isn't running or we're not on Linux, we'll still error later, but
    // this saves a very expensive fork call.
    let ptrace_status = std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope");
    if let Ok(stat) = ptrace_status {
        if let Some(stat) = stat.chars().next() {
            // Fast-error if ptrace is fully disabled on the system.
            if stat == '3' {
                return Err(SvInitError);
            }
        }
    }

    // Initialise the supervisor if it isn't already, placing it into SUPERVISOR.
    let mut lock = SUPERVISOR.lock().unwrap();
    if lock.is_some() {
        return Ok(());
    }

    // Prepare the IPC channels we need.
    let (message_tx, message_rx) = ipc::channel().unwrap();
    let (confirm_tx, confirm_rx) = ipc::channel().unwrap();
    let (event_tx, event_rx) = ipc::channel().unwrap();
    // SAFETY: Calling sysconf(_SC_PAGESIZE) is always safe and cannot error.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.try_into().unwrap();
    super::parent::PAGE_SIZE.store(page_size, std::sync::atomic::Ordering::Relaxed);

    unsafe {
        // TODO: Maybe use clone3() instead for better signalling of when the child exits?
        // SAFETY: Caller upholds that only one thread exists.
        match unistd::fork().unwrap() {
            unistd::ForkResult::Parent { child } => {
                // If somehow another thread does exist, prevent it from accessing the lock
                // and thus breaking our safety invariants.
                std::mem::forget(lock);
                // The child process is free to unwind, so we won't to avoid doubly freeing
                // system resources.
                let init = std::panic::catch_unwind(|| {
                    let listener = ChildListener::new(message_rx, confirm_tx.clone());
                    // Trace as many things as possible, to be able to handle them as needed.
                    let options = ptrace::Options::PTRACE_O_TRACESYSGOOD
                        | ptrace::Options::PTRACE_O_TRACECLONE
                        | ptrace::Options::PTRACE_O_TRACEFORK;
                    // Attach to the child process without stopping it.
                    match ptrace::seize(child, options) {
                        // Ptrace works :D
                        Ok(_) => {
                            let code = sv_loop(listener, child, event_tx, confirm_tx).unwrap_err();
                            // If a return code of 0 is not explicitly given, assume something went
                            // wrong and return 1.
                            std::process::exit(code.0.unwrap_or(1))
                        }
                        // Ptrace does not work and we failed to catch that.
                        Err(_) => {
                            // If we can't ptrace, Miri continues being the parent.
                            signal::kill(child, signal::SIGKILL).unwrap();
                            SvInitError
                        }
                    }
                });
                match init {
                    // The "Ok" case means that we couldn't ptrace.
                    Ok(e) => return Err(e),
                    Err(_p) => {
                        eprintln!(
                            "Supervisor process panicked!\n\"
                            Try running again without `-Zmiri-native-lib-enable-tracing`."
                        );
                        std::process::exit(1);
                    }
                }
            }
            unistd::ForkResult::Child => {
                // Make sure we never get orphaned and stuck in SIGSTOP or similar
                // SAFETY: prctl PR_SET_PDEATHSIG is always safe to call.
                let ret = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                assert_eq!(ret, 0);
                // First make sure the parent succeeded with ptracing us!
                signal::raise(signal::SIGSTOP).unwrap();
                // If we're the child process, save the supervisor info.
                let event_rx = Some(event_rx);
                *lock = Some(Supervisor { message_tx, confirm_rx, event_rx });
            }
        }
    }
    Ok(())
}

/// Instruct the supervisor process to return a particular code. Useful if for
/// whatever reason this code fails to be intercepted normally.
pub fn register_retcode_sv(code: i32) {
    let mut sv_guard = SUPERVISOR.lock().unwrap();
    if let Some(sv) = sv_guard.as_mut() {
        sv.message_tx.send(TraceRequest::OverrideRetcode(code)).unwrap();
        sv.confirm_rx.recv().unwrap();
    }
}

// These are functions that should not be called directly, and can only be reached
// by offseting the instruction pointer into them. However, they are here because
// they execute in the child process.

/// Disables protections on the page whose address is currently in `PAGE_ADDR`.
///
/// SAFETY: `PAGE_ADDR` should be set to a page-aligned pointer to an owned page,
/// `PAGE_SIZE` should be the host pagesize, and the range from `PAGE_ADDR` to
/// `PAGE_SIZE` * `PAGE_COUNT` must be owned and allocated memory. No other threads
/// should be running.
pub unsafe extern "C" fn mempr_off() {
    // Again, cannot allow unwinds to happen here.
    let len = PAGE_SIZE.load(Ordering::SeqCst).saturating_mul(PAGE_COUNT.load(Ordering::SeqCst));
    // SAFETY: Upheld by "caller".
    unsafe {
        // It's up to the caller to make sure this doesn't actually overflow, but
        // we mustn't unwind from here, so...
        if libc::mprotect(
            PAGE_ADDR.load(Ordering::SeqCst).cast(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
        ) != 0
        {
            // Can't return or unwind, but we can do this.
            std::process::exit(-1);
        }
    }
    // If this fails somehow we're doomed.
    if signal::raise(signal::SIGSTOP).is_err() {
        std::process::exit(-1);
    }
}

/// Reenables protection on the page set by `PAGE_ADDR`.
///
/// SAFETY: See `mempr_off()`.
pub unsafe extern "C" fn mempr_on() {
    let len = PAGE_SIZE.load(Ordering::SeqCst).wrapping_mul(PAGE_COUNT.load(Ordering::SeqCst));
    // SAFETY: Upheld by "caller".
    unsafe {
        if libc::mprotect(PAGE_ADDR.load(Ordering::SeqCst).cast(), len, libc::PROT_NONE) != 0 {
            std::process::exit(-1);
        }
    }
    if signal::raise(signal::SIGSTOP).is_err() {
        std::process::exit(-1);
    }
}

// Libc interceptors and associated scaffolding. We get dropped into these functions
// with the stack and all properly set up, so these can be simple extern "C" fns.

pub unsafe extern "C" fn fake_malloc(size: libc::size_t) -> *mut libc::c_void {
    do_libc_thing(AllocAction::Allocate(SizeAlign { size, align: None })).unwrap()
}

pub unsafe extern "C" fn fake_calloc(count: libc::size_t, size: libc::size_t) -> *mut libc::c_void {
    unsafe {
        let size = size.strict_mul(count);
        let ret = fake_malloc(size);
        ret.write_bytes(0, size);
        ret
    }
}

pub unsafe extern "C" fn fake_aligned_alloc(
    align: libc::size_t,
    size: libc::size_t,
) -> *mut libc::c_void {
    assert!(align.is_power_of_two() && size.is_multiple_of(align));
    do_libc_thing(AllocAction::Allocate(SizeAlign { size, align: Some(align) })).unwrap()
}

pub unsafe extern "C" fn fake_posix_memalign(
    memptr: *mut *mut libc::c_void,
    align: libc::size_t,
    size: libc::size_t,
) -> libc::c_int {
    if !align.is_power_of_two() {
        return libc::EINVAL;
    }
    let ptr = do_libc_thing(AllocAction::Allocate(SizeAlign { size, align: Some(align) })).unwrap();
    unsafe {
        *memptr = ptr;
    }
    0
}

pub unsafe extern "C" fn fake_realloc(
    ptr: *mut libc::c_void,
    size: libc::size_t,
) -> *mut libc::c_void {
    do_libc_thing(AllocAction::Reallocate(ptr, SizeAlign { size, align: None })).unwrap()
}

pub unsafe extern "C" fn fake_free(ptr: *mut libc::c_void) {
    let r = do_libc_thing(AllocAction::Free(ptr));
    assert!(r.is_none());
}

#[derive(Clone, Copy)]
struct SizeAlign {
    size: usize,
    align: Option<usize>,
}

#[derive(Clone, Copy)]
enum AllocAction {
    Allocate(SizeAlign),
    Free(*mut libc::c_void),
    Reallocate(*mut libc::c_void, SizeAlign),
}

/// Wrapper for `do_libc_thing_inner`, handling the possibility of an error
/// being returned.
fn do_libc_thing(action: AllocAction) -> Option<*mut libc::c_void> {
    // Grab the pointer to the MiriInterpCx.
    let this = unsafe {
        &mut *{
            let p = MACHINE_PTR.load(Ordering::Relaxed).cast::<crate::MiriInterpCx<'_>>();
            assert_ne!(p.addr(), 0);
            p
        }
    };
    let ret = match do_libc_thing_inner(this, action).report_err() {
        Ok(p) => p,
        Err(e) => {
            crate::diagnostics::report_result(this, e);
            std::process::exit(1);
        }
    };
    // Inform the supervisor that we're about to return from the interceptor.
    signal::raise(signal::SIGSTOP).unwrap();
    ret
}

/// Does the actual shimming in a generic way over possible libc functions. For
/// register clobber reasons, we want this to not get inlined or it could cause
/// weird behaviour.
#[inline(never)]
fn do_libc_thing_inner<'tcx>(
    this: &mut MiriInterpCx<'tcx>,
    action: AllocAction,
) -> InterpResult<'tcx, Option<*mut libc::c_void>> {
    // Are we returning into libc?
    let ret_is_libc = RET_IS_LIBC.load(Ordering::Relaxed);

    // Grab the list of pages managed by the allocator and unprotect memory in case
    // it's accessed.
    let old_pages = this.machine.allocator.as_ref().unwrap().borrow().pages().collect::<Vec<_>>();
    // SAFETY: We're unprotecting the pages, so this is ok.
    unsafe {
        super::Supervisor::protect_pages(
            old_pages.iter().copied(),
            mman::ProtFlags::PROT_READ | mman::ProtFlags::PROT_WRITE,
        )
        .unwrap();
    }

    // Apply all events that have happened thus far on the list of allocations
    // that existed before this call.
    let events = EVT_RX.lock().unwrap().as_ref().unwrap().recv().unwrap();
    this.tracing_apply_accesses(events)?;

    // Reset these values for the supervisor to read.
    NEW_PAGES_ADDR.store(std::ptr::null_mut(), Ordering::Relaxed);
    NEW_PAGES_COUNT.store(0, Ordering::Relaxed);
    DEL_PAGES_ADDR.store(std::ptr::null_mut(), Ordering::Relaxed);
    DEL_PAGES_COUNT.store(0, Ordering::Relaxed);

    // Did we end up passing through the call to libc?
    let mut did_libc_call = false;

    let ret = match action {
        AllocAction::Free(ptr) => {
            // Like with realloc, if we're freeing outside of our pages just forward
            // the call to libc itself.
            if ret_is_libc
                || !old_pages.iter().any(|(base, len)| {
                    (base.addr().get()..base.addr().get().strict_add(*len)).contains(&ptr.addr())
                })
            {
                did_libc_call = true;
                unsafe { libc::free(ptr) }
            } else {
                let miri_ptr = crate::Pointer::new(
                    Some(crate::Provenance::Wildcard),
                    rustc_abi::Size::from_bytes(ptr.addr()),
                );
                this.free(miri_ptr)?;
            }
            interp_ok(None)
        }
        AllocAction::Allocate(size) | AllocAction::Reallocate(_, size) => {
            let align = size
                .align
                .unwrap_or(this.malloc_align(size.size.to_u64()).bytes().try_into().unwrap());
            let size = size.size;
            let retptr = match action {
                AllocAction::Free(_) => unreachable!(),
                AllocAction::Allocate(_) => {
                    if ret_is_libc {
                        did_libc_call = true;
                        let mut retptr = std::ptr::null_mut();
                        let r = unsafe { libc::posix_memalign(&raw mut retptr, align, size) };
                        assert_eq!(r, 0);
                        Err(retptr)
                    } else {
                        Ok(this
                            .allocate_ptr(
                                Size::from_bytes(size),
                                Align::from_bytes(align.to_u64()).unwrap(), // fixme
                                MemoryKind::Machine(MiriMemoryKind::C),
                                AllocInit::Uninit,
                            )?
                            .into())
                    }
                }
                AllocAction::Reallocate(ptr, _) => {
                    // There shouldn't be any reallocating functions that take an
                    // align, but not sure.
                    assert_eq!(align.to_u64(), this.malloc_align(size.to_u64()).bytes());
                    // Don't do a `ptr_from_addr_cast`, since it might print an extraneous warning.
                    let miri_ptr = crate::Pointer::new(
                        Some(crate::Provenance::Wildcard),
                        rustc_abi::Size::from_bytes(ptr.addr()),
                    );

                    if ret_is_libc
                        // If we're returning into libc or the original pointer wasn't
                        // in our pages, pass this forward to libc.
                        || !old_pages.iter().any(|(base, len)| {
                            (base.addr().get()..base.addr().get().strict_add(*len)).contains(&ptr.addr())
                        })
                    {
                        did_libc_call = true;
                        Err(unsafe { libc::realloc(ptr, size) })
                    } else {
                        // Otherwise do it with our allocator.
                        Ok(this.realloc(miri_ptr, size.to_u64())?)
                    }
                }
            };
            match retptr {
                Ok(retptr) => {
                    // This pointer is instantiated in foreign code, so it should
                    // be assumed to be exposed from the start.
                    this.expose_provenance(retptr.provenance.unwrap())?;
                    // We want to get the actual *mut T pointer, not just cast the address
                    // from it, to avoid having UB in this handler.
                    let (id, ..) = this.ptr_get_alloc_id(retptr, size.try_into().unwrap())?;
                    let ret = this.get_alloc_raw_mut(id)?.0.get_bytes_unchecked_raw_mut();
                    std::hint::black_box(ret.expose_provenance());
                    interp_ok(Some(ret.cast::<libc::c_void>()))
                }
                Err(retptr) => interp_ok(Some(retptr)),
            }
        }
    };

    // We always need to pass the call to libc proper if we're returning to it,
    // so make sure we didn't make a mistake in our code.
    if ret_is_libc {
        assert!(did_libc_call);
    }

    // Pages to reprotect.
    let prot_pages = if did_libc_call {
        old_pages
    } else {
        // If the call was forwarded, this is unnecessary since it'll do nothing so skip it.
        let new_pages =
            this.machine.allocator.as_ref().unwrap().borrow().pages().collect::<Vec<_>>();

        let mut new_count = 0u32;
        for pgs in &new_pages {
            if !old_pages.contains(pgs) {
                NEW_PAGES_ADDR.store(pgs.0.as_ptr().cast(), Ordering::Relaxed);
                NEW_PAGES_COUNT.store(pgs.1, Ordering::Relaxed);
                new_count = new_count.strict_add(1);
            }
        }
        assert!(new_count <= 1);

        let mut del_count = 0u32;
        for pgs in &old_pages {
            if !new_pages.contains(pgs) {
                DEL_PAGES_ADDR.store(pgs.0.as_ptr().cast(), Ordering::Relaxed);
                DEL_PAGES_COUNT.store(pgs.1, Ordering::Relaxed);
                del_count = del_count.strict_add(1);
            }
        }
        assert!(del_count <= 1);
        new_pages
    };

    // SAFETY: We're going back into the FFI call proper, so we want accesses
    // logged again.
    unsafe {
        super::Supervisor::protect_pages(prot_pages.into_iter(), mman::ProtFlags::PROT_NONE)
            .unwrap()
    };

    ret
    //interp_ok(ret)
}
