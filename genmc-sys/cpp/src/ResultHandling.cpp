#include "ResultHandling.hpp"

auto operator<<(llvm::raw_ostream& s, const GenmcScalar& v) -> llvm::raw_ostream& {
    if (v.is_init)
        return s << "{" << v.value << "}";
    else
        return s << "{UNINITIALIZED}";
}
