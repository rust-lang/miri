use libffi::low::CodePtr;
use libffi::middle::{Arg as ArgPtr, Cif, Type as FfiType};

/// Perform the actual FFI call.
///
/// SAFETY: The safety invariants of the foreign function being called must be
/// upheld (if any).
pub unsafe fn call<R: libffi::high::CType>(fun: CodePtr, args: &[OwnedArg]) -> R {
    let mut arg_tys = vec![];
    let mut arg_ptrs = vec![];
    for arg in args {
        arg_tys.push(arg.ty());
        arg_ptrs.push(arg.ptr())
    }
    let cif = Cif::new(arg_tys, R::reify().into_middle());
    // SAFETY: Caller upholds that the function is safe to call, and since we
    // were passed a slice reference we know the `OwnedArg`s won't have been
    // dropped by this point.
    unsafe { cif.call(fun, &arg_ptrs) }
}

/// An argument for an FFI call.
#[derive(Debug, Clone)]
pub enum OwnedArg {
    /// Primitive type.
    Primitive(ScalarArg),
    /// ADT with its computed type layout and bytes.
    Adt(FfiType, Box<[u8]>),
}

impl OwnedArg {
    /// Gets the libffi type descriptor for this argument.
    fn ty(&self) -> FfiType {
        match self {
            OwnedArg::Primitive(scalar_arg) => scalar_arg.ty(),
            OwnedArg::Adt(ty, _) => ty.clone(),
        }
    }

    /// Instantiates a libffi argument pointer pointing to this argument's bytes.
    /// NB: Since `libffi::middle::Arg` ignores the lifetime of the reference
    /// it's derived from, it is up to the caller to ensure the `OwnedArg` is
    /// not dropped before unsafely calling `libffi::middle::Cif::call()`!
    fn ptr(&self) -> ArgPtr {
        match self {
            OwnedArg::Primitive(scalar_arg) => scalar_arg.ptr(),
            // FIXME: Using `&items[0]` to reference the whole array is definitely
            // unsound under SB, but we're waiting on
            // https://github.com/libffi-rs/libffi-rs/commit/112a37b3b6ffb35bd75241fbcc580de40ba74a73
            // to land in a release so that we don't need to do this.
            OwnedArg::Adt(_, items) => ArgPtr::new(&items[0]),
        }
    }
}

impl From<ScalarArg> for OwnedArg {
    fn from(prim: ScalarArg) -> Self {
        Self::Primitive(prim)
    }
}

#[derive(Debug, Clone)]
/// Enum of supported scalar arguments to external C functions.
pub enum ScalarArg {
    /// 8-bit signed integer.
    Int8(i8),
    /// 16-bit signed integer.
    Int16(i16),
    /// 32-bit signed integer.
    Int32(i32),
    /// 64-bit signed integer.
    Int64(i64),
    /// isize.
    ISize(isize),
    /// 8-bit unsigned integer.
    UInt8(u8),
    /// 16-bit unsigned integer.
    UInt16(u16),
    /// 32-bit unsigned integer.
    UInt32(u32),
    /// 64-bit unsigned integer.
    UInt64(u64),
    /// usize.
    USize(usize),
    /// Raw pointer, stored as C's `void*`.
    RawPtr(*mut std::ffi::c_void),
}

impl ScalarArg {
    /// See `OwnedArg::ty()`.
    fn ty(&self) -> FfiType {
        match self {
            ScalarArg::Int8(_) => FfiType::i8(),
            ScalarArg::Int16(_) => FfiType::i16(),
            ScalarArg::Int32(_) => FfiType::i32(),
            ScalarArg::Int64(_) => FfiType::i64(),
            ScalarArg::ISize(_) => FfiType::isize(),
            ScalarArg::UInt8(_) => FfiType::u8(),
            ScalarArg::UInt16(_) => FfiType::u16(),
            ScalarArg::UInt32(_) => FfiType::u32(),
            ScalarArg::UInt64(_) => FfiType::u64(),
            ScalarArg::USize(_) => FfiType::usize(),
            ScalarArg::RawPtr(_) => FfiType::pointer(),
        }
    }

    /// See `OwnedArg::ptr()`.
    fn ptr(&self) -> ArgPtr {
        match self {
            ScalarArg::Int8(i) => ArgPtr::new(i),
            ScalarArg::Int16(i) => ArgPtr::new(i),
            ScalarArg::Int32(i) => ArgPtr::new(i),
            ScalarArg::Int64(i) => ArgPtr::new(i),
            ScalarArg::ISize(i) => ArgPtr::new(i),
            ScalarArg::UInt8(i) => ArgPtr::new(i),
            ScalarArg::UInt16(i) => ArgPtr::new(i),
            ScalarArg::UInt32(i) => ArgPtr::new(i),
            ScalarArg::UInt64(i) => ArgPtr::new(i),
            ScalarArg::USize(i) => ArgPtr::new(i),
            ScalarArg::RawPtr(i) => ArgPtr::new(i),
        }
    }
}
