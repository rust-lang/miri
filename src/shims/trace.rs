use ipc_channel::ipc;
use nix::sys::{ptrace, signal, wait};
use nix::unistd;

use crate::discrete_alloc;

// Note: we do NOT ever want to block the child from accessing this!!
static SUPERVISOR: std::sync::Mutex<Option<Supervisor>> = std::sync::Mutex::new(None);
static mut PAGE_ADDR: *mut libc::c_void = std::ptr::null_mut();
static mut CLICK_HERE_4_FREE_STACK: [u8; 1024] = [0; 1024];

#[cfg(target_pointer_width = "64")]
const BITS: u32 = 64;
#[cfg(target_pointer_width = "32")]
const BITS: u32 = 32;

trait ArchIndependentRegs {
    fn ax(&self) -> usize;
    fn di(&self) -> usize;
    fn si(&self) -> usize;
    fn ip(&self) -> usize;
    fn sp(&self) -> usize;
    fn orig_ax(&self) -> usize;
    fn set_ip(&mut self, ip: usize);
    fn set_sp(&mut self, sp: usize);
}

#[cfg(target_arch = "x86_64")]
#[allow(clippy::cast_possible_truncation)]
#[rustfmt::skip]
impl ArchIndependentRegs for libc::user_regs_struct {
    fn ax(&self) -> usize { self.rax as _ }
    fn di(&self) -> usize { self.rdi as _ }
    fn si(&self) -> usize { self.rsi as _ }
    fn ip(&self) -> usize { self.rip as _ }
    fn sp(&self) -> usize { self.rsp as _ }
    fn orig_ax(&self) -> usize { self.orig_rax as _ }
    fn set_ip(&mut self, ip: usize) { self.rip = ip as _ }
    fn set_sp(&mut self, sp: usize) { self.rsp = sp as _ }
}

#[cfg(target_arch = "x86")]
#[allow(clippy::cast_possible_truncation)]
#[rustfmt::skip]
impl ArchIndependentRegs for libc::user_regs_struct {
    fn ax(&self) -> usize { self.eax as _ }
    fn di(&self) -> usize { self.edi as _ }
    fn si(&self) -> usize { self.esi as _ }
    fn ip(&self) -> usize { self.eip as _ }
    fn sp(&self) -> usize { self.esp as _ }
    fn orig_ax(&self) -> usize { self.orig_eax as _ }
    fn set_ip(&mut self, ip: usize) { self.eip = ip as _ }
    fn set_sp(&mut self, sp: usize) { self.esp = sp as _ }
}

pub struct Supervisor {
    t_message: ipc::IpcSender<TraceRequest>,
    r_event: ipc::IpcReceiver<MemEvents>,
}

impl Supervisor {
    pub fn init() -> Result<(), ()> {
        let sv = SUPERVISOR.lock().map_err(|_| ())?;
        let is_none = sv.is_none();

        // I'm scared to find out what happens if we fork while holding a mutex
        drop(sv);

        if is_none {
            let (t_message, r_message) = ipc::channel().unwrap();
            let (t_event, r_event) = ipc::channel().unwrap();
            unsafe {
                match unistd::fork().unwrap() {
                    unistd::ForkResult::Parent { child } => {
                        let p = std::panic::catch_unwind(|| {
                            let listener =
                                ChildListener { rx: r_message, pid: child, attached: false };
                            sv_loop(listener, t_event)
                        });
                        eprintln!("{p:?}");
                        std::process::abort();
                    }
                    unistd::ForkResult::Child => {
                        let mut sv = SUPERVISOR.lock().map_err(|_| ())?;
                        *sv = Some(Supervisor { t_message, r_event });
                        Ok(())
                    }
                }
            }
        } else {
            Err(())
        }
    }

    #[allow(static_mut_refs)]
    pub unsafe fn start_ffi() -> Option<()> {
        let mut sv_guard = SUPERVISOR.lock().ok()?;
        let sv = sv_guard.take()?;
        let exposed = discrete_alloc::MachineAlloc::pages();
        //let (mappings, stack) = parse_mappings();
        /*let heap_addr = unsafe {
            let ptr = libc::malloc(1);
            libc::free(ptr);
            ptr as usize
        };*/
        sv.t_message.send(TraceRequest::BeginFfi(exposed)).ok()?;
        *sv_guard = Some(sv);
        unsafe {
            discrete_alloc::MachineAlloc::prepare_ffi()?;
        }
        signal::raise(signal::SIGSTOP).unwrap();
        Some(())
    }

    #[allow(static_mut_refs)]
    pub unsafe fn end_ffi() -> Option<()> {
        let mut sv_guard = SUPERVISOR.lock().ok()?;
        let sv = sv_guard.take()?;
        sv.t_message.send(TraceRequest::EndFfi).ok()?;
        *sv_guard = Some(sv);
        discrete_alloc::MachineAlloc::unprep_ffi()?;
        Some(())
    }

    pub fn get_events() -> Option<MemEvents> {
        let mut sv_guard = SUPERVISOR.lock().ok()?;
        let sv = sv_guard.take()?;
        let ret = sv.r_event.recv().ok();
        *sv_guard = Some(sv);
        ret
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum TraceRequest {
    BeginFfi(Vec<usize>),
    EndFfi,
    Die,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct MemEvents {
    pub accesses: Vec<(usize, usize, MemAccessType)>,
    pub mappings: Vec<(usize, usize)>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum MemAccessType {
    Read,
    Write,
    ReadWrite,
}

impl MemAccessType {
    fn update(&mut self, other: MemAccessType) {
        match self {
            MemAccessType::Read =>
                match other {
                    MemAccessType::Read => (),
                    _ => *self = MemAccessType::ReadWrite,
                },
            MemAccessType::Write =>
                match other {
                    MemAccessType::Write => (),
                    _ => *self = MemAccessType::ReadWrite,
                },
            MemAccessType::ReadWrite => (),
        }
    }
}

struct ChildListener {
    rx: ipc::IpcReceiver<TraceRequest>,
    pid: unistd::Pid,
    attached: bool,
}

enum ExecEvent {
    Status(wait::WaitStatus),
    Request(TraceRequest),
    Died(i32),
}

impl Iterator for ChildListener {
    type Item = ExecEvent;

    fn next(&mut self) -> Option<Self::Item> {
        let opts = wait::WaitPidFlag::WNOHANG;
        loop {
            match wait::waitpid(self.pid, Some(opts)) {
                Ok(stat) =>
                    match stat {
                        wait::WaitStatus::Exited(_pid, retcode) =>
                            return Some(ExecEvent::Died(retcode)),
                        wait::WaitStatus::StillAlive => (),
                        _ =>
                            if self.attached {
                                return Some(ExecEvent::Status(stat));
                            },
                    },
                Err(errno) => {
                    return Some(ExecEvent::Died(errno as i32));
                }
            }

            if let Ok(msg) = self.rx.try_recv() {
                match &msg {
                    TraceRequest::BeginFfi(_) => self.attached = true,
                    TraceRequest::EndFfi => self.attached = false,
                    _ => (),
                }
                return Some(ExecEvent::Request(msg));
            }

            std::thread::yield_now();
        }
    }
}

/// This is the main loop of the supervisor process. It runs in a separate
/// process from the rest of Miri (but because we fork, addresses for anything
/// created before the fork are the same).
fn sv_loop(listener: ChildListener, t_event: ipc::IpcSender<MemEvents>) -> ! {
    // Things that we return to the child process
    let mut accesses: Vec<(usize, usize, MemAccessType)> = vec![];
    let mut mappings: Vec<(usize, usize)> = vec![];

    // Memory allocated on the MiriMachine
    let mut ch_pages = vec![];

    // Random bits of syscall-related state
    let mut enter = true;
    let mut flush_mapped = None;
    let mut munmap_args = None;

    // Straight up magic numbers we need
    let malloc_addr = libc::malloc as usize;
    let malloc_bytes = unsafe { (malloc_addr as *mut i64).read_volatile() }; // i'm sorry
    let realloc_addr = libc::realloc as usize;
    let realloc_bytes = unsafe { (realloc_addr as *mut i64).read_volatile() };
    let free_addr = libc::free as usize;
    let free_bytes = unsafe { (free_addr as *mut i64).read_volatile() };

    let main_pid = listener.pid;
    let mut retcode = 0;

    #[allow(clippy::cast_possible_wrap)]
    #[allow(clippy::arithmetic_side_effects)]
    'listen: for evt in listener {
        match evt {
            ExecEvent::Status(wait_status) =>
                match wait_status {
                    // Process killed by signal
                    wait::WaitStatus::Signaled(_pid, signal, _) => {
                        eprintln!("Process killed by {signal:?}");
                        retcode = 1;
                        break 'listen;
                    }
                    wait::WaitStatus::Stopped(pid, signal) => {
                        match signal {
                            signal::SIGSEGV => {
                                handle_segfault(pid, &ch_pages, &mut accesses);
                            }
                            signal::SIGTRAP => {
                                // should only trigger on malloc-related calls
                                handle_sigtrap(
                                    pid,
                                    &mut mappings,
                                    malloc_bytes,
                                    realloc_bytes,
                                    free_bytes,
                                );
                            }
                            _ => {
                                eprintln!(
                                    "Process unexpectedly stopped at {signal}; continuing..."
                                );
                                ptrace::syscall(pid, None).unwrap();
                            }
                        }
                    }
                    wait::WaitStatus::PtraceEvent(pid, signal, sig) => {
                        eprintln!("Got unexpected event {sig} with {signal}; continuing...");
                        ptrace::syscall(pid, None).unwrap();
                    }
                    wait::WaitStatus::PtraceSyscall(pid) => {
                        let regs = ptrace::getregs(pid).unwrap();
                        let syscall_nr = regs.orig_ax();
                        if enter {
                            if syscall_nr as i64 == libc::SYS_mmap {
                                let len = regs.si();
                                flush_mapped = Some(len);
                            } else if syscall_nr as i64 == libc::SYS_munmap {
                                // The unmap call might hit multiple mappings we've saved,
                                // or overlap with them partially (or both)
                                let um_start = regs.di();
                                let um_len = regs.si();
                                let um_end = um_start + um_len;
                                let mut idxes = vec![];
                                for (idx, &(mp_start, len)) in mappings.iter().enumerate() {
                                    let mp_end = mp_start + len;
                                    let cond = (mp_start..mp_end).contains(&um_start)
                                        || (mp_start..mp_end).contains(&um_end)
                                        || (um_start..um_end).contains(&mp_start);

                                    if cond {
                                        idxes.push(idx);
                                    }
                                }
                                if !idxes.is_empty() {
                                    // We iterate thru this later while removing elements so if
                                    // it's not reversed we will mess up the mappings badly!
                                    idxes.reverse();
                                    munmap_args = Some((idxes, um_start, um_len));
                                }
                            } // TODO: handle brk/sbrk
                        } else {
                            if syscall_nr as i64 == libc::SYS_mmap {
                                if regs.ax() as isize > 0 {
                                    if let Some(len) = flush_mapped.take() {
                                        let addr = regs.ax();
                                        mappings.push((addr, len as _));
                                    } else {
                                        eprintln!(
                                            "Process returned from mmap syscall without entering it? Attempting to continue..."
                                        );
                                    }
                                }
                            } else if syscall_nr as i64 == libc::SYS_munmap {
                                if let Some((idxes, um_start, um_len)) = munmap_args.take() {
                                    if regs.ax() as isize > 0 {
                                        // Unmap succeeded, so take out the mapping(s) from our list
                                        // but it may be only partial so we may readd some sections
                                        for idx in idxes {
                                            let um_end = um_start + um_len;
                                            let (mp_start, mp_len) = mappings.remove(idx);
                                            let mp_end = mp_len + mp_len;

                                            if mp_start < um_start {
                                                let preserved_len_head = um_start - mp_start;
                                                mappings.push((mp_start, preserved_len_head));
                                            }
                                            if mp_end > um_end {
                                                let preserved_len_tail = mp_end - um_end;
                                                mappings.push((um_end, preserved_len_tail));
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        enter = !enter;
                        ptrace::syscall(pid, None).unwrap();
                    }
                    _ => (),
                },
            ExecEvent::Request(trace_request) =>
                match trace_request {
                    TraceRequest::BeginFfi(child_pages) => {
                        ch_pages = child_pages;

                        ptrace::seize(main_pid, ptrace::Options::PTRACE_O_TRACESYSGOOD).unwrap();
                        let _ = wait::waitpid(main_pid, None).unwrap();

                        ptrace::write(main_pid, malloc_addr as _, 0xcc).unwrap();
                        ptrace::write(main_pid, realloc_addr as _, 0xcc).unwrap();
                        ptrace::write(main_pid, free_addr as _, 0xcc).unwrap();

                        ptrace::syscall(main_pid, None).unwrap();
                    }

                    TraceRequest::EndFfi => {
                        signal::kill(main_pid, signal::SIGSTOP).unwrap();
                        t_event.send(MemEvents { accesses, mappings }).unwrap();
                        accesses = vec![];
                        mappings = vec![];
                        let _ = wait::waitpid(main_pid, None).unwrap();
                        ptrace::write(main_pid, malloc_addr as _, malloc_bytes).unwrap();
                        ptrace::write(main_pid, realloc_addr as _, realloc_bytes).unwrap();
                        ptrace::write(main_pid, free_addr as _, free_bytes).unwrap();
                        ptrace::detach(main_pid, None).unwrap();
                        signal::kill(main_pid, signal::SIGCONT).unwrap();
                    }

                    TraceRequest::Die => break 'listen,
                },
            ExecEvent::Died(child_code) => {
                if child_code != 0 {
                    eprintln!("Process exited with code {child_code}");
                    retcode = child_code;
                }
                break 'listen;
            }
        }
    }

    std::process::exit(retcode);
}

#[allow(clippy::cast_possible_wrap)]
#[allow(clippy::arithmetic_side_effects)]
fn handle_segfault(
    pid: unistd::Pid,
    ch_pages: &[usize],
    accesses: &mut Vec<(usize, usize, MemAccessType)>,
) {
    let siginfo = ptrace::getsiginfo(pid).unwrap();
    let addr = unsafe { siginfo.si_addr().addr() };
    let page_addr = addr - addr % 4096;

    if ch_pages.contains(&page_addr) {
        // Overall structure:
        // - Get the address that caused the segfault
        // - Unprotect the memory
        // - Step 1 instruction
        // - Parse executed code to estimate size & type of access
        // - Reprotect the memory
        // - Continue
        let regs_bak = ptrace::getregs(pid).unwrap();
        let mut new_regs = regs_bak;
        let ip_prestep = regs_bak.ip();

        // Move the instr ptr into the deprotection code
        new_regs.set_ip((mempr_off as *const ()).addr());
        // Don't mess up the stack by accident!
        new_regs.set_sp(unsafe { (&raw mut CLICK_HERE_4_FREE_STACK[512]).addr() });

        ptrace::write(pid, &raw mut PAGE_ADDR as _, page_addr as _).unwrap();
        ptrace::setregs(pid, new_regs).unwrap();

        ptrace::cont(pid, None).unwrap();
        let _ = wait::waitpid(pid, None).unwrap();

        // Step 1 instruction then reprotect memory
        ptrace::setregs(pid, regs_bak).unwrap();
        ptrace::step(pid, None).unwrap();

        let _ = wait::waitpid(pid, None);
        let regs_bak = ptrace::getregs(pid).unwrap();
        new_regs = regs_bak;
        let ip_poststep = regs_bak.ip();
        let diff = (ip_poststep - ip_prestep).div_ceil(8);
        let instr = (ip_prestep..ip_prestep + diff).fold(vec![], |mut ret, ip| {
            ret.append(&mut ptrace::read(pid, ip as _).unwrap().to_ne_bytes().to_vec());
            ret
        });
        let mut decoder = iced_x86::Decoder::new(BITS, instr.as_slice(), 0);
        let mut fac = iced_x86::InstructionInfoFactory::new();
        let instr = decoder.decode();
        let memsize = instr.op_code().memory_size().size();
        let mem = fac.info(&instr).used_memory();
        let acc = mem.iter().fold(None, |mut curr: Option<MemAccessType>, m| {
            if let Some(m) = match m.access() {
                iced_x86::OpAccess::Read => Some(MemAccessType::Read),
                iced_x86::OpAccess::CondRead => Some(MemAccessType::Read),
                iced_x86::OpAccess::Write => Some(MemAccessType::Write),
                iced_x86::OpAccess::CondWrite => Some(MemAccessType::Write),
                iced_x86::OpAccess::ReadWrite => Some(MemAccessType::ReadWrite),
                iced_x86::OpAccess::ReadCondWrite => Some(MemAccessType::ReadWrite),
                _ => None,
            } {
                if let Some(curr) = curr.as_mut() {
                    curr.update(m);
                } else {
                    curr = Some(m);
                }
            }
            curr
        });
        if let Some(acc) = acc {
            match accesses.iter().position(|&(a, len, _)| a == addr && len == memsize) {
                Some(pos) => accesses[pos].2.update(acc),
                None => accesses.push((addr, memsize, acc)),
            }
        }
        new_regs.set_ip((mempr_on as *const ()).addr());
        new_regs.set_sp(unsafe { (&raw mut CLICK_HERE_4_FREE_STACK[512]).addr() });
        ptrace::setregs(pid, new_regs).unwrap();
        ptrace::cont(pid, None).unwrap();
        let _ = wait::waitpid(pid, None).unwrap();

        ptrace::setregs(pid, regs_bak).unwrap();
        ptrace::syscall(pid, None).unwrap();
    } else {
        let regs = ptrace::getregs(pid).unwrap();
        eprintln!("Segfault occurred during FFI at {addr:#018x}\nRegister dump: {regs:#x?}");
        ptrace::kill(pid).unwrap();
    }
}

#[allow(clippy::arithmetic_side_effects)]
fn handle_sigtrap(
    pid: unistd::Pid,
    mappings: &mut Vec<(usize, usize)>,
    malloc_bytes: i64,
    realloc_bytes: i64,
    free_bytes: i64,
) {
    // We can re-derive these pointers, no need to pass them in
    let malloc_addr = libc::malloc as usize;
    let realloc_addr = libc::realloc as usize;
    let free_addr = libc::free as usize;

    let regs = ptrace::getregs(pid).unwrap();
    match regs.ip() - 1 {
        a if a == malloc_addr => {
            let size = regs.di(); // !
            let ptr = intercept_retptr(pid, regs, malloc_addr, malloc_bytes);
            mappings.push((ptr as _, size));
        }
        a if a == realloc_addr => {
            let old_ptr = regs.di();
            let size = regs.si();
            let pos = mappings
                .iter()
                .position(|&(ptr, size)| ptr <= old_ptr as _ && (old_ptr as usize) < ptr + size);
            if let Some(pos) = pos {
                let _ = mappings.remove(pos);
            }
            let ptr = intercept_retptr(pid, regs, realloc_addr, realloc_bytes);
            mappings.push((ptr as _, size as _));
        }
        a if a == free_addr => {
            let old_ptr = regs.di();
            //let size = regs.rdi;
            let pos =
                mappings.iter().position(|&(ptr, size)| ptr <= old_ptr && old_ptr < ptr + size);
            if let Some(pos) = pos {
                let _ = mappings.remove(pos);
            }
            let _ = intercept_retptr(pid, regs, free_addr, free_bytes);
        }
        a => {
            eprintln!("Process got an unexpected SIGTRAP at addr {a:#018x?}; continuing...");
            ptrace::syscall(pid, None).unwrap();
        }
    }
}

#[allow(clippy::arithmetic_side_effects)]
fn intercept_retptr(
    pid: unistd::Pid,
    mut regs: libc::user_regs_struct,
    fn_addr: usize,
    fn_bytes: i64,
) -> usize {
    // Outline:
    // - Move instr ptr back before the sigtrap happened
    // - Restore the function to what it's supposed to be
    // - Change the function we're returning to so it gives us a sigtrap
    // - Catch it there
    // - Get the register-sized return value
    // - Patch the function back so it traps as before
    regs.set_ip(regs.ip() - 1);
    let ret_addr = ptrace::read(pid, regs.sp() as _).unwrap();
    let ret_bytes = ptrace::read(pid, ret_addr as _).unwrap();
    ptrace::write(pid, ret_addr as _, 0xcc).unwrap();
    ptrace::write(pid, fn_addr as _, fn_bytes).unwrap();
    ptrace::setregs(pid, regs).unwrap();

    ptrace::cont(pid, None).unwrap();
    let _ = wait::waitpid(pid, None).unwrap();

    // now we're getting the return hopefully
    let mut regs = ptrace::getregs(pid).unwrap();
    let ptr = regs.ax(); // !
    regs.set_ip(regs.ip() - 1);
    ptrace::write(pid, fn_addr as _, 0xcc).unwrap();
    ptrace::write(pid, ret_addr as _, ret_bytes).unwrap();
    ptrace::setregs(pid, regs).unwrap();

    ptrace::syscall(pid, None).unwrap();
    ptr
}

// We only get dropped into these functions via offsetting the instr pointer
// manually, so we *must not ever* unwind from it
pub unsafe extern "C" fn mempr_off() {
    unsafe {
        let _ = libc::mprotect(PAGE_ADDR, 4096, libc::PROT_READ | libc::PROT_WRITE);
    }
    let _ = signal::raise(signal::SIGSTOP);
}

pub unsafe extern "C" fn mempr_on() {
    unsafe {
        let _ = libc::mprotect(PAGE_ADDR, 4096, libc::PROT_NONE);
    }
    let _ = signal::raise(signal::SIGSTOP);
}
