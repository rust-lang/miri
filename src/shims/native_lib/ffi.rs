use libffi::low::CodePtr;
use libffi::middle::{Arg as ArgPtr, Cif, Type as FfiType};

/// Perform the actual FFI call.
///
/// SAFETY: The `FfiArg`s passed must have been correctly instantiated (i.e. their
/// type layout must match the data they point to), and the safety invariants of
/// the foreign function being called must be upheld (if any).
pub unsafe fn call<'a, R: libffi::high::CType>(fun: CodePtr, args: Vec<FfiArg<'a>>) -> R {
    let mut arg_tys = vec![];
    let mut arg_ptrs = vec![];
    for arg in args {
        arg_tys.push(arg.ty);
        arg_ptrs.push(arg.ptr)
    }
    let cif = Cif::new(arg_tys, R::reify().into_middle());
    unsafe { cif.call(fun, &arg_ptrs) }
}

/// A wrapper type for `libffi::middle::Type` which also holds a pointer to the data.
pub struct FfiArg<'a> {
    /// The type layout information for the pointed-to data.
    ty: FfiType,
    /// A pointer to the data described in `ty`.
    ptr: ArgPtr,
    /// Lifetime of the actual pointed-to data.
    _p: std::marker::PhantomData<&'a [u8]>,
}

impl<'a> FfiArg<'a> {
    fn new(ty: FfiType, ptr: ArgPtr) -> Self {
        Self { ty, ptr, _p: std::marker::PhantomData }
    }
}

/// An owning form of `FfiArg`.
/// We introduce this enum instead of just calling `Arg::new` and storing a list of
/// `libffi::middle::Arg` directly, because the `libffi::middle::Arg` just wraps a reference to
/// the value it represents and we need to store a copy of the value, and pass a reference to
/// this copy to C instead.
#[derive(Debug, Clone)]
pub enum CArg {
    /// Primitive type.
    Primitive(CPrimitive),
    /// Struct with its computed type layout and bytes.
    Struct(FfiType, Box<[u8]>),
}

impl CArg {
    /// Convert a `CArg` to the required FFI argument type.
    pub fn arg_downcast<'a>(&'a self) -> FfiArg<'a> {
        match self {
            CArg::Primitive(cprim) => cprim.arg_downcast(),
            // FIXME: Using `&items[0]` to reference the whole array is definitely
            // unsound under SB, but we're waiting on
            // https://github.com/libffi-rs/libffi-rs/commit/112a37b3b6ffb35bd75241fbcc580de40ba74a73
            // to land in a release so that we don't need to do this.
            CArg::Struct(cstruct, items) => FfiArg::new(cstruct.clone(), ArgPtr::new(&items[0])),
        }
    }
}

impl From<CPrimitive> for CArg {
    fn from(prim: CPrimitive) -> Self {
        Self::Primitive(prim)
    }
}

#[derive(Debug, Clone)]
/// Enum of supported primitive arguments to external C functions.
pub enum CPrimitive {
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

impl CPrimitive {
    /// Convert a primitive to the required FFI argument type.
    fn arg_downcast<'a>(&'a self) -> FfiArg<'a> {
        match self {
            CPrimitive::Int8(i) => FfiArg::new(FfiType::i8(), ArgPtr::new(i)),
            CPrimitive::Int16(i) => FfiArg::new(FfiType::i16(), ArgPtr::new(i)),
            CPrimitive::Int32(i) => FfiArg::new(FfiType::i32(), ArgPtr::new(i)),
            CPrimitive::Int64(i) => FfiArg::new(FfiType::i64(), ArgPtr::new(i)),
            CPrimitive::ISize(i) => FfiArg::new(FfiType::isize(), ArgPtr::new(i)),
            CPrimitive::UInt8(i) => FfiArg::new(FfiType::u8(), ArgPtr::new(i)),
            CPrimitive::UInt16(i) => FfiArg::new(FfiType::u16(), ArgPtr::new(i)),
            CPrimitive::UInt32(i) => FfiArg::new(FfiType::u32(), ArgPtr::new(i)),
            CPrimitive::UInt64(i) => FfiArg::new(FfiType::u64(), ArgPtr::new(i)),
            CPrimitive::USize(i) => FfiArg::new(FfiType::usize(), ArgPtr::new(i)),
            CPrimitive::RawPtr(i) => FfiArg::new(FfiType::pointer(), ArgPtr::new(i)),
        }
    }
}
