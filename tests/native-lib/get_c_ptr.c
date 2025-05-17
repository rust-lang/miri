#include <stdint.h>
#include <stdlib.h>

// See comments in build_native_lib()
#define EXPORT __attribute__((visibility("default")))

EXPORT int64_t* get_c_ptr() {
    int64_t* retval = malloc(8);
    return retval;
}
