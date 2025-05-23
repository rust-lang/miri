use std::ops::Range;

use ipc_channel::ipc;
use nix::sys::{ptrace, signal, wait};
use nix::unistd;

use crate::discrete_alloc;
use crate::helpers::ToU64;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const BREAKPT_INSTR: isize = 0xCC;
#[cfg(target_arch = "aarch64")]
const BREAKPT_INSTR: isize = 0xD420;

// FIXME: Make this architecture-specific
const ARCH_MAX_ACCESS_SIZE: u64 = 256;

// We do NOT ever want to block the child from accessing this!!
static SUPERVISOR: std::sync::Mutex<Option<Supervisor>> = std::sync::Mutex::new(None);
static mut PAGE_ADDR: *mut libc::c_void = std::ptr::null_mut();
static mut PAGE_SIZE: u64 = 4096;
static mut CLICK_HERE_4_FREE_STACK: [u8; 1024] = [0; 1024];

trait ArchIndependentRegs {
    // see https://man7.org/linux/man-pages/man2/syscall.2.html
    fn retval(&self) -> usize;
    fn arg1(&self) -> usize;
    fn arg2(&self) -> usize;
    fn syscall_nr(&self) -> usize;
    fn ip(&self) -> usize;
    fn sp(&self) -> usize;
    fn set_ip(&mut self, ip: usize);
    fn set_sp(&mut self, sp: usize);
}

// It's fine / desirable behaviour for values to wrap here, we care about just
// preserving the bit pattern
#[cfg(target_arch = "x86_64")]
#[expect(clippy::as_conversions)]
#[rustfmt::skip]
impl ArchIndependentRegs for libc::user_regs_struct {
    fn retval(&self) -> usize { self.rax as _ }
    fn arg1(&self) -> usize { self.rdi as _ }
    fn arg2(&self) -> usize { self.rsi as _ }
    fn syscall_nr(&self) -> usize { self.orig_rax as _ }
    fn ip(&self) -> usize { self.rip as _ }
    fn sp(&self) -> usize { self.rsp as _ }
    fn set_ip(&mut self, ip: usize) { self.rip = ip as _ }
    fn set_sp(&mut self, sp: usize) { self.rsp = sp as _ }
}

#[cfg(target_arch = "x86")]
#[expect(clippy::as_conversions)]
#[rustfmt::skip]
impl ArchIndependentRegs for libc::user_regs_struct {
    fn retval(&self) -> usize { self.eax as _ }
    fn arg1(&self) -> usize { self.edi as _ }
    fn arg2(&self) -> usize { self.esi as _ }
    fn syscall_nr(&self) -> usize { self.orig_eax as _ }
    fn ip(&self) -> usize { self.eip as _ }
    fn sp(&self) -> usize { self.esp as _ }
    fn set_ip(&mut self, ip: usize) { self.eip = ip as _ }
    fn set_sp(&mut self, sp: usize) { self.esp = sp as _ }
}

#[cfg(target_arch = "aarch64")]
#[expect(clippy::as_conversions)]
#[rustfmt::skip]
impl ArchIndependentRegs for libc::user_regs_struct {
    fn retval(&self) -> usize { self.regs[0] as _ }
    fn arg1(&self) -> usize { self.regs[0] as _ }
    fn arg2(&self) -> usize { self.regs[1] as _ }
    fn syscall_nr(&self) -> usize { self.regs[8] as _ }
    fn ip(&self) -> usize { self.pc as _ }
    fn sp(&self) -> usize { self.sp as _ }
    fn set_ip(&mut self, ip: usize) { self.pc = ip as _ }
    fn set_sp(&mut self, sp: usize) { self.sp = sp as _ }
}

pub struct Supervisor {
    t_message: ipc::IpcSender<TraceRequest>,
    r_event: ipc::IpcReceiver<MemEvents>,
}

impl Supervisor {
    pub fn init() -> Result<(), ()> {
        let ptrace_status = std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope");
        if let Ok(stat) = ptrace_status {
            if let Some(stat) = stat.chars().next() {
                // Fast-error if ptrace is disabled on the system and it's linux
                if stat != '0' && stat != '1' {
                    return Err(());
                }
            }
        }

        let sv = SUPERVISOR.lock().map_err(|_| ())?;
        let is_none = sv.is_none();

        // I'm scared to find out what happens if we fork while holding a mutex
        drop(sv);

        if is_none {
            unsafe {
                let ret = libc::sysconf(libc::_SC_PAGESIZE);
                if ret > 0 {
                    PAGE_SIZE = ret.try_into().unwrap()
                }
            }

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
                        std::process::exit(-1);
                    }
                    unistd::ForkResult::Child => {
                        let mut sv = SUPERVISOR.lock().map_err(|_| ())?;
                        *sv = Some(Supervisor { t_message, r_event });
                    }
                }
            }
        }
        Ok(())
    }

    pub unsafe fn start_ffi() {
        let mut sv_guard = SUPERVISOR.lock().unwrap();
        if let Some(sv) = sv_guard.take() {
            let exposed = discrete_alloc::MachineAlloc::pages();
            sv.t_message.send(TraceRequest::BeginFfi(exposed)).unwrap();
            *sv_guard = Some(sv);
            unsafe {
                if discrete_alloc::MachineAlloc::prepare_ffi().is_err() {
                    // Don't mess up unwinding by maybe leaving the memory partly protected
                    discrete_alloc::MachineAlloc::unprep_ffi();
                    panic!("Cannot protect memory for FFI call!");
                }
            }
            signal::raise(signal::SIGSTOP).unwrap();
        }
    }

    pub unsafe fn end_ffi() {
        let mut sv_guard = SUPERVISOR.lock().unwrap();
        if let Some(sv) = sv_guard.take() {
            sv.t_message.send(TraceRequest::EndFfi).unwrap();
            *sv_guard = Some(sv);
            drop(sv_guard);
            discrete_alloc::MachineAlloc::unprep_ffi();
        }
    }

    pub fn get_events() -> Option<MemEvents> {
        let mut sv_guard = SUPERVISOR.lock().unwrap();
        let sv = sv_guard.take()?;
        // On the off-chance something really weird happens, don't block forever
        let ret = sv
            .r_event
            .try_recv_timeout(std::time::Duration::from_secs(1))
            .map_err(|e| {
                match e {
                    ipc::TryRecvError::IpcError(e) => ipc::TryRecvError::IpcError(e),
                    ipc::TryRecvError::Empty => {
                        // timed out!
                        eprintln!("Waiting for accesses from supervisor timed out!");
                        ipc::TryRecvError::Empty
                    }
                }
            })
            .ok();
        *sv_guard = Some(sv);
        ret
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum TraceRequest {
    BeginFfi(Vec<u64>),
    EndFfi,
    Die,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct MemEvents {
    pub reads: Vec<Range<u64>>,
    pub writes: Vec<Range<u64>>,
    pub mappings: Vec<Range<u64>>,
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
                    // Not really any other way to do this cast
                    #[expect(clippy::as_conversions)]
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
    let mut reads: Vec<Range<u64>> = vec![];
    let mut writes: Vec<Range<u64>> = vec![];
    let mut mappings: Vec<Range<u64>> = vec![];

    // An instance of the Capstone disassembler, so we don't spawn one on every access
    let cs = get_disasm();

    // Memory allocated on the MiriMachine
    let mut ch_pages = vec![];

    // Straight up magic numbers we need; no way to get around `as` casts (I
    // think at least)
    #[expect(clippy::as_conversions)]
    let malloc_addr = libc::malloc as usize;
    let malloc_bytes =
        unsafe { std::ptr::with_exposed_provenance::<i64>(malloc_addr).read_volatile() }; // i'm sorry
    #[expect(clippy::as_conversions)]
    let realloc_addr = libc::realloc as usize;
    let realloc_bytes =
        unsafe { std::ptr::with_exposed_provenance::<i64>(realloc_addr).read_volatile() };
    #[expect(clippy::as_conversions)]
    let free_addr = libc::free as usize;
    let free_bytes = unsafe { std::ptr::with_exposed_provenance::<i64>(free_addr).read_volatile() };

    let main_pid = listener.pid;
    let mut retcode = 0;

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
                                if let Err(ret) =
                                    handle_segfault(pid, &ch_pages, &cs, &mut reads, &mut writes)
                                {
                                    retcode = ret;
                                    break 'listen;
                                }
                            }
                            signal::SIGTRAP => {
                                // should only trigger on malloc-related calls
                                if let Err(ret) = handle_sigtrap(
                                    pid,
                                    &mut mappings,
                                    malloc_bytes,
                                    realloc_bytes,
                                    free_bytes,
                                ) {
                                    retcode = ret;
                                    break 'listen;
                                }
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
                        let syscall_nr = regs.syscall_nr();
                        match i64::try_from(syscall_nr).unwrap() {
                            n if n == libc::SYS_mmap => {
                                // No need for a discrete fn here, it's very tiny
                                let len = regs.arg2();
                                ptrace::syscall(pid, None).unwrap();
                                match wait_for_syscall(pid, libc::SYS_mmap) {
                                    Ok(regs) => {
                                        // We *want* this to wrap
                                        #[expect(clippy::as_conversions)]
                                        if regs.retval() as isize > 0 {
                                            let addr = regs.retval();
                                            mappings.push(
                                                addr.to_u64()
                                                    ..addr.to_u64().strict_add(len.to_u64()),
                                            );
                                        }
                                    }
                                    Err(ret) => {
                                        retcode = ret;
                                        break 'listen;
                                    }
                                }
                            }
                            n if n == libc::SYS_munmap => {
                                if let Err(ret) = handle_munmap(pid, regs, &mut mappings) {
                                    retcode = ret;
                                    break 'listen;
                                }
                            }
                            // TODO: handle brk/sbrk
                            _ => (),
                        }

                        ptrace::syscall(pid, None).unwrap();
                    }
                    _ => (),
                },
            ExecEvent::Request(trace_request) =>
                match trace_request {
                    TraceRequest::BeginFfi(child_pages) => {
                        ch_pages = child_pages;

                        ptrace::seize(main_pid, ptrace::Options::PTRACE_O_TRACESYSGOOD).unwrap();
                        if let Err(ret) = wait_for_signal(main_pid, signal::SIGSTOP, false) {
                            retcode = ret;
                            break 'listen;
                        }

                        ptrace::write(
                            main_pid,
                            std::ptr::with_exposed_provenance_mut::<libc::c_void>(malloc_addr),
                            BREAKPT_INSTR.try_into().unwrap(),
                        )
                        .unwrap();
                        ptrace::write(
                            main_pid,
                            std::ptr::with_exposed_provenance_mut::<libc::c_void>(realloc_addr),
                            BREAKPT_INSTR.try_into().unwrap(),
                        )
                        .unwrap();
                        ptrace::write(
                            main_pid,
                            std::ptr::with_exposed_provenance_mut::<libc::c_void>(free_addr),
                            BREAKPT_INSTR.try_into().unwrap(),
                        )
                        .unwrap();

                        ptrace::syscall(main_pid, None).unwrap();
                    }

                    TraceRequest::EndFfi => {
                        signal::kill(main_pid, signal::SIGSTOP).unwrap();
                        t_event.send(MemEvents { reads, writes, mappings }).unwrap();
                        reads = vec![];
                        writes = vec![];
                        mappings = vec![];
                        if let Err(ret) = wait_for_signal(main_pid, signal::SIGSTOP, false) {
                            retcode = ret;
                            break 'listen;
                        }
                        ptrace::write(
                            main_pid,
                            std::ptr::with_exposed_provenance_mut::<libc::c_void>(malloc_addr),
                            malloc_bytes,
                        )
                        .unwrap();
                        ptrace::write(
                            main_pid,
                            std::ptr::with_exposed_provenance_mut::<libc::c_void>(realloc_addr),
                            realloc_bytes,
                        )
                        .unwrap();
                        ptrace::write(
                            main_pid,
                            std::ptr::with_exposed_provenance_mut::<libc::c_void>(free_addr),
                            free_bytes,
                        )
                        .unwrap();
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

fn get_disasm() -> capstone::Capstone {
    use capstone::prelude::*;
    let cs_pre = Capstone::new();
    {
        #[cfg(target_arch = "x86_64")]
        {
            cs_pre.x86().mode(arch::x86::ArchMode::Mode64)
        }
        #[cfg(target_arch = "x86")]
        {
            cs_pre.x86().mode(arch::x86::ArchMode::Mode32)
        }
        #[cfg(target_arch = "aarch64")]
        {
            cs_pre.arm64()
        }
        #[cfg(target_arch = "arm")]
        {
            cs_pre.arm()
        }
        #[cfg(target_arch = "riscv64")]
        {
            cs_pre.riscv().mode(arch::riscv::ArchMode::RiscV64)
        }
        #[cfg(target_arch = "riscv32")]
        {
            cs_pre.riscv().mode(arch::riscv::ArchMode::RiscV32)
        }
    }
    .detail(true)
    .build()
    .unwrap()
}

/// Waits for a specific signal to be triggered.
fn wait_for_signal(
    pid: unistd::Pid,
    wait_signal: signal::Signal,
    init_cont: bool,
) -> Result<(), i32> {
    if init_cont {
        ptrace::cont(pid, None).unwrap();
    }
    loop {
        let stat = wait::waitpid(pid, None).unwrap();
        let signal = match stat {
            wait::WaitStatus::Exited(_, status) => return Err(status),
            wait::WaitStatus::Signaled(_, signal, _) => signal,
            wait::WaitStatus::Stopped(_, signal) => signal,
            wait::WaitStatus::PtraceEvent(_, signal, _) => signal,
            _ => {
                ptrace::cont(pid, None).unwrap();
                continue;
            }
        };
        if signal == wait_signal {
            break;
        } else {
            ptrace::cont(pid, None).unwrap();
        }
    }
    Ok(())
}

/// Waits for the child to return from its current syscall, grabbing its registers.
fn wait_for_syscall(pid: unistd::Pid, syscall: i64) -> Result<libc::user_regs_struct, i32> {
    loop {
        ptrace::syscall(pid, None).unwrap();
        let stat = wait::waitpid(pid, None).unwrap();
        match stat {
            wait::WaitStatus::Exited(_, status) => return Err(status),
            wait::WaitStatus::PtraceSyscall(pid) => {
                let regs = ptrace::getregs(pid).unwrap();
                if regs.syscall_nr() == usize::try_from(syscall).unwrap() {
                    return Ok(regs);
                } else {
                    panic!("Missed syscall while waiting for it: {syscall}");
                }
            }
            _ => (),
        }
    }
}

fn handle_munmap(
    pid: unistd::Pid,
    regs: libc::user_regs_struct,
    mappings: &mut Vec<Range<u64>>,
) -> Result<(), i32> {
    // The unmap call might hit multiple mappings we've saved,
    // or overlap with them partially (or both)
    let um_start = regs.arg1().to_u64();
    let um_len = regs.arg2().to_u64();
    let um_end = um_start.strict_add(um_len);
    let mut idxes = vec![];
    for (idx, mp) in mappings.iter().enumerate() {
        let cond = mp.contains(&um_start)
            || mp.contains(&um_end)
            || (um_start..um_end).contains(&mp.start);

        if cond {
            idxes.push(idx);
        }
    }
    // We iterate thru this while removing elements so if
    // it's not reversed we will mess up the mappings badly!
    idxes.reverse();

    ptrace::syscall(pid, None).unwrap();
    let regs = wait_for_syscall(pid, libc::SYS_munmap)?;
    //let regs = ptrace::getregs(pid).unwrap();

    // Again, this *should* wrap
    #[expect(clippy::as_conversions)]
    if regs.retval() as isize > 0 {
        // Unmap succeeded, so take out the mapping(s) from our list
        // but it may be only partial so we may readd some sections
        for idx in idxes {
            let um_end = um_start.strict_add(um_len);
            let mp = mappings.remove(idx);

            if mp.start < um_start {
                let preserved_len_head = um_start.strict_sub(mp.start);
                mappings.push(mp.start..mp.start.strict_add(preserved_len_head));
            }
            if mp.end > um_end {
                let preserved_len_tail = mp.end.strict_sub(um_end);
                mappings.push(um_end..um_end.strict_add(preserved_len_tail));
            }
        }
    }

    Ok(())
}

fn handle_segfault(
    pid: unistd::Pid,
    ch_pages: &[u64],
    cs: &capstone::Capstone,
    reads: &mut Vec<Range<u64>>,
    writes: &mut Vec<Range<u64>>,
) -> Result<(), i32> {
    // This is just here to not pollute the main namespace with capstone::prelude::*
    // and so that we can get a Result instead of just unwrapping on error
    #[inline]
    fn capstone_disassemble(
        instr: &[u8],
        addr: u64,
        cs: &capstone::Capstone,
        reads: &mut Vec<Range<u64>>,
        writes: &mut Vec<Range<u64>>,
    ) -> capstone::CsResult<()> {
        use capstone::prelude::*;

        let insns = cs.disasm_count(instr, 0x1000, 1)?;
        let ins_detail = cs.insn_detail(&insns[0])?;
        let arch_detail = ins_detail.arch_detail();

        for op in arch_detail.operands() {
            match op {
                arch::ArchOperand::X86Operand(x86_operand) => {
                    let size: u64 = x86_operand.size.into();
                    match x86_operand.op_type {
                        arch::x86::X86OperandType::Mem(_) => {
                            // It's called a "RegAccessType" but it also applies to memory
                            let acc_ty = x86_operand.access.unwrap();
                            if acc_ty.is_readable() {
                                reads.push(addr..addr.strict_add(size));
                            }
                            if acc_ty.is_writable() {
                                writes.push(addr..addr.strict_add(size));
                            }
                        }
                        _ => (),
                    }
                }
                arch::ArchOperand::Arm64Operand(arm64_operand) => {
                    // Annoyingly, we don't get the size here, so just be pessimistic for now
                    match arm64_operand.op_type {
                        arch::arm64::Arm64OperandType::Mem(_arm64_op_mem) => {
                            //
                        }
                        _ => (),
                    }
                }
                arch::ArchOperand::ArmOperand(arm_operand) =>
                    match arm_operand.op_type {
                        arch::arm::ArmOperandType::Mem(_) => {
                            let acc_ty = arm_operand.access.unwrap();
                            if acc_ty.is_readable() {
                                reads.push(addr..addr.strict_add(ARCH_MAX_ACCESS_SIZE));
                            }
                            if acc_ty.is_writable() {
                                writes.push(addr..addr.strict_add(ARCH_MAX_ACCESS_SIZE));
                            }
                        }
                        _ => (),
                    },
                arch::ArchOperand::RiscVOperand(_risc_voperand) => todo!(),
                _ => unimplemented!(),
            }
        }

        Ok(())
    }

    let siginfo = ptrace::getsiginfo(pid).unwrap();
    let addr = unsafe { siginfo.si_addr().addr().to_u64() };
    let page_addr = addr.strict_sub(addr.strict_rem(unsafe { PAGE_SIZE }));

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
        #[expect(clippy::as_conversions)]
        new_regs.set_ip(mempr_off as usize);
        // Don't mess up the stack by accident!
        new_regs.set_sp(unsafe { (&raw mut CLICK_HERE_4_FREE_STACK[512]).addr() });

        ptrace::write(pid, (&raw mut PAGE_ADDR).cast(), libc::c_long::try_from(page_addr).unwrap())
            .unwrap();
        ptrace::setregs(pid, new_regs).unwrap();

        wait_for_signal(pid, signal::SIGSTOP, true)?;

        // Step 1 instruction then reprotect memory
        ptrace::setregs(pid, regs_bak).unwrap();
        ptrace::step(pid, None).unwrap();
        // Don't use wait_for_signal here since 1 instruction doesn't give room
        // for any uncertainty + we don't want it `cont()`ing randomly
        let _ = wait::waitpid(pid, None).unwrap();

        let regs_bak = ptrace::getregs(pid).unwrap();
        new_regs = regs_bak;
        let ip_poststep = regs_bak.ip();
        let diff = (ip_poststep.strict_sub(ip_prestep)).div_ceil(8);
        let instr = (ip_prestep..ip_prestep.strict_add(diff)).fold(vec![], |mut ret, ip| {
            ret.append(
                &mut ptrace::read(pid, std::ptr::without_provenance_mut(ip))
                    .unwrap()
                    .to_ne_bytes()
                    .to_vec(),
            );
            ret
        });

        if capstone_disassemble(&instr, addr, cs, reads, writes).is_err() {
            reads.push(addr..addr.strict_add(ARCH_MAX_ACCESS_SIZE));
            writes.push(addr..addr.strict_add(ARCH_MAX_ACCESS_SIZE));
        }

        #[expect(clippy::as_conversions)]
        new_regs.set_ip(mempr_on as usize);
        new_regs.set_sp(unsafe { (&raw mut CLICK_HERE_4_FREE_STACK[512]).addr() });
        ptrace::setregs(pid, new_regs).unwrap();
        wait_for_signal(pid, signal::SIGSTOP, true)?;

        ptrace::setregs(pid, regs_bak).unwrap();
        ptrace::syscall(pid, None).unwrap();
    } else {
        let regs = ptrace::getregs(pid).unwrap();
        eprintln!("Segfault occurred during FFI at {addr:#018x}\nRegister dump: {regs:#x?}");
        ptrace::kill(pid).unwrap();
    }

    Ok(())
}

fn handle_sigtrap(
    pid: unistd::Pid,
    mappings: &mut Vec<Range<u64>>,
    malloc_bytes: i64,
    realloc_bytes: i64,
    free_bytes: i64,
) -> Result<(), i32> {
    // We can re-derive these pointers, no need to pass them in
    #[expect(clippy::as_conversions)]
    let malloc_addr = libc::malloc as usize;
    #[expect(clippy::as_conversions)]
    let realloc_addr = libc::realloc as usize;
    #[expect(clippy::as_conversions)]
    let free_addr = libc::free as usize;

    let regs = ptrace::getregs(pid).unwrap();
    match regs.ip().strict_sub(1) {
        a if a == malloc_addr => {
            let size = regs.arg1().to_u64(); // !
            if let Ok(ptr) = intercept_retptr(pid, regs, malloc_addr, malloc_bytes)?.try_into() {
                mappings.push(ptr..ptr.strict_add(size));
            }
        }
        a if a == realloc_addr => {
            let old_ptr = regs.arg1().to_u64();
            let size = regs.arg2().to_u64();
            let pos = mappings.iter().position(|rg| rg.start <= old_ptr && old_ptr < rg.end);
            if let Some(pos) = pos {
                let _ = mappings.remove(pos);
            }
            if let Ok(ptr) = intercept_retptr(pid, regs, realloc_addr, realloc_bytes)?.try_into() {
                mappings.push(ptr..ptr.strict_add(size));
            }
        }
        a if a == free_addr => {
            let old_ptr = regs.arg1().to_u64();
            let pos = mappings.iter().position(|rg| rg.start <= old_ptr && old_ptr < rg.end);
            if let Some(pos) = pos {
                let _ = mappings.remove(pos);
            }
            intercept_retptr(pid, regs, free_addr, free_bytes)?;
        }
        a => {
            eprintln!("Process got an unexpected SIGTRAP at addr {a:#018x?}; continuing...");
            ptrace::syscall(pid, None).unwrap();
        }
    }

    Ok(())
}

fn intercept_retptr(
    pid: unistd::Pid,
    mut regs: libc::user_regs_struct,
    fn_addr: usize,
    fn_bytes: i64,
) -> Result<isize, i32> {
    // Outline:
    // - Move instr ptr back before the sigtrap happened
    // - Restore the function to what it's supposed to be
    // - Change the function we're returning to so it gives us a sigtrap
    // - Catch it there
    // - Get the register-sized return value
    // - Patch the function back so it traps as before
    regs.set_ip(regs.ip().strict_sub(1));
    // Again, just need to keep the same bit pattern
    #[expect(clippy::as_conversions)]
    let ret_addr: usize =
        ptrace::read(pid, std::ptr::without_provenance_mut(regs.sp())).unwrap() as _;
    let ret_bytes = ptrace::read(pid, std::ptr::without_provenance_mut(ret_addr)).unwrap();

    ptrace::write(
        pid,
        std::ptr::without_provenance_mut(ret_addr),
        BREAKPT_INSTR.try_into().unwrap(),
    )
    .unwrap();
    // This one we did technically expose provenance for but it's in a different process anyways, so...
    ptrace::write(pid, std::ptr::without_provenance_mut(fn_addr), fn_bytes).unwrap();
    ptrace::setregs(pid, regs).unwrap();
    wait_for_signal(pid, signal::SIGTRAP, true)?;

    // now we're getting the return hopefully
    let mut regs = ptrace::getregs(pid).unwrap();
    #[expect(clippy::as_conversions)]
    let ptr = regs.retval() as isize; // !
    regs.set_ip(regs.ip().strict_sub(1));
    ptrace::write(
        pid,
        std::ptr::without_provenance_mut(fn_addr),
        BREAKPT_INSTR.try_into().unwrap(),
    )
    .unwrap();
    ptrace::write(pid, std::ptr::without_provenance_mut(ret_addr), ret_bytes).unwrap();
    ptrace::setregs(pid, regs).unwrap();

    ptrace::syscall(pid, None).unwrap();
    Ok(ptr)
}

// We only get dropped into these functions via offsetting the instr pointer
// manually, so we *must not ever* unwind from it
pub unsafe extern "C" fn mempr_off() {
    unsafe {
        if libc::mprotect(
            PAGE_ADDR,
            PAGE_SIZE.try_into().unwrap_unchecked(),
            libc::PROT_READ | libc::PROT_WRITE,
        ) != 0
        {
            std::process::exit(-20);
        }
        // This might error e.g. if the next page is unallocated or not owned by us - that's fine.
        // The point is just to allow for cross-page accesses, which may or may not happen
        //let _ = libc::mprotect(
        //    PAGE_ADDR.wrapping_add(PAGE_SIZE),
        //    PAGE_SIZE,
        //    libc::PROT_READ | libc::PROT_WRITE,
        //);
    }
    // If this fails somehow we're doomed
    if signal::raise(signal::SIGSTOP).is_err() {
        std::process::exit(-21);
    }
}

pub unsafe extern "C" fn mempr_on() {
    unsafe {
        if libc::mprotect(PAGE_ADDR, PAGE_SIZE.try_into().unwrap_unchecked(), libc::PROT_NONE) != 0
        {
            std::process::exit(-22);
        }
        //let _ = libc::mprotect(PAGE_ADDR.wrapping_add(PAGE_SIZE), PAGE_SIZE, libc::PROT_NONE);
    }
    if signal::raise(signal::SIGSTOP).is_err() {
        std::process::exit(-23);
    }
}
