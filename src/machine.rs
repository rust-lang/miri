//! Global machine state as well as implementation of the interpreter engine
//! `Machine` trait.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;

use rand::rngs::StdRng;
use rand::SeedableRng;

use rustc_ast::ast::Mutability;
use rustc_data_structures::fx::{FxHashMap, FxHashSet};
#[allow(unused)]
use rustc_data_structures::static_assert_size;
use rustc_middle::{
    mir,
    ty::{
        self,
        layout::{LayoutCx, LayoutError, LayoutOf, TyAndLayout},
        Instance, Ty, TyCtxt, TypeAndMut,
    },
};
use rustc_span::def_id::{CrateNum, DefId};
use rustc_span::Symbol;
use rustc_target::abi::Size;
use rustc_target::spec::abi::Abi;

use crate::{
    concurrency::{data_race, weak_memory},
    shims::unix::FileHandler,
    *,
};

// Some global facts about the emulated machine.
pub const PAGE_SIZE: u64 = 4 * 1024; // FIXME: adjust to target architecture
pub const STACK_ADDR: u64 = 32 * PAGE_SIZE; // not really about the "stack", but where we start assigning integer addresses to allocations
pub const STACK_SIZE: u64 = 16 * PAGE_SIZE; // whatever
pub const NUM_CPUS: u64 = 1;

/// Extra data stored with each stack frame
pub struct FrameData<'tcx> {
    /// Extra data for Stacked Borrows.
    pub stacked_borrows: Option<stacked_borrows::FrameExtra>,

    /// If this is Some(), then this is a special "catch unwind" frame (the frame of `try_fn`
    /// called by `try`). When this frame is popped during unwinding a panic,
    /// we stop unwinding, use the `CatchUnwindData` to handle catching.
    pub catch_unwind: Option<CatchUnwindData<'tcx>>,

    /// If `measureme` profiling is enabled, holds timing information
    /// for the start of this frame. When we finish executing this frame,
    /// we use this to register a completed event with `measureme`.
    pub timing: Option<measureme::DetachedTiming>,
}

impl<'tcx> std::fmt::Debug for FrameData<'tcx> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Omitting `timing`, it does not support `Debug`.
        let FrameData { stacked_borrows, catch_unwind, timing: _ } = self;
        f.debug_struct("FrameData")
            .field("stacked_borrows", stacked_borrows)
            .field("catch_unwind", catch_unwind)
            .finish()
    }
}

/// Extra memory kinds
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MiriMemoryKind {
    /// `__rust_alloc` memory.
    Rust,
    /// `malloc` memory.
    C,
    /// Windows `HeapAlloc` memory.
    WinHeap,
    /// Memory for args, errno, and other parts of the machine-managed environment.
    /// This memory may leak.
    Machine,
    /// Memory allocated by the runtime (e.g. env vars). Separate from `Machine`
    /// because we clean it up and leak-check it.
    Runtime,
    /// Globals copied from `tcx`.
    /// This memory may leak.
    Global,
    /// Memory for extern statics.
    /// This memory may leak.
    ExternStatic,
    /// Memory for thread-local statics.
    /// This memory may leak.
    Tls,
}

impl From<MiriMemoryKind> for MemoryKind<MiriMemoryKind> {
    #[inline(always)]
    fn from(kind: MiriMemoryKind) -> MemoryKind<MiriMemoryKind> {
        MemoryKind::Machine(kind)
    }
}

impl MayLeak for MiriMemoryKind {
    #[inline(always)]
    fn may_leak(self) -> bool {
        use self::MiriMemoryKind::*;
        match self {
            Rust | C | WinHeap | Runtime => false,
            Machine | Global | ExternStatic | Tls => true,
        }
    }
}

impl fmt::Display for MiriMemoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::MiriMemoryKind::*;
        match self {
            Rust => write!(f, "Rust heap"),
            C => write!(f, "C heap"),
            WinHeap => write!(f, "Windows heap"),
            Machine => write!(f, "machine-managed memory"),
            Runtime => write!(f, "language runtime memory"),
            Global => write!(f, "global (static or const)"),
            ExternStatic => write!(f, "extern static"),
            Tls => write!(f, "thread-local static"),
        }
    }
}

/// Pointer provenance.
#[derive(Debug, Clone, Copy)]
pub enum Provenance {
    Concrete {
        alloc_id: AllocId,
        /// Stacked Borrows tag.
        sb: SbTag,
    },
    Wildcard,
}

// This needs to be `Eq`+`Hash` because the `Machine` trait needs that because validity checking
// *might* be recursive and then it has to track which places have already been visited.
// However, comparing provenance is meaningless, since `Wildcard` might be any provenance -- and of
// course we don't actually do recursive checking.
// We could change `RefTracking` to strip provenance for its `seen` set but that type is generic so that is quite annoying.
// Instead owe add the required instances but make them panic.
impl PartialEq for Provenance {
    fn eq(&self, _other: &Self) -> bool {
        panic!("Provenance must not be compared")
    }
}
impl Eq for Provenance {}
impl std::hash::Hash for Provenance {
    fn hash<H: std::hash::Hasher>(&self, _state: &mut H) {
        panic!("Provenance must not be hashed")
    }
}

/// The "extra" information a pointer has over a regular AllocId.
#[derive(Copy, Clone, PartialEq)]
pub enum ProvenanceExtra {
    Concrete(SbTag),
    Wildcard,
}

#[cfg(all(target_arch = "x86_64", target_pointer_width = "64"))]
static_assert_size!(Pointer<Provenance>, 24);
// FIXME: this would with in 24bytes but layout optimizations are not smart enough
// #[cfg(all(target_arch = "x86_64", target_pointer_width = "64"))]
//static_assert_size!(Pointer<Option<Provenance>>, 24);
#[cfg(all(target_arch = "x86_64", target_pointer_width = "64"))]
static_assert_size!(Scalar<Provenance>, 32);

impl interpret::Provenance for Provenance {
    /// We use absolute addresses in the `offset` of a `Pointer<Provenance>`.
    const OFFSET_IS_ADDR: bool = true;

    /// We cannot err on partial overwrites, it happens too often in practice (due to unions).
    const ERR_ON_PARTIAL_PTR_OVERWRITE: bool = false;

    fn fmt(ptr: &Pointer<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (prov, addr) = ptr.into_parts(); // address is absolute
        write!(f, "{:#x}", addr.bytes())?;

        match prov {
            Provenance::Concrete { alloc_id, sb } => {
                // Forward `alternate` flag to `alloc_id` printing.
                if f.alternate() {
                    write!(f, "[{:#?}]", alloc_id)?;
                } else {
                    write!(f, "[{:?}]", alloc_id)?;
                }
                // Print Stacked Borrows tag.
                write!(f, "{:?}", sb)?;
            }
            Provenance::Wildcard => {
                write!(f, "[wildcard]")?;
            }
        }

        Ok(())
    }

    fn get_alloc_id(self) -> Option<AllocId> {
        match self {
            Provenance::Concrete { alloc_id, .. } => Some(alloc_id),
            Provenance::Wildcard => None,
        }
    }

    fn join(left: Option<Self>, right: Option<Self>) -> Option<Self> {
        match (left, right) {
            // If both are the *same* concrete tag, that is the result.
            (
                Some(Provenance::Concrete { alloc_id: left_alloc, sb: left_sb }),
                Some(Provenance::Concrete { alloc_id: right_alloc, sb: right_sb }),
            ) if left_alloc == right_alloc && left_sb == right_sb => left,
            // If one side is a wildcard, the best possible outcome is that it is equal to the other
            // one, and we use that.
            (Some(Provenance::Wildcard), o) | (o, Some(Provenance::Wildcard)) => o,
            // Otherwise, fall back to `None`.
            _ => None,
        }
    }
}

impl fmt::Debug for ProvenanceExtra {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProvenanceExtra::Concrete(pid) => write!(f, "{pid:?}"),
            ProvenanceExtra::Wildcard => write!(f, "<wildcard>"),
        }
    }
}

impl ProvenanceExtra {
    pub fn and_then<T>(self, f: impl FnOnce(SbTag) -> Option<T>) -> Option<T> {
        match self {
            ProvenanceExtra::Concrete(pid) => f(pid),
            ProvenanceExtra::Wildcard => None,
        }
    }
}

/// Extra per-allocation data
#[derive(Debug, Clone)]
pub struct AllocExtra {
    /// Stacked Borrows state is only added if it is enabled.
    pub stacked_borrows: Option<stacked_borrows::AllocExtra>,
    /// Data race detection via the use of a vector-clock,
    ///  this is only added if it is enabled.
    pub data_race: Option<data_race::AllocExtra>,
    /// Weak memory emulation via the use of store buffers,
    ///  this is only added if it is enabled.
    pub weak_memory: Option<weak_memory::AllocExtra>,
}

/// Precomputed layouts of primitive types
pub struct PrimitiveLayouts<'tcx> {
    pub unit: TyAndLayout<'tcx>,
    pub i8: TyAndLayout<'tcx>,
    pub i16: TyAndLayout<'tcx>,
    pub i32: TyAndLayout<'tcx>,
    pub isize: TyAndLayout<'tcx>,
    pub u8: TyAndLayout<'tcx>,
    pub u16: TyAndLayout<'tcx>,
    pub u32: TyAndLayout<'tcx>,
    pub usize: TyAndLayout<'tcx>,
    pub bool: TyAndLayout<'tcx>,
    pub mut_raw_ptr: TyAndLayout<'tcx>,   // *mut ()
    pub const_raw_ptr: TyAndLayout<'tcx>, // *const ()
}

impl<'mir, 'tcx: 'mir> PrimitiveLayouts<'tcx> {
    fn new(layout_cx: LayoutCx<'tcx, TyCtxt<'tcx>>) -> Result<Self, LayoutError<'tcx>> {
        let tcx = layout_cx.tcx;
        let mut_raw_ptr = tcx.mk_ptr(TypeAndMut { ty: tcx.types.unit, mutbl: Mutability::Mut });
        let const_raw_ptr = tcx.mk_ptr(TypeAndMut { ty: tcx.types.unit, mutbl: Mutability::Not });
        Ok(Self {
            unit: layout_cx.layout_of(tcx.mk_unit())?,
            i8: layout_cx.layout_of(tcx.types.i8)?,
            i16: layout_cx.layout_of(tcx.types.i16)?,
            i32: layout_cx.layout_of(tcx.types.i32)?,
            isize: layout_cx.layout_of(tcx.types.isize)?,
            u8: layout_cx.layout_of(tcx.types.u8)?,
            u16: layout_cx.layout_of(tcx.types.u16)?,
            u32: layout_cx.layout_of(tcx.types.u32)?,
            usize: layout_cx.layout_of(tcx.types.usize)?,
            bool: layout_cx.layout_of(tcx.types.bool)?,
            mut_raw_ptr: layout_cx.layout_of(mut_raw_ptr)?,
            const_raw_ptr: layout_cx.layout_of(const_raw_ptr)?,
        })
    }
}

/// The machine itself.
///
/// If you add anything here that stores machine values, remember to update
/// `visit_all_machine_values`!
pub struct MiriMachine<'mir, 'tcx> {
    // We carry a copy of the global `TyCtxt` for convenience, so methods taking just `&Evaluator` have `tcx` access.
    pub tcx: TyCtxt<'tcx>,

    /// Stacked Borrows global data.
    pub stacked_borrows: Option<stacked_borrows::GlobalState>,

    /// Data race detector global data.
    pub data_race: Option<data_race::GlobalState>,

    /// Ptr-int-cast module global data.
    pub intptrcast: intptrcast::GlobalState,

    /// Environment variables set by `setenv`.
    /// Miri does not expose env vars from the host to the emulated program.
    pub(crate) env_vars: EnvVars<'tcx>,

    /// Program arguments (`Option` because we can only initialize them after creating the ecx).
    /// These are *pointers* to argc/argv because macOS.
    /// We also need the full command line as one string because of Windows.
    pub(crate) argc: Option<MemPlace<Provenance>>,
    pub(crate) argv: Option<MemPlace<Provenance>>,
    pub(crate) cmd_line: Option<MemPlace<Provenance>>,

    /// TLS state.
    pub(crate) tls: TlsData<'tcx>,

    /// What should Miri do when an op requires communicating with the host,
    /// such as accessing host env vars, random number generation, and
    /// file system access.
    pub(crate) isolated_op: IsolatedOp,

    /// Whether to enforce the validity invariant.
    pub(crate) validate: bool,

    /// Whether to enforce [ABI](Abi) of function calls.
    pub(crate) enforce_abi: bool,

    /// The table of file descriptors.
    pub(crate) file_handler: shims::unix::FileHandler,
    /// The table of directory descriptors.
    pub(crate) dir_handler: shims::unix::DirHandler,

    /// This machine's monotone clock.
    pub(crate) clock: Clock,

    /// The set of threads.
    pub(crate) threads: ThreadManager<'mir, 'tcx>,

    /// Precomputed `TyLayout`s for primitive data types that are commonly used inside Miri.
    pub(crate) layouts: PrimitiveLayouts<'tcx>,

    /// Allocations that are considered roots of static memory (that may leak).
    pub(crate) static_roots: Vec<AllocId>,

    /// The `measureme` profiler used to record timing information about
    /// the emulated program.
    profiler: Option<measureme::Profiler>,
    /// Used with `profiler` to cache the `StringId`s for event names
    /// uesd with `measureme`.
    string_cache: FxHashMap<String, measureme::StringId>,

    /// Cache of `Instance` exported under the given `Symbol` name.
    /// `None` means no `Instance` exported under the given name is found.
    pub(crate) exported_symbols_cache: FxHashMap<Symbol, Option<Instance<'tcx>>>,

    /// Whether to raise a panic in the context of the evaluated process when unsupported
    /// functionality is encountered. If `false`, an error is propagated in the Miri application context
    /// instead (default behavior)
    pub(crate) panic_on_unsupported: bool,

    /// Equivalent setting as RUST_BACKTRACE on encountering an error.
    pub(crate) backtrace_style: BacktraceStyle,

    /// Crates which are considered local for the purposes of error reporting.
    pub(crate) local_crates: Vec<CrateNum>,

    /// Mapping extern static names to their base pointer.
    extern_statics: FxHashMap<Symbol, Pointer<Provenance>>,

    /// The random number generator used for resolving non-determinism.
    /// Needs to be queried by ptr_to_int, hence needs interior mutability.
    pub(crate) rng: RefCell<StdRng>,

    /// The allocation IDs to report when they are being allocated
    /// (helps for debugging memory leaks and use after free bugs).
    tracked_alloc_ids: FxHashSet<AllocId>,

    /// Controls whether alignment of memory accesses is being checked.
    pub(crate) check_alignment: AlignmentCheck,

    /// Failure rate of compare_exchange_weak, between 0.0 and 1.0
    pub(crate) cmpxchg_weak_failure_rate: f64,

    /// Corresponds to -Zmiri-mute-stdout-stderr and doesn't write the output but acts as if it succeeded.
    pub(crate) mute_stdout_stderr: bool,

    /// Whether weak memory emulation is enabled
    pub(crate) weak_memory: bool,

    /// The probability of the active thread being preempted at the end of each basic block.
    pub(crate) preemption_rate: f64,

    /// If `Some`, we will report the current stack every N basic blocks.
    pub(crate) report_progress: Option<u32>,
    // The total number of blocks that have been executed.
    pub(crate) basic_block_count: u64,

    /// Handle of the optional shared object file for external functions.
    #[cfg(unix)]
    pub external_so_lib: Option<(libloading::Library, std::path::PathBuf)>,

    /// Run a garbage collector for SbTags every N basic blocks.
    pub(crate) gc_interval: u32,
    /// The number of blocks that passed since the last SbTag GC pass.
    pub(crate) since_gc: u32,
}

impl<'mir, 'tcx> MiriMachine<'mir, 'tcx> {
    pub(crate) fn new(config: &MiriConfig, layout_cx: LayoutCx<'tcx, TyCtxt<'tcx>>) -> Self {
        let local_crates = helpers::get_local_crates(layout_cx.tcx);
        let layouts =
            PrimitiveLayouts::new(layout_cx).expect("Couldn't get layouts of primitive types");
        let profiler = config.measureme_out.as_ref().map(|out| {
            measureme::Profiler::new(out).expect("Couldn't create `measureme` profiler")
        });
        let rng = StdRng::seed_from_u64(config.seed.unwrap_or(0));
        let stacked_borrows = config.stacked_borrows.then(|| {
            RefCell::new(stacked_borrows::GlobalStateInner::new(
                config.tracked_pointer_tags.clone(),
                config.tracked_call_ids.clone(),
                config.retag_fields,
            ))
        });
        let data_race = config.data_race_detector.then(|| data_race::GlobalState::new(config));
        MiriMachine {
            tcx: layout_cx.tcx,
            stacked_borrows,
            data_race,
            intptrcast: RefCell::new(intptrcast::GlobalStateInner::new(config)),
            // `env_vars` depends on a full interpreter so we cannot properly initialize it yet.
            env_vars: EnvVars::default(),
            argc: None,
            argv: None,
            cmd_line: None,
            tls: TlsData::default(),
            isolated_op: config.isolated_op,
            validate: config.validate,
            enforce_abi: config.check_abi,
            file_handler: FileHandler::new(config.mute_stdout_stderr),
            dir_handler: Default::default(),
            layouts,
            threads: ThreadManager::default(),
            static_roots: Vec::new(),
            profiler,
            string_cache: Default::default(),
            exported_symbols_cache: FxHashMap::default(),
            panic_on_unsupported: config.panic_on_unsupported,
            backtrace_style: config.backtrace_style,
            local_crates,
            extern_statics: FxHashMap::default(),
            rng: RefCell::new(rng),
            tracked_alloc_ids: config.tracked_alloc_ids.clone(),
            check_alignment: config.check_alignment,
            cmpxchg_weak_failure_rate: config.cmpxchg_weak_failure_rate,
            mute_stdout_stderr: config.mute_stdout_stderr,
            weak_memory: config.weak_memory_emulation,
            preemption_rate: config.preemption_rate,
            report_progress: config.report_progress,
            basic_block_count: 0,
            clock: Clock::new(config.isolated_op == IsolatedOp::Allow),
            #[cfg(unix)]
            external_so_lib: config.external_so_file.as_ref().map(|lib_file_path| {
                let target_triple = layout_cx.tcx.sess.opts.target_triple.triple();
                // Check if host target == the session target.
                if env!("TARGET") != target_triple {
                    panic!(
                        "calling external C functions in linked .so file requires host and target to be the same: host={}, target={}",
                        env!("TARGET"),
                        target_triple,
                    );
                }
                // Note: it is the user's responsibility to provide a correct SO file.
                // WATCH OUT: If an invalid/incorrect SO file is specified, this can cause
                // undefined behaviour in Miri itself!
                (
                    unsafe {
                        libloading::Library::new(lib_file_path)
                            .expect("failed to read specified extern shared object file")
                    },
                    lib_file_path.clone(),
                )
            }),
            gc_interval: config.gc_interval,
            since_gc: 0,
        }
    }

    pub(crate) fn late_init(
        this: &mut MiriInterpCx<'mir, 'tcx>,
        config: &MiriConfig,
    ) -> InterpResult<'tcx> {
        EnvVars::init(this, config)?;
        MiriMachine::init_extern_statics(this)?;
        ThreadManager::init(this);
        Ok(())
    }

    fn add_extern_static(
        this: &mut MiriInterpCx<'mir, 'tcx>,
        name: &str,
        ptr: Pointer<Option<Provenance>>,
    ) {
        // This got just allocated, so there definitely is a pointer here.
        let ptr = ptr.into_pointer_or_addr().unwrap();
        this.machine.extern_statics.try_insert(Symbol::intern(name), ptr).unwrap();
    }

    fn alloc_extern_static(
        this: &mut MiriInterpCx<'mir, 'tcx>,
        name: &str,
        val: ImmTy<'tcx, Provenance>,
    ) -> InterpResult<'tcx> {
        let place = this.allocate(val.layout, MiriMemoryKind::ExternStatic.into())?;
        this.write_immediate(*val, &place.into())?;
        Self::add_extern_static(this, name, place.ptr);
        Ok(())
    }

    /// Sets up the "extern statics" for this machine.
    fn init_extern_statics(this: &mut MiriInterpCx<'mir, 'tcx>) -> InterpResult<'tcx> {
        match this.tcx.sess.target.os.as_ref() {
            "linux" => {
                // "environ"
                Self::add_extern_static(
                    this,
                    "environ",
                    this.machine.env_vars.environ.unwrap().ptr,
                );
                // A couple zero-initialized pointer-sized extern statics.
                // Most of them are for weak symbols, which we all set to null (indicating that the
                // symbol is not supported, and triggering fallback code which ends up calling a
                // syscall that we do support).
                for name in &["__cxa_thread_atexit_impl", "getrandom", "statx", "__clock_gettime64"]
                {
                    let val = ImmTy::from_int(0, this.machine.layouts.usize);
                    Self::alloc_extern_static(this, name, val)?;
                }
            }
            "freebsd" => {
                // "environ"
                Self::add_extern_static(
                    this,
                    "environ",
                    this.machine.env_vars.environ.unwrap().ptr,
                );
            }
            "android" => {
                // "signal"
                let layout = this.machine.layouts.const_raw_ptr;
                let dlsym = Dlsym::from_str("signal".as_bytes(), &this.tcx.sess.target.os)?
                    .expect("`signal` must be an actual dlsym on android");
                let ptr = this.create_fn_alloc_ptr(FnVal::Other(dlsym));
                let val = ImmTy::from_scalar(Scalar::from_pointer(ptr, this), layout);
                Self::alloc_extern_static(this, "signal", val)?;
                // A couple zero-initialized pointer-sized extern statics.
                // Most of them are for weak symbols, which we all set to null (indicating that the
                // symbol is not supported, and triggering fallback code.)
                for name in &["bsd_signal"] {
                    let val = ImmTy::from_int(0, this.machine.layouts.usize);
                    Self::alloc_extern_static(this, name, val)?;
                }
            }
            "windows" => {
                // "_tls_used"
                // This is some obscure hack that is part of the Windows TLS story. It's a `u8`.
                let val = ImmTy::from_int(0, this.machine.layouts.u8);
                Self::alloc_extern_static(this, "_tls_used", val)?;
            }
            _ => {} // No "extern statics" supported on this target
        }
        Ok(())
    }

    pub(crate) fn communicate(&self) -> bool {
        self.isolated_op == IsolatedOp::Allow
    }

    /// Check whether the stack frame that this `FrameInfo` refers to is part of a local crate.
    pub(crate) fn is_local(&self, frame: &FrameInfo<'_>) -> bool {
        let def_id = frame.instance.def_id();
        def_id.is_local() || self.local_crates.contains(&def_id.krate)
    }
}

impl VisitMachineValues for MiriMachine<'_, '_> {
    fn visit_machine_values(&self, visit: &mut impl FnMut(&Operand<Provenance>)) {
        // FIXME: visit the missing fields: env vars, weak mem, the MemPlace fields in the machine,
        // DirHandler, extern_statics, the Stacked Borrows base pointers; maybe more.
        let MiriMachine { threads, tls, .. } = self;

        threads.visit_machine_values(visit);
        tls.visit_machine_values(visit);
    }
}

/// A rustc InterpCx for Miri.
pub type MiriInterpCx<'mir, 'tcx> = InterpCx<'mir, 'tcx, MiriMachine<'mir, 'tcx>>;

/// A little trait that's useful to be inherited by extension traits.
pub trait MiriInterpCxExt<'mir, 'tcx> {
    fn eval_context_ref<'a>(&'a self) -> &'a MiriInterpCx<'mir, 'tcx>;
    fn eval_context_mut<'a>(&'a mut self) -> &'a mut MiriInterpCx<'mir, 'tcx>;
}
impl<'mir, 'tcx> MiriInterpCxExt<'mir, 'tcx> for MiriInterpCx<'mir, 'tcx> {
    #[inline(always)]
    fn eval_context_ref(&self) -> &MiriInterpCx<'mir, 'tcx> {
        self
    }
    #[inline(always)]
    fn eval_context_mut(&mut self) -> &mut MiriInterpCx<'mir, 'tcx> {
        self
    }
}

/// Machine hook implementations.
impl<'mir, 'tcx> Machine<'mir, 'tcx> for MiriMachine<'mir, 'tcx> {
    type MemoryKind = MiriMemoryKind;
    type ExtraFnVal = Dlsym;

    type FrameExtra = FrameData<'tcx>;
    type AllocExtra = AllocExtra;

    type Provenance = Provenance;
    type ProvenanceExtra = ProvenanceExtra;

    type MemoryMap = MonoHashMap<
        AllocId,
        (MemoryKind<MiriMemoryKind>, Allocation<Provenance, Self::AllocExtra>),
    >;

    const GLOBAL_KIND: Option<MiriMemoryKind> = Some(MiriMemoryKind::Global);

    const PANIC_ON_ALLOC_FAIL: bool = false;

    #[inline(always)]
    fn enforce_alignment(ecx: &MiriInterpCx<'mir, 'tcx>) -> bool {
        ecx.machine.check_alignment != AlignmentCheck::None
    }

    #[inline(always)]
    fn use_addr_for_alignment_check(ecx: &MiriInterpCx<'mir, 'tcx>) -> bool {
        ecx.machine.check_alignment == AlignmentCheck::Int
    }

    #[inline(always)]
    fn enforce_validity(ecx: &MiriInterpCx<'mir, 'tcx>) -> bool {
        ecx.machine.validate
    }

    #[inline(always)]
    fn enforce_abi(ecx: &MiriInterpCx<'mir, 'tcx>) -> bool {
        ecx.machine.enforce_abi
    }

    #[inline(always)]
    fn checked_binop_checks_overflow(ecx: &MiriInterpCx<'mir, 'tcx>) -> bool {
        ecx.tcx.sess.overflow_checks()
    }

    #[inline(always)]
    fn find_mir_or_eval_fn(
        ecx: &mut MiriInterpCx<'mir, 'tcx>,
        instance: ty::Instance<'tcx>,
        abi: Abi,
        args: &[OpTy<'tcx, Provenance>],
        dest: &PlaceTy<'tcx, Provenance>,
        ret: Option<mir::BasicBlock>,
        unwind: StackPopUnwind,
    ) -> InterpResult<'tcx, Option<(&'mir mir::Body<'tcx>, ty::Instance<'tcx>)>> {
        ecx.find_mir_or_eval_fn(instance, abi, args, dest, ret, unwind)
    }

    #[inline(always)]
    fn call_extra_fn(
        ecx: &mut MiriInterpCx<'mir, 'tcx>,
        fn_val: Dlsym,
        abi: Abi,
        args: &[OpTy<'tcx, Provenance>],
        dest: &PlaceTy<'tcx, Provenance>,
        ret: Option<mir::BasicBlock>,
        _unwind: StackPopUnwind,
    ) -> InterpResult<'tcx> {
        ecx.call_dlsym(fn_val, abi, args, dest, ret)
    }

    #[inline(always)]
    fn call_intrinsic(
        ecx: &mut MiriInterpCx<'mir, 'tcx>,
        instance: ty::Instance<'tcx>,
        args: &[OpTy<'tcx, Provenance>],
        dest: &PlaceTy<'tcx, Provenance>,
        ret: Option<mir::BasicBlock>,
        unwind: StackPopUnwind,
    ) -> InterpResult<'tcx> {
        ecx.call_intrinsic(instance, args, dest, ret, unwind)
    }

    #[inline(always)]
    fn assert_panic(
        ecx: &mut MiriInterpCx<'mir, 'tcx>,
        msg: &mir::AssertMessage<'tcx>,
        unwind: Option<mir::BasicBlock>,
    ) -> InterpResult<'tcx> {
        ecx.assert_panic(msg, unwind)
    }

    #[inline(always)]
    fn abort(_ecx: &mut MiriInterpCx<'mir, 'tcx>, msg: String) -> InterpResult<'tcx, !> {
        throw_machine_stop!(TerminationInfo::Abort(msg))
    }

    #[inline(always)]
    fn binary_ptr_op(
        ecx: &MiriInterpCx<'mir, 'tcx>,
        bin_op: mir::BinOp,
        left: &ImmTy<'tcx, Provenance>,
        right: &ImmTy<'tcx, Provenance>,
    ) -> InterpResult<'tcx, (Scalar<Provenance>, bool, Ty<'tcx>)> {
        ecx.binary_ptr_op(bin_op, left, right)
    }

    fn thread_local_static_base_pointer(
        ecx: &mut MiriInterpCx<'mir, 'tcx>,
        def_id: DefId,
    ) -> InterpResult<'tcx, Pointer<Provenance>> {
        ecx.get_or_create_thread_local_alloc(def_id)
    }

    fn extern_static_base_pointer(
        ecx: &MiriInterpCx<'mir, 'tcx>,
        def_id: DefId,
    ) -> InterpResult<'tcx, Pointer<Provenance>> {
        let link_name = ecx.item_link_name(def_id);
        if let Some(&ptr) = ecx.machine.extern_statics.get(&link_name) {
            // Various parts of the engine rely on `get_alloc_info` for size and alignment
            // information. That uses the type information of this static.
            // Make sure it matches the Miri allocation for this.
            let Provenance::Concrete { alloc_id, .. } = ptr.provenance else {
                panic!("extern_statics cannot contain wildcards")
            };
            let (shim_size, shim_align, _kind) = ecx.get_alloc_info(alloc_id);
            let extern_decl_layout =
                ecx.tcx.layout_of(ty::ParamEnv::empty().and(ecx.tcx.type_of(def_id))).unwrap();
            if extern_decl_layout.size != shim_size || extern_decl_layout.align.abi != shim_align {
                throw_unsup_format!(
                    "`extern` static `{name}` from crate `{krate}` has been declared \
                    with a size of {decl_size} bytes and alignment of {decl_align} bytes, \
                    but Miri emulates it via an extern static shim \
                    with a size of {shim_size} bytes and alignment of {shim_align} bytes",
                    name = ecx.tcx.def_path_str(def_id),
                    krate = ecx.tcx.crate_name(def_id.krate),
                    decl_size = extern_decl_layout.size.bytes(),
                    decl_align = extern_decl_layout.align.abi.bytes(),
                    shim_size = shim_size.bytes(),
                    shim_align = shim_align.bytes(),
                )
            }
            Ok(ptr)
        } else {
            throw_unsup_format!(
                "`extern` static `{name}` from crate `{krate}` is not supported by Miri",
                name = ecx.tcx.def_path_str(def_id),
                krate = ecx.tcx.crate_name(def_id.krate),
            )
        }
    }

    fn adjust_allocation<'b>(
        ecx: &MiriInterpCx<'mir, 'tcx>,
        id: AllocId,
        alloc: Cow<'b, Allocation>,
        kind: Option<MemoryKind<Self::MemoryKind>>,
    ) -> InterpResult<'tcx, Cow<'b, Allocation<Self::Provenance, Self::AllocExtra>>> {
        let kind = kind.expect("we set our STATIC_KIND so this cannot be None");
        if ecx.machine.tracked_alloc_ids.contains(&id) {
            ecx.emit_diagnostic(NonHaltingDiagnostic::CreatedAlloc(
                id,
                alloc.size(),
                alloc.align,
                kind,
            ));
        }

        let alloc = alloc.into_owned();
        let stacks = ecx.machine.stacked_borrows.as_ref().map(|stacked_borrows| {
            Stacks::new_allocation(
                id,
                alloc.size(),
                stacked_borrows,
                kind,
                ecx.machine.current_span(),
            )
        });
        let race_alloc = ecx.machine.data_race.as_ref().map(|data_race| {
            data_race::AllocExtra::new_allocation(
                data_race,
                &ecx.machine.threads,
                alloc.size(),
                kind,
            )
        });
        let buffer_alloc = ecx.machine.weak_memory.then(weak_memory::AllocExtra::new_allocation);
        let alloc: Allocation<Provenance, Self::AllocExtra> = alloc.adjust_from_tcx(
            &ecx.tcx,
            AllocExtra {
                stacked_borrows: stacks.map(RefCell::new),
                data_race: race_alloc,
                weak_memory: buffer_alloc,
            },
            |ptr| ecx.global_base_pointer(ptr),
        )?;
        Ok(Cow::Owned(alloc))
    }

    fn adjust_alloc_base_pointer(
        ecx: &MiriInterpCx<'mir, 'tcx>,
        ptr: Pointer<AllocId>,
    ) -> Pointer<Provenance> {
        if cfg!(debug_assertions) {
            // The machine promises to never call us on thread-local or extern statics.
            let alloc_id = ptr.provenance;
            match ecx.tcx.try_get_global_alloc(alloc_id) {
                Some(GlobalAlloc::Static(def_id)) if ecx.tcx.is_thread_local_static(def_id) => {
                    panic!("adjust_alloc_base_pointer called on thread-local static")
                }
                Some(GlobalAlloc::Static(def_id)) if ecx.tcx.is_foreign_item(def_id) => {
                    panic!("adjust_alloc_base_pointer called on extern static")
                }
                _ => {}
            }
        }
        let absolute_addr = intptrcast::GlobalStateInner::rel_ptr_to_addr(ecx, ptr);
        let sb_tag = if let Some(stacked_borrows) = &ecx.machine.stacked_borrows {
            stacked_borrows.borrow_mut().base_ptr_tag(ptr.provenance, &ecx.machine)
        } else {
            // Value does not matter, SB is disabled
            SbTag::default()
        };
        Pointer::new(
            Provenance::Concrete { alloc_id: ptr.provenance, sb: sb_tag },
            Size::from_bytes(absolute_addr),
        )
    }

    #[inline(always)]
    fn ptr_from_addr_cast(
        ecx: &MiriInterpCx<'mir, 'tcx>,
        addr: u64,
    ) -> InterpResult<'tcx, Pointer<Option<Self::Provenance>>> {
        intptrcast::GlobalStateInner::ptr_from_addr_cast(ecx, addr)
    }

    fn expose_ptr(
        ecx: &mut InterpCx<'mir, 'tcx, Self>,
        ptr: Pointer<Self::Provenance>,
    ) -> InterpResult<'tcx> {
        match ptr.provenance {
            Provenance::Concrete { alloc_id, sb } =>
                intptrcast::GlobalStateInner::expose_ptr(ecx, alloc_id, sb),
            Provenance::Wildcard => {
                // No need to do anything for wildcard pointers as
                // their provenances have already been previously exposed.
                Ok(())
            }
        }
    }

    /// Convert a pointer with provenance into an allocation-offset pair,
    /// or a `None` with an absolute address if that conversion is not possible.
    fn ptr_get_alloc(
        ecx: &MiriInterpCx<'mir, 'tcx>,
        ptr: Pointer<Self::Provenance>,
    ) -> Option<(AllocId, Size, Self::ProvenanceExtra)> {
        let rel = intptrcast::GlobalStateInner::abs_ptr_to_rel(ecx, ptr);

        rel.map(|(alloc_id, size)| {
            let sb = match ptr.provenance {
                Provenance::Concrete { sb, .. } => ProvenanceExtra::Concrete(sb),
                Provenance::Wildcard => ProvenanceExtra::Wildcard,
            };
            (alloc_id, size, sb)
        })
    }

    #[inline(always)]
    fn before_memory_read(
        _tcx: TyCtxt<'tcx>,
        machine: &Self,
        alloc_extra: &AllocExtra,
        (alloc_id, prov_extra): (AllocId, Self::ProvenanceExtra),
        range: AllocRange,
    ) -> InterpResult<'tcx> {
        if let Some(data_race) = &alloc_extra.data_race {
            data_race.read(
                alloc_id,
                range,
                machine.data_race.as_ref().unwrap(),
                &machine.threads,
            )?;
        }
        if let Some(stacked_borrows) = &alloc_extra.stacked_borrows {
            stacked_borrows.borrow_mut().before_memory_read(
                alloc_id,
                prov_extra,
                range,
                machine.stacked_borrows.as_ref().unwrap(),
                machine.current_span(),
                &machine.threads,
            )?;
        }
        if let Some(weak_memory) = &alloc_extra.weak_memory {
            weak_memory.memory_accessed(range, machine.data_race.as_ref().unwrap());
        }
        Ok(())
    }

    #[inline(always)]
    fn before_memory_write(
        _tcx: TyCtxt<'tcx>,
        machine: &mut Self,
        alloc_extra: &mut AllocExtra,
        (alloc_id, prov_extra): (AllocId, Self::ProvenanceExtra),
        range: AllocRange,
    ) -> InterpResult<'tcx> {
        if let Some(data_race) = &mut alloc_extra.data_race {
            data_race.write(
                alloc_id,
                range,
                machine.data_race.as_mut().unwrap(),
                &machine.threads,
            )?;
        }
        if let Some(stacked_borrows) = &mut alloc_extra.stacked_borrows {
            stacked_borrows.get_mut().before_memory_write(
                alloc_id,
                prov_extra,
                range,
                machine.stacked_borrows.as_ref().unwrap(),
                machine.current_span(),
                &machine.threads,
            )?;
        }
        if let Some(weak_memory) = &alloc_extra.weak_memory {
            weak_memory.memory_accessed(range, machine.data_race.as_ref().unwrap());
        }
        Ok(())
    }

    #[inline(always)]
    fn before_memory_deallocation(
        _tcx: TyCtxt<'tcx>,
        machine: &mut Self,
        alloc_extra: &mut AllocExtra,
        (alloc_id, prove_extra): (AllocId, Self::ProvenanceExtra),
        range: AllocRange,
    ) -> InterpResult<'tcx> {
        if machine.tracked_alloc_ids.contains(&alloc_id) {
            machine.emit_diagnostic(NonHaltingDiagnostic::FreedAlloc(alloc_id));
        }
        if let Some(data_race) = &mut alloc_extra.data_race {
            data_race.deallocate(
                alloc_id,
                range,
                machine.data_race.as_mut().unwrap(),
                &machine.threads,
            )?;
        }
        if let Some(stacked_borrows) = &mut alloc_extra.stacked_borrows {
            stacked_borrows.get_mut().before_memory_deallocation(
                alloc_id,
                prove_extra,
                range,
                machine.stacked_borrows.as_ref().unwrap(),
                machine.current_span(),
                &machine.threads,
            )
        } else {
            Ok(())
        }
    }

    #[inline(always)]
    fn retag(
        ecx: &mut InterpCx<'mir, 'tcx, Self>,
        kind: mir::RetagKind,
        place: &PlaceTy<'tcx, Provenance>,
    ) -> InterpResult<'tcx> {
        if ecx.machine.stacked_borrows.is_some() { ecx.retag(kind, place) } else { Ok(()) }
    }

    #[inline(always)]
    fn init_frame_extra(
        ecx: &mut InterpCx<'mir, 'tcx, Self>,
        frame: Frame<'mir, 'tcx, Provenance>,
    ) -> InterpResult<'tcx, Frame<'mir, 'tcx, Provenance, FrameData<'tcx>>> {
        // Start recording our event before doing anything else
        let timing = if let Some(profiler) = ecx.machine.profiler.as_ref() {
            let fn_name = frame.instance.to_string();
            let entry = ecx.machine.string_cache.entry(fn_name.clone());
            let name = entry.or_insert_with(|| profiler.alloc_string(&*fn_name));

            Some(profiler.start_recording_interval_event_detached(
                *name,
                measureme::EventId::from_label(*name),
                ecx.get_active_thread().to_u32(),
            ))
        } else {
            None
        };

        let stacked_borrows = ecx.machine.stacked_borrows.as_ref();

        let extra = FrameData {
            stacked_borrows: stacked_borrows.map(|sb| sb.borrow_mut().new_frame(&ecx.machine)),
            catch_unwind: None,
            timing,
        };
        Ok(frame.with_extra(extra))
    }

    fn stack<'a>(
        ecx: &'a InterpCx<'mir, 'tcx, Self>,
    ) -> &'a [Frame<'mir, 'tcx, Self::Provenance, Self::FrameExtra>] {
        ecx.active_thread_stack()
    }

    fn stack_mut<'a>(
        ecx: &'a mut InterpCx<'mir, 'tcx, Self>,
    ) -> &'a mut Vec<Frame<'mir, 'tcx, Self::Provenance, Self::FrameExtra>> {
        ecx.active_thread_stack_mut()
    }

    fn before_terminator(ecx: &mut InterpCx<'mir, 'tcx, Self>) -> InterpResult<'tcx> {
        ecx.machine.basic_block_count += 1u64; // a u64 that is only incremented by 1 will "never" overflow
        ecx.machine.since_gc += 1;
        // Possibly report our progress.
        if let Some(report_progress) = ecx.machine.report_progress {
            if ecx.machine.basic_block_count % u64::from(report_progress) == 0 {
                ecx.emit_diagnostic(NonHaltingDiagnostic::ProgressReport {
                    block_count: ecx.machine.basic_block_count,
                });
            }
        }

        // Search for SbTags to find all live pointers, then remove all other tags from borrow
        // stacks.
        // When debug assertions are enabled, run the GC as often as possible so that any cases
        // where it mistakenly removes an important tag become visible.
        if ecx.machine.gc_interval > 0 && ecx.machine.since_gc >= ecx.machine.gc_interval {
            ecx.machine.since_gc = 0;
            ecx.garbage_collect_tags()?;
        }

        // These are our preemption points.
        ecx.maybe_preempt_active_thread();

        // Make sure some time passes.
        ecx.machine.clock.tick();

        Ok(())
    }

    #[inline(always)]
    fn after_stack_push(ecx: &mut InterpCx<'mir, 'tcx, Self>) -> InterpResult<'tcx> {
        if ecx.machine.stacked_borrows.is_some() { ecx.retag_return_place() } else { Ok(()) }
    }

    #[inline(always)]
    fn after_stack_pop(
        ecx: &mut InterpCx<'mir, 'tcx, Self>,
        mut frame: Frame<'mir, 'tcx, Provenance, FrameData<'tcx>>,
        unwinding: bool,
    ) -> InterpResult<'tcx, StackPopJump> {
        let timing = frame.extra.timing.take();
        if let Some(stacked_borrows) = &ecx.machine.stacked_borrows {
            stacked_borrows.borrow_mut().end_call(&frame.extra);
        }
        let res = ecx.handle_stack_pop_unwind(frame.extra, unwinding);
        if let Some(profiler) = ecx.machine.profiler.as_ref() {
            profiler.finish_recording_interval_event(timing.unwrap());
        }
        res
    }
}
