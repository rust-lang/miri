#include <stddef.h>
#include <unistd.h>
#include <sys/mman.h>

// See comments in build_native_lib()
#define EXPORT __attribute__((visibility("default")))

/* Test: test_write_to_mapped */

EXPORT void* map_page(void) {
    size_t pg_size = (size_t)sysconf(_SC_PAGESIZE);
    return mmap(NULL, pg_size, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

EXPORT void unmap_page(void* pg) {
    size_t pg_size = (size_t)sysconf(_SC_PAGESIZE);
    munmap(pg, pg_size);
}
