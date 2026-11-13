/* Native measurement boundaries for the checked construction core.
 * This file supplies only a monotonic timer and optimization barrier. Every data
 * operation remains emitted LLVM. No helper is called in ordinary programs unless
 * its measurement IR operation is present. Compile using the matched Clang tools.
 */
#define _POSIX_C_SOURCE 200809L
#define _DARWIN_C_SOURCE
#include <stdint.h>
#include <stdlib.h>
#include <time.h>

/* Return monotonic nanoseconds with no caller memory mutation. Invalid platform
 * results or arithmetic overflow terminate instead of fabricating measurements.
 * OS timing overhead is part of the harness and never a generated kernel claim.
 */
uint64_t holo_now(void) {
    struct timespec now;
    /* Validate the native boundary before converting signed OS values. */
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0 || now.tv_sec < 0 ||
        now.tv_nsec < 0 || now.tv_nsec >= 1000000000L ||
        (uint64_t)now.tv_sec > (UINT64_MAX - (uint64_t)now.tv_nsec) / UINT64_C(1000000000)) {
        abort();
    }
    return (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
}

/* Expose backing at phase boundaries so stores cannot vanish or move across it.
 * The pointer is not retained or dereferenced; the compiler-visible clobber models
 * observation. One call per lifecycle is disclosed, never one per element.
 */
void holo_opaque(void *backing) {
    __asm__ __volatile__("" : : "r"(backing) : "memory");
}
