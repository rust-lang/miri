#include <stdint.h>
#include <stdlib.h>

// See comments in build_native_lib()
#define EXPORT __attribute__((visibility("default")))

/* Test: test_free_foreign_natively */

EXPORT void* allocate_bytes(int8_t count) {
    return malloc(count);
}

/* Test: test_free_native_foreignly */

EXPORT void free_ptr(void* ptr) {
    free(ptr);
}

/* Test: alloc_uninit */

EXPORT void write_byte_with_ofs(char* ptr, size_t ofs, int8_t byte) {
    *(ptr + ofs) = byte;
}
