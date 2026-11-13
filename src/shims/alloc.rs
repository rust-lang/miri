use rustc_abi::{Align, AlignFromBytesError, Size};
use rustc_ast::expand::allocator::SpecialAllocatorMethod;
use rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrFlags;
use rustc_middle::mir::Local;
use rustc_middle::ty::Ty;
use rustc_middle::ty::layout::HasTyCtxt;
use rustc_span::Symbol;
use rustc_target::callconv::FnAbi;
use rustc_target::spec::{Arch, Os};

use crate::*;

/// Holds data for a custom allocator `alloc` frame.
#[derive(Debug)]
pub struct AllocFrameData {
    // Holds size of layout argument to `alloc` call.
    pub size: Size,
    // Holds alignment of layout argument to `alloc` call.
    pub align: Align,
}
/// Checks if a memory access to an allocation is blocked.
/// Needed at this indentation level to be called from memory hooks.
pub fn check_if_access_blocked<'tcx>(
    alloc_extra: &AllocExtra<'tcx>,
    range: &AllocRange,
) -> InterpResult<'tcx> {
    let mut blocked_ranges = alloc_extra.blocked_ranges.iter();
    if blocked_ranges
        .any(|blocked_range| range.end() > blocked_range.start && range.start < blocked_range.end())
    {
        panic!("Trying to access blocked range.");
    };
    interp_ok(())
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    /// Initialize `AllocFrameData` from current frame.
    /// Assumes current frame is a custom allocator `alloc` frame.
    fn init_alloc_frame_data(&self) -> InterpResult<'tcx, AllocFrameData> {
        let this = self.eval_context_ref();
        // We know the size is stored in the local with index 1,
        // and the alignment in the local with index 2.
        let size_scalar = this.read_scalar(&this.local_to_op(Local::from_usize(1usize), None)?)?;
        let align_scalar = this.read_scalar(&this.local_to_op(Local::from_usize(2usize), None)?)?;
        let size = Size::from_bytes(size_scalar.to_target_usize(this)?);
        let align = Align::from_bytes(align_scalar.to_target_usize(this)?).unwrap();
        interp_ok(AllocFrameData { size, align })
    }

    /// Handles Stack Pop of a custom allocator `alloc` frame.
    /// After the pop, we need to replace the pointer returned by the function call
    /// by a new pointer to a new allocation with guaranteed correct allocation bounds.
    /// We then store the original pointer in that allocation, so we can find it when it
    /// is time to deallocate.
    fn handle_alloc_frame_pop(
        &mut self,
        frame: &Frame<'tcx, Provenance, FrameExtra<'tcx>>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        // Get data of `alloc` call which is stored in the caller frame. We are calling this function after
        // the pop of the `alloc` frame and thus the caller frame is the current frame.
        let alloc_frame_data = this
            .frame()
            .extra
            .alloc_frame
            .as_ref()
            .expect("Caller frame should have a alloc_frame_data");
        // Move size and align args out of the Interpreter's context.
        let (size, align) = (alloc_frame_data.size, alloc_frame_data.align);
        let original_ptr = this.read_pointer(frame.return_place())?;
        // Need to block adress range given out in original allocation
        this.block_range_of_pointer(original_ptr, size)?;
        // Create new allocation, and write it into the return value
        let new_ptr =
            this.allocate_ptr(size, align, MiriMemoryKind::Rust.into(), AllocInit::Uninit)?;
        // Store original pointer in new allocation
        let (alloc_id, _, _) = this
            .ptr_try_get_alloc_id(new_ptr.into(), size.bytes().try_into().unwrap())
            .expect("Should not happen, was just allocated");
        let (alloc_extra_mut, _) = this.get_alloc_extra_mut(alloc_id)?;
        alloc_extra_mut.parent = Some(original_ptr);
        this.write_pointer(new_ptr, frame.return_place())?;
        interp_ok(())
    }

    /// Handles the initilization of the arguments in a call stack frame
    /// of a custom allocator `dealloc` frame.
    /// We assume that the pointer we receive comes from a `std::alloc::alloc` call
    /// and thus that allocation will have an original pointer attached. We also need to
    /// deallocate the allocation created in the `alloc` to prevent memory leaks and remove
    /// the blocked region from the original allocation.
    /// TODO: Set memory to uninitialized
    fn handle_init_dealloc_frame(&mut self) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let arg_ptr = this.read_pointer(&this.local_to_op(Local::from(1usize), None)?)?;
        let (arg_alloc_id, ..) =
            this.ptr_try_get_alloc_id(arg_ptr, 0).expect("Tried to deallocate a non_live pointer");
        let original_ptr = this
            .get_alloc_extra(arg_alloc_id)?
            .parent
            .expect("Tried to deallocate a pointer not allocated by the global allocator.");
        this.deallocate_ptr(arg_ptr, None, MiriMemoryKind::Rust.into())?;
        this.write_pointer(original_ptr, &this.local_to_place(Local::from(1usize))?)?;
        // Remove blocked range from original allocation
        let (original_alloc_id, offset, ..) = this
            .ptr_try_get_alloc_id(original_ptr, 0)
            .expect("The original allocation should still be live");
        let index = this
            .get_alloc_extra(original_alloc_id)?
            .blocked_ranges
            .iter()
            .position(|range| range.start == offset)
            .expect("Alloc should have added a range with this pointer's offset.");
        this.get_alloc_extra_mut(original_alloc_id)?.0.blocked_ranges.swap_remove(index);
        interp_ok(())
    }

    /// Blocks the range of the pointer argument in its allocation.
    fn block_range_of_pointer(&mut self, ptr: Pointer, size: Size) -> InterpResult<'tcx> {
        let this = self.eval_context_ref();
        let (alloc_id, offset, _) = this
            .ptr_try_get_alloc_id(ptr, size.bytes_usize().try_into().unwrap())
            .expect("Trying to block range of a pointer that is not live.");
        let range = AllocRange::from(offset..size);
        // First check if it is blocked, it shouldn't as an allocator should not hand out
        // same addresses twice.
        this.check_if_address_range_is_blocked(alloc_id, range)?;
        let (alloc_extra_mut, _) = self.eval_context_mut().get_alloc_extra_mut(alloc_id)?;
        alloc_extra_mut.blocked_ranges.push(range);
        interp_ok(())
    }

    /// Checks if given pointer is in a blocked range of its allocation.
    fn check_if_address_range_is_blocked(
        &self,
        alloc_id: AllocId,
        range: AllocRange,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_ref();
        check_if_access_blocked(this.get_alloc_extra(alloc_id)?, &range)
    }
    /// Checks if given frame is the stack frame of a custom global allocator `alloc` call.
    fn is_custom_alloc_frame(&self, frame: &Frame<'tcx, Provenance, FrameExtra<'tcx>>) -> bool {
        let this = self.eval_context_ref();
        // We use the functions codegen attributes to determine the type of the function.
        // We know that the ALLOCATOR flag is only set if and only if it is a custom global
        // allocator `alloc` call.
        let def_id = frame.instance().def_id();
        let flags = this.tcx().codegen_fn_attrs(def_id).flags;
        flags.contains(CodegenFnAttrFlags::ALLOCATOR)
    }
    /// Checks if current frame is the frame of a custom global allocator `alloc` call.
    fn has_custom_alloc_frame(&self) -> bool {
        let this = self.eval_context_ref();
        self.is_custom_alloc_frame(this.frame())
    }

    /// Checks if current frame is the frame of a custom global allocator `dealloc` call.
    fn has_custom_dealloc_frame(&self) -> bool {
        let this = self.eval_context_ref();
        // The DEALLOCATOR flag is only set if and only if it is a custom global allocator
        // `dealloc` call.
        let def_id = this.frame().instance().def_id();
        let flags = this.tcx().codegen_fn_attrs(def_id).flags;
        flags.contains(CodegenFnAttrFlags::DEALLOCATOR)
    }

    /// Returns the alignment that `malloc` would guarantee for requests of the given size.
    fn malloc_align(&self, size: u64) -> Align {
        let ecx = self.eval_context_ref();
        // The C standard says: "The pointer returned if the allocation succeeds is suitably aligned
        // so that it may be assigned to a pointer to any type of object with a fundamental
        // alignment requirement and size less than or equal to the size requested."
        // So first we need to figure out what the limits are for "fundamental alignment".
        // This is given by `alignof(max_align_t)`. The following list is taken from
        // `library/std/src/sys/alloc/mod.rs` (where this is called `MIN_ALIGN`) and should
        // be kept in sync.
        let os = &ecx.tcx.sess.target.os;
        let max_fundamental_align = match &ecx.tcx.sess.target.arch {
            Arch::RiscV32 if matches!(os, Os::EspIdf | Os::Zkvm) => 4,
            Arch::Xtensa if matches!(os, Os::EspIdf) => 4,
            Arch::X86
            | Arch::Arm
            | Arch::M68k
            | Arch::CSky
            | Arch::LoongArch32
            | Arch::Mips
            | Arch::Mips32r6
            | Arch::PowerPC
            | Arch::PowerPC64
            | Arch::Sparc
            | Arch::Wasm32
            | Arch::Hexagon
            | Arch::RiscV32
            | Arch::Xtensa => 8,
            Arch::X86_64
            | Arch::AArch64
            | Arch::Arm64EC
            | Arch::LoongArch64
            | Arch::Mips64
            | Arch::Mips64r6
            | Arch::S390x
            | Arch::Sparc64
            | Arch::RiscV64
            | Arch::Wasm64 => 16,
            arch @ (Arch::AmdGpu
            | Arch::Avr
            | Arch::Bpf
            | Arch::Msp430
            | Arch::Nvptx64
            | Arch::SpirV
            | Arch::Other(_)) => bug!("unsupported target architecture for malloc: `{arch}`"),
        };
        // The C standard only requires sufficient alignment for any *type* with size less than or
        // equal to the size requested. Types one can define in standard C seem to never have an alignment
        // bigger than their size. So if the size is 2, then only alignment 2 is guaranteed, even if
        // `max_fundamental_align` is bigger.
        // This matches what some real-world implementations do, see e.g.
        // - https://github.com/jemalloc/jemalloc/issues/1533
        // - https://github.com/llvm/llvm-project/issues/53540
        // - https://www.open-std.org/jtc1/sc22/wg14/www/docs/n2293.htm
        if size >= max_fundamental_align {
            return Align::from_bytes(max_fundamental_align).unwrap();
        }
        // C doesn't have zero-sized types, so presumably nothing is guaranteed here.
        if size == 0 {
            return Align::ONE;
        }
        // We have `size < min_align`. Round `size` *down* to the next power of two and use that.
        fn prev_power_of_two(x: u64) -> u64 {
            let next_pow2 = x.next_power_of_two();
            if next_pow2 == x {
                // x *is* a power of two, just use that.
                x
            } else {
                // x is between two powers, so next = 2*prev.
                next_pow2 / 2
            }
        }
        Align::from_bytes(prev_power_of_two(size)).unwrap()
    }

    /// Check some basic requirements for this allocation request:
    /// non-zero size, power-of-two alignment.
    fn check_rust_alloc_request(&self, size: u64, align: u64) -> InterpResult<'tcx> {
        let ecx = self.eval_context_ref();
        if size == 0 {
            throw_ub_format!("creating allocation with size 0");
        }
        if size > ecx.max_size_of_val().bytes() {
            throw_ub_format!("creating an allocation larger than half the address space");
        }
        if let Err(e) = Align::from_bytes(align) {
            match e {
                AlignFromBytesError::TooLarge(_) => {
                    throw_unsup_format!(
                        "creating allocation with alignment {align} exceeding rustc's maximum \
                         supported value"
                    );
                }
                AlignFromBytesError::NotPowerOfTwo(_) => {
                    throw_ub_format!("creating allocation with non-power-of-two alignment {align}");
                }
            }
        }

        interp_ok(())
    }

    fn rust_special_allocator_method(
        &mut self,
        method: SpecialAllocatorMethod,
        link_name: Symbol,
        abi: &FnAbi<'tcx, Ty<'tcx>>,
        args: &[OpTy<'tcx>],
        dest: &PlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let ecx = self.eval_context_mut();

        match method {
            SpecialAllocatorMethod::Alloc | SpecialAllocatorMethod::AllocZeroed => {
                let [size, align] = &ecx.check_shim_sig(
                    shim_sig!(extern "Rust" fn(usize, core::mem::Alignment) -> *()),
                    (link_name, abi, args),
                )?;
                let size = ecx.read_target_usize(size)?;
                let align = ecx.read_target_usize(align)?;

                ecx.check_rust_alloc_request(size, align)?;

                let ptr = ecx.allocate_ptr(
                    Size::from_bytes(size),
                    Align::from_bytes(align).unwrap(),
                    MiriMemoryKind::Rust.into(),
                    if matches!(method, SpecialAllocatorMethod::AllocZeroed) {
                        AllocInit::Zero
                    } else {
                        AllocInit::Uninit
                    },
                )?;

                ecx.write_pointer(ptr, dest)
            }
            SpecialAllocatorMethod::Dealloc => {
                let [ptr, old_size, align] = &ecx.check_shim_sig(
                    shim_sig!(extern "Rust" fn(*(), usize, core::mem::Alignment) -> ()),
                    (link_name, abi, args),
                )?;
                let ptr = ecx.read_pointer(ptr)?;
                let old_size = ecx.read_target_usize(old_size)?;
                let align = ecx.read_target_usize(align)?;

                // No need to check old_size/align; we anyway check that they match the allocation.
                ecx.deallocate_ptr(
                    ptr,
                    Some((Size::from_bytes(old_size), Align::from_bytes(align).unwrap())),
                    MiriMemoryKind::Rust.into(),
                )
            }
            SpecialAllocatorMethod::Realloc => {
                let [ptr, old_size, align, new_size] = &ecx.check_shim_sig(
                    shim_sig!(extern "Rust" fn(*(), usize, core::mem::Alignment, usize) -> *()),
                    (link_name, abi, args),
                )?;
                let ptr = ecx.read_pointer(ptr)?;
                let old_size = ecx.read_target_usize(old_size)?;
                let align = ecx.read_target_usize(align)?;
                let new_size = ecx.read_target_usize(new_size)?;
                // No need to check old_size; we anyway check that they match the allocation.

                ecx.check_rust_alloc_request(new_size, align)?;

                let align = Align::from_bytes(align).unwrap();
                let new_ptr = ecx.reallocate_ptr(
                    ptr,
                    Some((Size::from_bytes(old_size), align)),
                    Size::from_bytes(new_size),
                    align,
                    MiriMemoryKind::Rust.into(),
                    AllocInit::Uninit,
                )?;
                ecx.write_pointer(new_ptr, dest)
            }
        }
    }

    fn malloc(&mut self, size: u64, init: AllocInit) -> InterpResult<'tcx, Pointer> {
        let ecx = self.eval_context_mut();
        let align = ecx.malloc_align(size);
        let ptr =
            ecx.allocate_ptr(Size::from_bytes(size), align, MiriMemoryKind::C.into(), init)?;
        interp_ok(ptr.into())
    }

    fn posix_memalign(
        &mut self,
        memptr: &OpTy<'tcx>,
        align: &OpTy<'tcx>,
        size: &OpTy<'tcx>,
    ) -> InterpResult<'tcx, Scalar> {
        let ecx = self.eval_context_mut();
        let memptr = ecx.deref_pointer(memptr)?;
        let align = ecx.read_target_usize(align)?;
        let size = ecx.read_target_usize(size)?;

        // Align must be power of 2, and also at least ptr-sized (POSIX rules).
        // But failure to adhere to this is not UB, it's an error condition.
        if !align.is_power_of_two() || align < ecx.pointer_size().bytes() {
            interp_ok(ecx.eval_libc("EINVAL"))
        } else {
            let ptr = ecx.allocate_ptr(
                Size::from_bytes(size),
                Align::from_bytes(align).unwrap(),
                MiriMemoryKind::C.into(),
                AllocInit::Uninit,
            )?;
            ecx.write_pointer(ptr, &memptr)?;
            interp_ok(Scalar::from_i32(0))
        }
    }

    fn free(&mut self, ptr: Pointer) -> InterpResult<'tcx> {
        let ecx = self.eval_context_mut();
        if !ecx.ptr_is_null(ptr)? {
            ecx.deallocate_ptr(ptr, None, MiriMemoryKind::C.into())?;
        }
        interp_ok(())
    }

    fn realloc(&mut self, old_ptr: Pointer, new_size: u64) -> InterpResult<'tcx, Pointer> {
        let ecx = self.eval_context_mut();
        let new_align = ecx.malloc_align(new_size);
        if ecx.ptr_is_null(old_ptr)? {
            // Here we must behave like `malloc`.
            self.malloc(new_size, AllocInit::Uninit)
        } else {
            if new_size == 0 {
                // C, in their infinite wisdom, made this UB.
                // <https://www.open-std.org/jtc1/sc22/wg14/www/docs/n2464.pdf>
                throw_ub_format!("`realloc` with a size of zero");
            } else {
                let new_ptr = ecx.reallocate_ptr(
                    old_ptr,
                    None,
                    Size::from_bytes(new_size),
                    new_align,
                    MiriMemoryKind::C.into(),
                    AllocInit::Uninit,
                )?;
                interp_ok(new_ptr.into())
            }
        }
    }

    fn aligned_alloc(
        &mut self,
        align: &OpTy<'tcx>,
        size: &OpTy<'tcx>,
    ) -> InterpResult<'tcx, Pointer> {
        let ecx = self.eval_context_mut();
        let align = ecx.read_target_usize(align)?;
        let size = ecx.read_target_usize(size)?;

        // Alignment must be a power of 2, and "supported by the implementation".
        // We decide that "supported by the implementation" means that the
        // size must be a multiple of the alignment. (This restriction seems common
        // enough that it is stated on <https://en.cppreference.com/w/c/memory/aligned_alloc>
        // as a general rule, but the actual standard has no such rule.)
        // If any of these are violated, we have to return NULL.
        // All fundamental alignments must be supported.
        //
        // macOS and Illumos are buggy in that they require the alignment
        // to be at least the size of a pointer, so they do not support all fundamental
        // alignments. We do not emulate those platform bugs.
        //
        // Linux also sets errno to EINVAL, but that's non-standard behavior that we do not
        // emulate.
        // FreeBSD says some of these cases are UB but that's violating the C standard.
        // http://en.cppreference.com/w/cpp/memory/c/aligned_alloc
        // Linux: https://linux.die.net/man/3/aligned_alloc
        // FreeBSD: https://man.freebsd.org/cgi/man.cgi?query=aligned_alloc&apropos=0&sektion=3&manpath=FreeBSD+9-current&format=html
        match size.checked_rem(align) {
            Some(0) if align.is_power_of_two() => {
                let align = align.max(ecx.malloc_align(size).bytes());
                let ptr = ecx.allocate_ptr(
                    Size::from_bytes(size),
                    Align::from_bytes(align).unwrap(),
                    MiriMemoryKind::C.into(),
                    AllocInit::Uninit,
                )?;
                interp_ok(ptr.into())
            }
            _ => interp_ok(Pointer::null()),
        }
    }
}
