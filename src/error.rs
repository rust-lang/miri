use std::error::Error;
use std::fmt;

#[derive(Clone, Debug)]
pub enum EvalError {
    DanglingPointerDeref,
    InvalidBool,
    InvalidDiscriminant,
    PointerOutOfBounds,
    ReadPointerAsBytes,
    ReadBytesAsPointer,
    InvalidPointerMath,
    ReadUndefBytes,
}

pub type EvalResult<T> = Result<T, EvalError>;

impl Error for EvalError {
    fn description(&self) -> &str {
        match *self {
            EvalError::DanglingPointerDeref => "dangling pointer was dereferenced",
            EvalError::InvalidBool => "invalid boolean value read",
            EvalError::InvalidDiscriminant => "invalid enum discriminant value read",
            EvalError::PointerOutOfBounds => "pointer offset outside bounds of allocation",
            EvalError::ReadPointerAsBytes => {
                "a raw memory access tried to access part of a pointer value as raw bytes"
            }
            EvalError::ReadBytesAsPointer => {
                "attempted to interpret some raw bytes as a pointer address"
            }
            EvalError::InvalidPointerMath => {
                "attempted to do math or a comparison on pointers into different allocations"
            }
            EvalError::ReadUndefBytes => "attempted to read undefined bytes",
        }
    }

    fn cause(&self) -> Option<&Error> {
        None
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}
