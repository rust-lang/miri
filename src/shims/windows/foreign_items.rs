use std::iter;

use rustc_span::Symbol;
use rustc_target::abi::Size;
use rustc_target::spec::abi::Abi;

use crate::*;
use shims::foreign_items::EmulateByNameResult;
use shims::windows::handle::{EvalContextExt as _, Handle, PseudoHandle};
use shims::windows::sync::EvalContextExt as _;
use shims::windows::thread::EvalContextExt as _;

use smallvec::SmallVec;

impl<'mir, 'tcx: 'mir> EvalContextExt<'mir, 'tcx> for crate::MiriInterpCx<'mir, 'tcx> {}
pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriInterpCxExt<'mir, 'tcx> {
    fn emulate_foreign_item_by_name(
        &mut self,
        link_name: Symbol,
        abi: Abi,
        args: &[OpTy<'tcx, Provenance>],
        dest: &PlaceTy<'tcx, Provenance>,
    ) -> InterpResult<'tcx, EmulateByNameResult<'mir, 'tcx>> {
        let this = self.eval_context_mut();

        // See `fn emulate_foreign_item_by_name` in `shims/foreign_items.rs` for the general pattern.

        // Windows API stubs.
        // HANDLE = isize
        // NTSTATUS = LONH = i32
        // DWORD = ULONG = u32
        // BOOL = i32
        // BOOLEAN = u8
        match link_name.as_str() {
            // Environment related shims
            "GetEnvironmentVariableW" => {
                let [name, buf, size] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let result = this.GetEnvironmentVariableW(name, buf, size)?;
                this.write_scalar(Scalar::from_u32(result), dest)?;
            }
            "SetEnvironmentVariableW" => {
                let [name, value] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let result = this.SetEnvironmentVariableW(name, value)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }
            "GetEnvironmentStringsW" => {
                let [] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let result = this.GetEnvironmentStringsW()?;
                this.write_pointer(result, dest)?;
            }
            "FreeEnvironmentStringsW" => {
                let [env_block] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let result = this.FreeEnvironmentStringsW(env_block)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }
            "GetCurrentDirectoryW" => {
                let [size, buf] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let result = this.GetCurrentDirectoryW(size, buf)?;
                this.write_scalar(Scalar::from_u32(result), dest)?;
            }
            "SetCurrentDirectoryW" => {
                let [path] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let result = this.SetCurrentDirectoryW(path)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }

            // Allocation
            "HeapAlloc" => {
                let [handle, flags, size] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.read_scalar(handle)?.to_machine_isize(this)?;
                let flags = this.read_scalar(flags)?.to_u32()?;
                let size = this.read_scalar(size)?.to_machine_usize(this)?;
                let zero_init = (flags & 0x00000008) != 0; // HEAP_ZERO_MEMORY
                let res = this.malloc(size, zero_init, MiriMemoryKind::WinHeap)?;
                this.write_pointer(res, dest)?;
            }
            "HeapFree" => {
                let [handle, flags, ptr] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.read_scalar(handle)?.to_machine_isize(this)?;
                this.read_scalar(flags)?.to_u32()?;
                let ptr = this.read_pointer(ptr)?;
                this.free(ptr, MiriMemoryKind::WinHeap)?;
                this.write_scalar(Scalar::from_i32(1), dest)?;
            }
            "HeapReAlloc" => {
                let [handle, flags, ptr, size] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.read_scalar(handle)?.to_machine_isize(this)?;
                this.read_scalar(flags)?.to_u32()?;
                let ptr = this.read_pointer(ptr)?;
                let size = this.read_scalar(size)?.to_machine_usize(this)?;
                let res = this.realloc(ptr, size, MiriMemoryKind::WinHeap)?;
                this.write_pointer(res, dest)?;
            }

            // errno
            "SetLastError" => {
                let [error] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let error = this.read_scalar(error)?;
                this.set_last_error(error)?;
            }
            "GetLastError" => {
                let [] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let last_error = this.get_last_error()?;
                this.write_scalar(last_error, dest)?;
            }

            // Querying system information
            "GetSystemInfo" => {
                // Also called from `page_size` crate.
                let [system_info] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let system_info = this.deref_operand(system_info)?;
                // Initialize with `0`.
                this.write_bytes_ptr(
                    system_info.ptr,
                    iter::repeat(0u8).take(system_info.layout.size.bytes_usize()),
                )?;
                // Set selected fields.
                let word_layout = this.machine.layouts.u16;
                let dword_layout = this.machine.layouts.u32;
                let usize_layout = this.machine.layouts.usize;

                // Using `mplace_field` is error-prone, see: https://github.com/rust-lang/miri/issues/2136.
                // Pointer fields have different sizes on different targets.
                // To avoid all these issue we calculate the offsets ourselves.
                let field_sizes = [
                    word_layout.size,  // 0,  wProcessorArchitecture      : WORD
                    word_layout.size,  // 1,  wReserved                   : WORD
                    dword_layout.size, // 2,  dwPageSize                  : DWORD
                    usize_layout.size, // 3,  lpMinimumApplicationAddress : LPVOID
                    usize_layout.size, // 4,  lpMaximumApplicationAddress : LPVOID
                    usize_layout.size, // 5,  dwActiveProcessorMask       : DWORD_PTR
                    dword_layout.size, // 6,  dwNumberOfProcessors        : DWORD
                    dword_layout.size, // 7,  dwProcessorType             : DWORD
                    dword_layout.size, // 8,  dwAllocationGranularity     : DWORD
                    word_layout.size,  // 9,  wProcessorLevel             : WORD
                    word_layout.size,  // 10, wProcessorRevision          : WORD
                ];
                let field_offsets: SmallVec<[Size; 11]> = field_sizes
                    .iter()
                    .copied()
                    .scan(Size::ZERO, |a, x| {
                        let res = Some(*a);
                        *a += x;
                        res
                    })
                    .collect();

                // Set page size.
                let page_size = system_info.offset(field_offsets[2], dword_layout, &this.tcx)?;
                this.write_scalar(
                    Scalar::from_int(PAGE_SIZE, dword_layout.size),
                    &page_size.into(),
                )?;
                // Set number of processors.
                let num_cpus = system_info.offset(field_offsets[6], dword_layout, &this.tcx)?;
                this.write_scalar(Scalar::from_int(NUM_CPUS, dword_layout.size), &num_cpus.into())?;
            }

            // Thread-local storage
            "TlsAlloc" => {
                // This just creates a key; Windows does not natively support TLS destructors.

                // Create key and return it.
                let [] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let key = this.machine.tls.create_tls_key(None, dest.layout.size)?;
                this.write_scalar(Scalar::from_uint(key, dest.layout.size), dest)?;
            }
            "TlsGetValue" => {
                let [key] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let key = u128::from(this.read_scalar(key)?.to_u32()?);
                let active_thread = this.get_active_thread();
                let ptr = this.machine.tls.load_tls(key, active_thread, this)?;
                this.write_scalar(ptr, dest)?;
            }
            "TlsSetValue" => {
                let [key, new_ptr] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let key = u128::from(this.read_scalar(key)?.to_u32()?);
                let active_thread = this.get_active_thread();
                let new_data = this.read_scalar(new_ptr)?;
                this.machine.tls.store_tls(key, active_thread, new_data, &*this.tcx)?;

                // Return success (`1`).
                this.write_scalar(Scalar::from_i32(1), dest)?;
            }

            // Access to command-line arguments
            "GetCommandLineW" => {
                let [] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.write_pointer(
                    this.machine.cmd_line.expect("machine must be initialized").ptr,
                    dest,
                )?;
            }

            // Time related shims
            "GetSystemTimeAsFileTime" => {
                #[allow(non_snake_case)]
                let [LPFILETIME] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.GetSystemTimeAsFileTime(LPFILETIME)?;
            }
            "QueryPerformanceCounter" => {
                #[allow(non_snake_case)]
                let [lpPerformanceCount] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let result = this.QueryPerformanceCounter(lpPerformanceCount)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }
            "QueryPerformanceFrequency" => {
                #[allow(non_snake_case)]
                let [lpFrequency] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let result = this.QueryPerformanceFrequency(lpFrequency)?;
                this.write_scalar(Scalar::from_i32(result), dest)?;
            }
            "Sleep" => {
                let [timeout] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;

                this.Sleep(timeout)?;
            }

            // Synchronization primitives
            "AcquireSRWLockExclusive" => {
                let [ptr] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.AcquireSRWLockExclusive(ptr)?;
            }
            "ReleaseSRWLockExclusive" => {
                let [ptr] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.ReleaseSRWLockExclusive(ptr)?;
            }
            "TryAcquireSRWLockExclusive" => {
                let [ptr] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let ret = this.TryAcquireSRWLockExclusive(ptr)?;
                this.write_scalar(Scalar::from_u8(ret), dest)?;
            }
            "AcquireSRWLockShared" => {
                let [ptr] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.AcquireSRWLockShared(ptr)?;
            }
            "ReleaseSRWLockShared" => {
                let [ptr] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.ReleaseSRWLockShared(ptr)?;
            }
            "TryAcquireSRWLockShared" => {
                let [ptr] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let ret = this.TryAcquireSRWLockShared(ptr)?;
                this.write_scalar(Scalar::from_u8(ret), dest)?;
            }

            // Dynamic symbol loading
            "GetProcAddress" => {
                #[allow(non_snake_case)]
                let [hModule, lpProcName] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.read_scalar(hModule)?.to_machine_isize(this)?;
                let name = this.read_c_str(this.read_pointer(lpProcName)?)?;
                if let Some(dlsym) = Dlsym::from_str(name, &this.tcx.sess.target.os)? {
                    let ptr = this.create_fn_alloc_ptr(FnVal::Other(dlsym));
                    this.write_pointer(ptr, dest)?;
                } else {
                    this.write_null(dest)?;
                }
            }

            // Miscellaneous
            "SystemFunction036" => {
                // This is really 'RtlGenRandom'.
                let [ptr, len] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let ptr = this.read_pointer(ptr)?;
                let len = this.read_scalar(len)?.to_u32()?;
                this.gen_random(ptr, len.into())?;
                this.write_scalar(Scalar::from_bool(true), dest)?;
            }
            "BCryptGenRandom" => {
                let [algorithm, ptr, len, flags] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let algorithm = this.read_scalar(algorithm)?;
                let algorithm = algorithm.to_machine_usize(this)?;
                let ptr = this.read_pointer(ptr)?;
                let len = this.read_scalar(len)?.to_u32()?;
                let flags = this.read_scalar(flags)?.to_u32()?;
                match flags {
                    0 => {
                        if algorithm != 0x81 {
                            // BCRYPT_RNG_ALG_HANDLE
                            throw_unsup_format!(
                                "BCryptGenRandom algorithm must be BCRYPT_RNG_ALG_HANDLE when the flag is 0"
                            );
                        }
                    }
                    2 => {
                        // BCRYPT_USE_SYSTEM_PREFERRED_RNG
                        if algorithm != 0 {
                            throw_unsup_format!(
                                "BCryptGenRandom algorithm must be NULL when the flag is BCRYPT_USE_SYSTEM_PREFERRED_RNG"
                            );
                        }
                    }
                    _ => {
                        throw_unsup_format!(
                            "BCryptGenRandom is only supported with BCRYPT_USE_SYSTEM_PREFERRED_RNG or BCRYPT_RNG_ALG_HANDLE"
                        );
                    }
                }
                this.gen_random(ptr, len.into())?;
                this.write_null(dest)?; // STATUS_SUCCESS
            }
            "GetConsoleScreenBufferInfo" => {
                // `term` needs this, so we fake it.
                let [console, buffer_info] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.read_scalar(console)?.to_machine_isize(this)?;
                this.deref_operand(buffer_info)?;
                // Indicate an error.
                // FIXME: we should set last_error, but to what?
                this.write_null(dest)?;
            }
            "GetConsoleMode" => {
                // Windows "isatty" (in libtest) needs this, so we fake it.
                let [console, mode] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                this.read_scalar(console)?.to_machine_isize(this)?;
                this.deref_operand(mode)?;
                // Indicate an error.
                // FIXME: we should set last_error, but to what?
                this.write_null(dest)?;
            }
            "GetStdHandle" => {
                let [which] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let which = this.read_scalar(which)?.to_i32()?;
                // We just make this the identity function, so we know later in `NtWriteFile` which
                // one it is. This is very fake, but libtest needs it so we cannot make it a
                // std-only shim.
                // FIXME: this should return real HANDLEs when io support is added
                this.write_scalar(Scalar::from_machine_isize(which.into(), this), dest)?;
            }
            "CloseHandle" => {
                let [handle] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;

                this.CloseHandle(handle)?;

                this.write_scalar(Scalar::from_u32(1), dest)?;
            }

            // Threading
            "CreateThread" => {
                let [security, stacksize, start, arg, flags, thread] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;

                let thread_id =
                    this.CreateThread(security, stacksize, start, arg, flags, thread)?;

                this.write_scalar(Handle::Thread(thread_id).to_scalar(this), dest)?;
            }
            "WaitForSingleObject" => {
                let [handle, timeout] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;

                let ret = this.WaitForSingleObject(handle, timeout)?;
                this.write_scalar(Scalar::from_u32(ret), dest)?;
            }
            "GetCurrentThread" => {
                let [] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;

                this.write_scalar(
                    Handle::Pseudo(PseudoHandle::CurrentThread).to_scalar(this),
                    dest,
                )?;
            }

            // Incomplete shims that we "stub out" just to get pre-main initialization code to work.
            // These shims are enabled only when the caller is in the standard library.
            "GetProcessHeap" if this.frame_in_std() => {
                let [] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                // Just fake a HANDLE
                // It's fine to not use the Handle type here because its a stub
                this.write_scalar(Scalar::from_machine_isize(1, this), dest)?;
            }
            "GetModuleHandleA" if this.frame_in_std() => {
                #[allow(non_snake_case)]
                let [_lpModuleName] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                // We need to return something non-null here to make `compat_fn!` work.
                this.write_scalar(Scalar::from_machine_isize(1, this), dest)?;
            }
            "SetConsoleTextAttribute" if this.frame_in_std() => {
                #[allow(non_snake_case)]
                let [_hConsoleOutput, _wAttribute] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                // Pretend these does not exist / nothing happened, by returning zero.
                this.write_null(dest)?;
            }
            "AddVectoredExceptionHandler" if this.frame_in_std() => {
                #[allow(non_snake_case)]
                let [_First, _Handler] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                // Any non zero value works for the stdlib. This is just used for stack overflows anyway.
                this.write_scalar(Scalar::from_machine_usize(1, this), dest)?;
            }
            "SetThreadStackGuarantee" if this.frame_in_std() => {
                #[allow(non_snake_case)]
                let [_StackSizeInBytes] =
                    this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                // Any non zero value works for the stdlib. This is just used for stack overflows anyway.
                this.write_scalar(Scalar::from_u32(1), dest)?;
            }
            "GetCurrentProcessId" if this.frame_in_std() => {
                let [] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;
                let result = this.GetCurrentProcessId()?;
                this.write_scalar(Scalar::from_u32(result), dest)?;
            }
            // this is only callable from std because we know that std ignores the return value
            "SwitchToThread" if this.frame_in_std() => {
                let [] = this.check_shim(abi, Abi::System { unwind: false }, link_name, args)?;

                this.yield_active_thread();

                // FIXME: this should return a nonzero value if this call does result in switching to another thread.
                this.write_null(dest)?;
            }

            _ => return Ok(EmulateByNameResult::NotSupported),
        }

        Ok(EmulateByNameResult::NeedsJumping)
    }
}
