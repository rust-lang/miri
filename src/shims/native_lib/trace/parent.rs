use std::sync;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

use ipc_channel::ipc;
use nix::sys::{ptrace, signal, wait};
use nix::unistd;

use super::CALLBACK_STACK_SIZE;
use super::messages::{Confirmation, StartFfiInfo, TraceRequest};
use crate::shims::native_lib::{AccessEvent, AccessRange, MemEvents};

/// The flags to use when calling `waitid()`.
const WAIT_FLAGS: wait::WaitPidFlag =
    wait::WaitPidFlag::WUNTRACED.union(wait::WaitPidFlag::WEXITED);

/// The default word size on a given platform, in bytes.
#[cfg(target_arch = "x86")]
const ARCH_WORD_SIZE: usize = 4;
#[cfg(target_arch = "x86_64")]
const ARCH_WORD_SIZE: usize = 8;

// x86 max instruction length is 15 bytes:
// https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html
// See vol. 3B section 24.25.
const ARCH_MAX_INSTR_SIZE: usize = 15;

/// Opcode for an instruction to raise SIGTRAP, to be written in the child process.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const BREAKPT_INSTR: i16 = 0xCC;

/// The size of the breakpoint-triggering instruction, in bytes.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const BREAKPT_INSTR_SIZE: usize = 1;

/// The host pagesize, initialised to a sentinel zero value.
pub static PAGE_SIZE: AtomicUsize = AtomicUsize::new(0);
/// The address of the page set to be edited, initialised to a sentinel null
/// pointer.
pub(super) static PAGE_ADDR: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
/// How many consecutive pages to unprotect. 1 by default, unlikely to be set
/// higher than 2.
pub(super) static PAGE_COUNT: AtomicUsize = AtomicUsize::new(1);
/// A pointer to the `MiriInterpCx` for use within the libc shims.
pub(super) static MACHINE_PTR: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
/// Is the return address within the libc-mapped area(s)?
pub(super) static RET_IS_LIBC: AtomicBool = AtomicBool::new(false);

/// Information about which pages were allocated/deallocated after a single
/// libc intercepted event. After use, these are reset to 0.
///
/// INVARIANT: A single libc event can only allocate/deallocate one contiguous
/// block of pages (as would be the case in a large `realloc`).
pub(super) static NEW_PAGES_ADDR: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
pub(super) static NEW_PAGES_COUNT: AtomicUsize = AtomicUsize::new(0);
pub(super) static DEL_PAGES_ADDR: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
pub(super) static DEL_PAGES_COUNT: AtomicUsize = AtomicUsize::new(0);

/// The `event_rx` channel from the supervisor struct. The libc interceptors must
/// know which accesses happened before they were triggered, so e.g. an access in
/// an allocation that was later freed before the FFI call returned doesn't mistakenly
/// get marked as incorrect.
pub(super) static EVT_RX: sync::Mutex<Option<ipc::IpcReceiver<MemEvents>>> = sync::Mutex::new(None);

/// Allows us to get common arguments from the `user_regs_t` across architectures.
/// Normally this would land us ABI hell, but thankfully all of our usecases
/// consist of functions with a small number of register-sized integer arguments.
/// See <https://man7.org/linux/man-pages/man2/syscall.2.html> for sources.
trait ArchIndependentRegs {
    /// Gets the address of the instruction pointer.
    fn ip(&self) -> usize;
    /// Gets the address of the stack pointer.
    fn sp(&self) -> usize;
    /// Set the instruction pointer; remember to also set the stack pointer, or
    /// else the stack might get messed up!
    fn set_ip(&mut self, ip: usize);
    /// Set the stack pointer, ideally to a zeroed-out area.
    fn set_sp(&mut self, sp: usize);
}

// It's fine / desirable behaviour for values to wrap here, we care about just
// preserving the bit pattern.
#[cfg(target_arch = "x86_64")]
#[rustfmt::skip]
impl ArchIndependentRegs for libc::user_regs_struct {
    #[inline]
    fn ip(&self) -> usize { self.rip.try_into().unwrap() }
    #[inline]
    fn sp(&self) -> usize { self.rsp.try_into().unwrap() }
    #[inline]
    fn set_ip(&mut self, ip: usize) { self.rip = ip.try_into().unwrap() }
    #[inline]
    fn set_sp(&mut self, sp: usize) { self.rsp = sp.try_into().unwrap() }
}

#[cfg(target_arch = "x86")]
#[rustfmt::skip]
impl ArchIndependentRegs for libc::user_regs_struct {
    #[inline]
    fn ip(&self) -> usize { self.eip.cast_unsigned().try_into().unwrap() }
    #[inline]
    fn sp(&self) -> usize { self.esp.cast_unsigned().try_into().unwrap() }
    #[inline]
    fn set_ip(&mut self, ip: usize) { self.eip = ip.cast_signed().try_into().unwrap() }
    #[inline]
    fn set_sp(&mut self, sp: usize) { self.esp = sp.cast_signed().try_into().unwrap() }
}

/// A unified event representing something happening on the child process. Wraps
/// `nix`'s `WaitStatus` and our custom signals so it can all be done with one
/// `match` statement.
pub enum ExecEvent {
    /// Child process requests that we begin monitoring it.
    Start(StartFfiInfo),
    /// Child requests that we stop monitoring and pass over the events we
    /// detected.
    End,
    /// The child process with the specified pid was stopped by the given signal.
    Status(unistd::Pid, signal::Signal),
    /// The child process with the specified pid entered or existed a syscall.
    Syscall(unistd::Pid),
    /// The child exited or was killed; if we have a return code, it is
    /// specified.
    Died(Option<i32>),
}

/// A listener for the FFI start info channel along with relevant state.
pub struct ChildListener {
    /// The matching channel for the child's `Supervisor` struct.
    message_rx: ipc::IpcReceiver<TraceRequest>,
    /// ...
    confirm_tx: ipc::IpcSender<Confirmation>,
    /// Whether an FFI call is currently ongoing.
    attached: bool,
    /// If `Some`, overrides the return code with the given value.
    override_retcode: Option<i32>,
    /// Last code obtained from a child exiting.
    last_code: Option<i32>,
}

impl ChildListener {
    pub fn new(
        message_rx: ipc::IpcReceiver<TraceRequest>,
        confirm_tx: ipc::IpcSender<Confirmation>,
    ) -> Self {
        Self { message_rx, confirm_tx, attached: false, override_retcode: None, last_code: None }
    }
}

impl Iterator for ChildListener {
    type Item = ExecEvent;

    // Allows us to monitor the child process by just iterating over the listener.
    // NB: This should never return None!
    fn next(&mut self) -> Option<Self::Item> {
        // Do not block if the child has nothing to report for `waitid`.
        let opts = WAIT_FLAGS | wait::WaitPidFlag::WNOHANG;
        loop {
            // Listen to any child, not just the main one. Important if we want
            // to allow the C code to fork further, along with being a bit of
            // defensive programming since Linux sometimes assigns threads of
            // the same process different PIDs with unpredictable rules...
            match wait::waitid(wait::Id::All, opts) {
                Ok(stat) =>
                    match stat {
                        // Child exited normally with a specific code set.
                        wait::WaitStatus::Exited(_, code) => self.last_code = Some(code),
                        // Child was killed by a signal, without giving a code.
                        wait::WaitStatus::Signaled(_, _, _) => self.last_code = None,
                        // Child entered or exited a syscall.
                        wait::WaitStatus::PtraceSyscall(pid) =>
                            if self.attached {
                                return Some(ExecEvent::Syscall(pid));
                            },
                        // Child with the given pid was stopped by the given signal.
                        // It's somewhat unclear when which of these two is returned;
                        // we just treat them the same.
                        wait::WaitStatus::Stopped(pid, signal)
                        | wait::WaitStatus::PtraceEvent(pid, signal, _) =>
                            if self.attached {
                                // This is our end-of-FFI signal!
                                if signal == signal::SIGUSR1 {
                                    self.attached = false;
                                    return Some(ExecEvent::End);
                                } else {
                                    return Some(ExecEvent::Status(pid, signal));
                                }
                            } else {
                                // Just pass along the signal.
                                ptrace::cont(pid, signal).unwrap();
                            },
                        _ => (),
                    },
                // This case should only trigger when all children died.
                Err(_) => return Some(ExecEvent::Died(self.override_retcode.or(self.last_code))),
            }

            // Similarly, do a non-blocking poll of the IPC channel.
            if let Ok(req) = self.message_rx.try_recv() {
                match req {
                    TraceRequest::StartFfi(info) =>
                    // Should never trigger - but better to panic explicitly than deadlock!
                        if self.attached {
                            panic!("Attempting to begin FFI multiple times!");
                        } else {
                            self.attached = true;
                            return Some(ExecEvent::Start(info));
                        },
                    TraceRequest::OverrideRetcode(code) => {
                        self.override_retcode = Some(code);
                        self.confirm_tx.send(Confirmation).unwrap();
                    }
                }
            }

            // Not ideal, but doing anything else might sacrifice performance.
            std::thread::yield_now();
        }
    }
}

/// An error came up while waiting on the child process to do something.
/// It likely died, with this return code if we have one.
#[derive(Debug)]
pub struct ExecEnd(pub Option<i32>);

/// Whether to call `ptrace::cont()` immediately. Used exclusively by `wait_for_signal`.
enum InitialCont {
    Yes,
    No,
}

/// This is the main loop of the supervisor process. It runs in a separate
/// process from the rest of Miri (but because we fork, addresses for anything
/// created before the fork - like statics - are the same).
pub fn sv_loop(
    listener: ChildListener,
    init_pid: unistd::Pid,
    event_tx: ipc::IpcSender<MemEvents>,
    confirm_tx: ipc::IpcSender<Confirmation>,
) -> Result<!, ExecEnd> {
    // Get the pagesize set and make sure it isn't still on the zero sentinel value!
    let page_size = PAGE_SIZE.load(Ordering::Relaxed);
    assert_ne!(page_size, 0);

    // Things that we return to the child process.
    let mut acc_events = Vec::new();

    // Memory allocated for the MiriMachine.
    let mut ch_pages = Vec::new();
    let mut ch_stack = None;

    // An instance of the Capstone disassembler, so we don't spawn one on every access.
    let cs = get_disasm();

    // The pid of the last process we interacted with, used by default if we don't have a
    // reason to use a different one.
    let mut curr_pid = init_pid;

    // There's an initial sigstop we need to deal with.
    wait_for_signal(Some(curr_pid), signal::SIGSTOP, InitialCont::No)?;
    ptrace::cont(curr_pid, None).unwrap();

    for evt in listener {
        match evt {
            // Child started ffi, so prep memory.
            ExecEvent::Start(ch_info) => {
                // All the pages that the child process is "allowed to" access.
                ch_pages = ch_info.page_ptrs;
                // And the temporary callback stack it allocated for us to use later.
                ch_stack = Some(ch_info.stack_ptr);

                // We received the signal and are no longer in the main listener loop,
                // so we can let the child move on to the end of the ffi prep where it will
                // raise a SIGSTOP. We need it to be signal-stopped *and waited for* in
                // order to do most ptrace operations!
                confirm_tx.send(Confirmation).unwrap();
                // We can't trust simply calling `Pid::this()` in the child process to give the right
                // PID for us, so we get it this way.
                curr_pid = wait_for_signal(None, signal::SIGSTOP, InitialCont::No).unwrap();
                // Intercept libc events we care about.
                trap_libc(curr_pid);
                // Continue until next syscall.
                ptrace::syscall(curr_pid, None).unwrap();
            }
            // Child wants to end tracing.
            ExecEvent::End => {
                // Stop intercepting libc events.
                fixup_libc(curr_pid);
                // Hand over the access info we traced.
                event_tx.send(MemEvents { acc_events }).unwrap();
                // And reset our values.
                acc_events = Vec::new();
                ch_stack = None;

                // No need to monitor syscalls anymore, they'd just be ignored.
                ptrace::cont(curr_pid, None).unwrap();
            }
            // Child process was stopped by a signal
            ExecEvent::Status(pid, signal) =>
                match signal {
                    // If it was a segfault, check if it was an artificial one
                    // caused by it trying to access the MiriMachine memory.
                    signal::SIGSEGV =>
                        handle_segfault(
                            pid,
                            &ch_pages,
                            ch_stack.unwrap(),
                            page_size,
                            &cs,
                            &mut acc_events,
                        )?,
                    signal::SIGTRAP =>
                        handle_sigtrap(pid, page_size, &mut ch_pages, &event_tx, &mut acc_events)?,
                    // Something weird happened.
                    _ => {
                        eprintln!("Process unexpectedly got {signal}; continuing...");
                        // In case we're not tracing
                        if ptrace::syscall(pid, None).is_err() {
                            // If *this* fails too, something really weird happened
                            // and it's probably best to just panic.
                            signal::kill(pid, signal::SIGCONT).unwrap();
                        }
                    }
                },
            // Child entered or exited a syscall. For now we ignore this and just continue.
            ExecEvent::Syscall(pid) => {
                ptrace::syscall(pid, None).unwrap();
            }
            ExecEvent::Died(code) => return Err(ExecEnd(code)),
        }
    }

    unreachable!()
}

/// Set up SIGTRAPs on the first few bytes of malloc/free/etc.
#[expect(clippy::as_conversions)]
fn trap_libc(pid: unistd::Pid) {
    ptrace::write(pid, libc::malloc as *mut _, BREAKPT_INSTR.into()).unwrap();
    ptrace::write(pid, libc::calloc as *mut _, BREAKPT_INSTR.into()).unwrap();
    ptrace::write(pid, libc::posix_memalign as *mut _, BREAKPT_INSTR.into()).unwrap();
    ptrace::write(pid, libc::aligned_alloc as *mut _, BREAKPT_INSTR.into()).unwrap();
    ptrace::write(pid, libc::realloc as *mut _, BREAKPT_INSTR.into()).unwrap();
    ptrace::write(pid, libc::free as *mut _, BREAKPT_INSTR.into()).unwrap();
}

/// Fix up the libc values.
#[expect(clippy::as_conversions)]
fn fixup_libc(pid: unistd::Pid) {
    unsafe {
        ptrace::write(
            pid,
            libc::malloc as *mut _,
            (libc::malloc as *mut libc::c_long).read_volatile(),
        )
        .unwrap();
        ptrace::write(
            pid,
            libc::calloc as *mut _,
            (libc::calloc as *mut libc::c_long).read_volatile(),
        )
        .unwrap();
        ptrace::write(
            pid,
            libc::posix_memalign as *mut _,
            (libc::posix_memalign as *mut libc::c_long).read_volatile(),
        )
        .unwrap();
        ptrace::write(
            pid,
            libc::aligned_alloc as *mut _,
            (libc::aligned_alloc as *mut libc::c_long).read_volatile(),
        )
        .unwrap();
        ptrace::write(
            pid,
            libc::realloc as *mut _,
            (libc::realloc as *mut libc::c_long).read_volatile(),
        )
        .unwrap();
        ptrace::write(pid, libc::free as *mut _, (libc::free as *mut libc::c_long).read_volatile())
            .unwrap();
    }
}

/// Spawns a Capstone disassembler for the host architecture.
#[rustfmt::skip]
fn get_disasm() -> capstone::Capstone {
    use capstone::prelude::*;
    let cs_pre = Capstone::new();
    {
        #[cfg(target_arch = "x86_64")]
        {cs_pre.x86().mode(arch::x86::ArchMode::Mode64)}
        #[cfg(target_arch = "x86")]
        {cs_pre.x86().mode(arch::x86::ArchMode::Mode32)}
    }
    .detail(true)
    .build()
    .unwrap()
}

/// Waits for `wait_signal`. If `init_cont`, it will first do a `ptrace::cont`.
/// We want to avoid that in some cases, like at the beginning of FFI.
///
/// If `pid` is `None`, only one wait will be done and `init_cont` should be false.
fn wait_for_signal(
    pid: Option<unistd::Pid>,
    wait_signal: signal::Signal,
    init_cont: InitialCont,
) -> Result<unistd::Pid, ExecEnd> {
    if matches!(init_cont, InitialCont::Yes) {
        ptrace::cont(pid.unwrap(), None).unwrap();
    }
    // Repeatedly call `waitid` until we get the signal we want, or the process dies.
    loop {
        let wait_id = match pid {
            Some(pid) => wait::Id::Pid(pid),
            None => wait::Id::All,
        };
        let stat = wait::waitid(wait_id, WAIT_FLAGS).map_err(|_| ExecEnd(None))?;
        let (signal, pid) = match stat {
            // Report the cause of death, if we know it.
            wait::WaitStatus::Exited(_, code) => {
                return Err(ExecEnd(Some(code)));
            }
            wait::WaitStatus::Signaled(_, _, _) => return Err(ExecEnd(None)),
            wait::WaitStatus::Stopped(pid, signal)
            | wait::WaitStatus::PtraceEvent(pid, signal, _) => (signal, pid),
            // This covers PtraceSyscall and variants that are impossible with
            // the flags set (e.g. WaitStatus::StillAlive).
            _ => {
                ptrace::cont(pid.unwrap(), None).unwrap();
                continue;
            }
        };
        if signal == wait_signal {
            return Ok(pid);
        } else {
            ptrace::cont(pid, signal).map_err(|_| ExecEnd(None))?;
        }
    }
}

/// Add the memory events from `op` being executed while there is a memory access at `addr` to
/// `acc_events`. Return whether this was a memory operand.
fn capstone_find_events(
    addr: usize,
    op: &capstone::arch::ArchOperand,
    acc_events: &mut Vec<AccessEvent>,
) -> bool {
    use capstone::prelude::*;
    match op {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        arch::ArchOperand::X86Operand(x86_operand) => {
            match x86_operand.op_type {
                // We only care about memory accesses
                arch::x86::X86OperandType::Mem(_) => {
                    let push = AccessRange { addr, size: x86_operand.size.into() };
                    // It's called a "RegAccessType" but it also applies to memory
                    let acc_ty = x86_operand.access.unwrap();
                    // The same instruction might do both reads and writes, so potentially add both.
                    // We do not know the order in which they happened, but writing and then reading
                    // makes little sense so we put the read first. That is also the more
                    // conservative choice.
                    if acc_ty.is_readable() {
                        acc_events.push(AccessEvent::Read(push.clone()));
                    }
                    if acc_ty.is_writable() {
                        // FIXME: This could be made certain; either determine all cases where
                        // only reads happen, or have an intermediate mempr_* function to first
                        // map the page(s) as readonly and check if a segfault occurred.

                        // Per https://docs.rs/iced-x86/latest/iced_x86/enum.OpAccess.html,
                        // we know that the possible access types are Read, CondRead, Write,
                        // CondWrite, ReadWrite, and ReadCondWrite. Since we got a segfault
                        // we know some kind of access happened so Cond{Read, Write}s are
                        // certain reads and writes; the only uncertainty is with an RW op
                        // as it might be a ReadCondWrite with the write condition unmet.
                        acc_events.push(AccessEvent::Write(push, !acc_ty.is_readable()));
                    }

                    return true;
                }
                _ => (),
            }
        }
        // FIXME: arm64
        _ => unimplemented!(),
    }

    false
}

/// Extract the events from the given instruction.
fn capstone_disassemble(
    instr: &[u8],
    addr: usize,
    cs: &capstone::Capstone,
    acc_events: &mut Vec<AccessEvent>,
) -> capstone::CsResult<()> {
    // The arch_detail is what we care about, but it relies on these temporaries
    // that we can't drop. 0x1000 is the default base address for Captsone, and
    // we're expecting 1 instruction.
    let insns = cs.disasm_count(instr, 0x1000, 1)?;
    let ins_detail = cs.insn_detail(&insns[0])?;
    let arch_detail = ins_detail.arch_detail();

    let mut found_mem_op = false;

    for op in arch_detail.operands() {
        if capstone_find_events(addr, &op, acc_events) {
            if found_mem_op {
                panic!("more than one memory operand found; we don't know which one accessed what");
            }
            found_mem_op = true;
        }
    }

    Ok(())
}

/// Grabs the access that caused a segfault and logs it down if it's to our memory,
/// or kills the child and returns the appropriate error otherwise.
fn handle_segfault(
    pid: unistd::Pid,
    ch_pages: &[usize],
    ch_stack: usize,
    page_size: usize,
    cs: &capstone::Capstone,
    acc_events: &mut Vec<AccessEvent>,
) -> Result<(), ExecEnd> {
    // Get information on what caused the segfault. This contains the address
    // that triggered it.
    let siginfo = ptrace::getsiginfo(pid).unwrap();
    // All x86 instructions only have at most one memory operand (thankfully!)
    // SAFETY: si_addr is safe to call.
    let addr = unsafe { siginfo.si_addr().addr() };
    let page_addr = addr.strict_sub(addr.strict_rem(page_size));

    if !ch_pages.iter().any(|pg| (*pg..pg.strict_add(page_size)).contains(&addr)) {
        // This was a real segfault (not one of the Miri memory pages), so print some debug info and
        // quit.
        let regs = ptrace::getregs(pid).unwrap();
        eprintln!("Segfault occurred during FFI at {addr:#018x}");
        eprintln!("Expected access on pages: {ch_pages:#018x?}");
        eprintln!("Register dump: {regs:#x?}");
        ptrace::kill(pid).unwrap();
        return Err(ExecEnd(None));
    }

    // Overall structure:
    // - Get the address that caused the segfault
    // - Unprotect the memory: we force the child to execute `mempr_off`, passing parameters via
    //   global atomic variables. This is what we use the temporary callback stack for.
    // - Step 1 instruction
    // - Parse executed code to estimate size & type of access
    // - Reprotect the memory by executing `mempr_on` in the child, using the callback stack again.
    // - Continue

    // Ensure the stack is properly zeroed out!
    for a in (ch_stack..ch_stack.strict_add(CALLBACK_STACK_SIZE)).step_by(ARCH_WORD_SIZE) {
        ptrace::write(pid, std::ptr::with_exposed_provenance_mut(a), 0).unwrap();
    }

    // Guard against both architectures with upwards and downwards-growing stacks.
    let stack_ptr = ch_stack.strict_add(CALLBACK_STACK_SIZE / 2);
    let regs_bak = ptrace::getregs(pid).unwrap();
    let mut new_regs = regs_bak;

    // Read at least one instruction from the ip. It's possible that the instruction
    // that triggered the segfault was short and at the end of the mapped text area,
    // so some of these reads may fail; in that case, just write empty bytes. If all
    // reads failed, the disassembler will report an error.
    let instr = (0..(ARCH_MAX_INSTR_SIZE.div_ceil(ARCH_WORD_SIZE)))
        .flat_map(|ofs| {
            // This reads one word of memory; we divided by `ARCH_WORD_SIZE` above to compensate for that.
            ptrace::read(
                pid,
                std::ptr::without_provenance_mut(
                    regs_bak.ip().strict_add(ARCH_WORD_SIZE.strict_mul(ofs)),
                ),
            )
            .unwrap_or_default()
            .to_ne_bytes()
        })
        .collect::<Vec<_>>();

    // Now figure out the size + type of access and log it down.
    capstone_disassemble(&instr, addr, cs, acc_events).expect("Failed to disassemble instruction");

    // Move the instr ptr into the deprotection code.
    #[expect(clippy::as_conversions)]
    new_regs.set_ip(super::child::mempr_off as *const () as usize);
    // Don't mess up the stack by accident!
    new_regs.set_sp(stack_ptr);

    // Modify the PAGE_ADDR global on the child process to point to the page
    // that we want unprotected.
    ptrace::write(
        pid,
        (&raw const PAGE_ADDR).cast_mut().cast(),
        libc::c_long::try_from(page_addr.cast_signed()).unwrap(),
    )
    .unwrap();

    // Check if we also own the next page, and if so unprotect it in case
    // the access spans the page boundary.
    let flag = if ch_pages.contains(&page_addr.strict_add(page_size)) { 2 } else { 1 };
    ptrace::write(pid, (&raw const PAGE_COUNT).cast_mut().cast(), flag).unwrap();

    ptrace::setregs(pid, new_regs).unwrap();

    // Our mempr_* functions end with a raise(SIGSTOP).
    wait_for_signal(Some(pid), signal::SIGSTOP, InitialCont::Yes)?;

    // Step 1 instruction.
    ptrace::setregs(pid, regs_bak).unwrap();
    ptrace::step(pid, None).unwrap();
    // Don't use wait_for_signal here since 1 instruction doesn't give room
    // for any uncertainty + we don't want it `cont()`ing randomly by accident
    // Also, don't let it continue with unprotected memory if something errors!
    let stat = wait::waitid(wait::Id::Pid(pid), WAIT_FLAGS).map_err(|_| ExecEnd(None))?;
    match stat {
        wait::WaitStatus::Signaled(_, s, _)
        | wait::WaitStatus::Stopped(_, s)
        | wait::WaitStatus::PtraceEvent(_, s, _) =>
            assert!(
                !matches!(s, signal::SIGSEGV),
                "native code segfaulted when re-trying memory access\n\
                is the native code trying to call a Rust function?"
            ),
        _ => (),
    }

    // Zero out again to be safe
    for a in (ch_stack..ch_stack.strict_add(CALLBACK_STACK_SIZE)).step_by(ARCH_WORD_SIZE) {
        ptrace::write(pid, std::ptr::with_exposed_provenance_mut(a), 0).unwrap();
    }

    let regs_bak = ptrace::getregs(pid).unwrap();
    new_regs = regs_bak;

    // Reprotect everything and continue.
    #[expect(clippy::as_conversions)]
    new_regs.set_ip(super::child::mempr_on as *const () as usize);
    new_regs.set_sp(stack_ptr);
    ptrace::setregs(pid, new_regs).unwrap();
    wait_for_signal(Some(pid), signal::SIGSTOP, InitialCont::Yes)?;

    ptrace::setregs(pid, regs_bak).unwrap();
    ptrace::syscall(pid, None).unwrap();
    Ok(())
}

/// Determines what libc function was called that caused a sigtrap, giving control
/// to our shims to handle it instead.
fn handle_sigtrap(
    pid: unistd::Pid,
    page_size: usize,
    pages: &mut Vec<usize>,
    _event_tx: &ipc::IpcSender<MemEvents>,
    _acc_events: &mut Vec<AccessEvent>,
) -> Result<(), ExecEnd> {
    /// The libc functions we shim.
    enum LibcFn {
        Malloc,
        Calloc,
        AlignedAlloc,
        PosixMemalign,
        Realloc,
        Free,
    }

    /// Gets the libc function that a given instruction pointer corresponds to.
    fn get_libc_fn(addr: usize) -> Option<LibcFn> {
        // We'll be one instruction past the start
        #[expect(clippy::as_conversions)]
        match addr.strict_sub(BREAKPT_INSTR_SIZE) {
            a if a == (libc::malloc as *const () as usize) => Some(LibcFn::Malloc),
            a if a == (libc::calloc as *const () as usize) => Some(LibcFn::Calloc),
            a if a == (libc::aligned_alloc as *const () as usize) => Some(LibcFn::AlignedAlloc),
            a if a == (libc::posix_memalign as *const () as usize) => Some(LibcFn::PosixMemalign),
            a if a == (libc::realloc as *const () as usize) => Some(LibcFn::Realloc),
            a if a == (libc::free as *const () as usize) => Some(LibcFn::Free),
            _ => None,
        }
    }

    let regs = ptrace::getregs(pid).unwrap();
    match get_libc_fn(regs.ip()) {
        Some(_) => {
            // We'll possibly want to call libc functions in the interceptor shims,
            // so make sure they're working.
            fixup_libc(pid);
            // On x86, the return address will be the last item on the stack.
            let ret_addr: usize = ptrace::read(pid, std::ptr::without_provenance_mut(regs.sp()))
                .unwrap()
                .cast_unsigned()
                .try_into()
                .unwrap();

            // When libc is calling its own functions, we explicitly need to not
            // intercept them; therefore, we parse the process maps to determine
            // whether this is happening.
            let child_mappings = proc_maps::get_process_maps(pid.as_raw()).unwrap();
            // We know for sure libc functions are mapped *somewhere*, and they will be in a file
            // (unless something has gone awfully wrong).
            let libc_name = child_mappings
                .iter()
                .find(|&mp| {
                    // We use exit and not malloc since it seems malloc can be
                    // reported as being inside of the Miri binary's address space.
                    #[expect(clippy::as_conversions)]
                    (mp.start()..mp.start().strict_add(mp.size()))
                        .contains(&(libc::exit as *const () as usize))
                })
                .unwrap()
                .filename()
                .unwrap();
            // Is the return address inside of a block mapped from the same
            // file as libc functions?
            let ret_is_libc = child_mappings.iter().any(|mp| {
                if mp.filename().iter().any(|&name| name == libc_name) {
                    (mp.start()..mp.start().strict_add(mp.size())).contains(&ret_addr)
                } else {
                    false
                }
            });
            ptrace::write(pid, RET_IS_LIBC.as_ptr().cast(), ret_is_libc.into()).unwrap();

            // Override the return address to give us another sigtrap
            // (but save the original bytes).
            let ret_addr_bytes =
                ptrace::read(pid, std::ptr::without_provenance_mut(ret_addr)).unwrap();
            ptrace::write(pid, std::ptr::without_provenance_mut(ret_addr), BREAKPT_INSTR.into())
                .unwrap();
            wait_for_signal(Some(pid), signal::SIGTRAP, InitialCont::Yes).unwrap();

            // Unset the breakpoint stuff and move the ip back an instruction to compensate.
            ptrace::write(pid, std::ptr::without_provenance_mut(ret_addr), ret_addr_bytes).unwrap();
            let mut regs = ptrace::getregs(pid).unwrap();
            regs.set_ip(regs.ip().strict_sub(BREAKPT_INSTR_SIZE));
            ptrace::setregs(pid, regs).unwrap();

            // If the intercept modified the list of pages we need to monitor,
            // update our list accordingly.
            let new_pg_addr: usize = ptrace::read(pid, NEW_PAGES_ADDR.as_ptr().cast())
                .unwrap()
                .cast_unsigned()
                .try_into()
                .unwrap();
            if new_pg_addr != 0 {
                let new_pg_count: usize = ptrace::read(pid, NEW_PAGES_COUNT.as_ptr().cast())
                    .unwrap()
                    .cast_unsigned()
                    .try_into()
                    .unwrap();
                for add_fac in 0..new_pg_count {
                    pages.push(new_pg_addr.strict_add(add_fac.strict_mul(page_size)));
                }
            }

            let del_pg_addr: usize = ptrace::read(pid, DEL_PAGES_ADDR.as_ptr().cast())
                .unwrap()
                .cast_unsigned()
                .try_into()
                .unwrap();
            if del_pg_addr != 0 {
                let del_pg_count: usize = ptrace::read(pid, DEL_PAGES_COUNT.as_ptr().cast())
                    .unwrap()
                    .cast_unsigned()
                    .try_into()
                    .unwrap();
                for add_fac in 0..del_pg_count {
                    let pos = pages
                        .iter()
                        .position(|&pg| pg == del_pg_addr.strict_add(add_fac.strict_mul(page_size)))
                        .unwrap();
                    pages.remove(pos);
                }
            }
            // Now reenable stopping the process on libc calls.
            trap_libc(pid);
        }
        // This is a random sigtrap unrelated to our code.
        None => {
            eprintln!(
                "Process got an unexpected SIGTRAP at addr {:#0x?}; continuing...",
                regs.ip()
            );
        }
    };
    // Continue the process.
    ptrace::syscall(pid, None).unwrap();
    Ok(())
}
