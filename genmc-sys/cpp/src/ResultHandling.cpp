#include "ResultHandling.hpp"

auto operator<<(llvm::raw_ostream &s, const GenmcScalar &v) -> llvm::raw_ostream &
{
	if (v.is_init) {
		s << "{" << v.value << ", " << v.extra << "}";
	} else {
		s << "{UNINITIALIZED}";
	}
	return s;
}
