/*
 * Process entry for the experimental hosted input profile. This host opens
 * or adopts exactly one descriptor, admits it, then lends it to checked Holo
 * until entry returns. Descriptor setup and cleanup run outside the data loop.
 */

/* Expose close-on-exec and nonblocking open flags on supported targets. */
#if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
#define _DARWIN_C_SOURCE
#elif defined(__linux__) && !defined(_GNU_SOURCE)
#define _GNU_SOURCE
#endif

#include "refill.h"

#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

/* The generated checked entry borrows the admitted fd and returns its status. */
extern int32_t holo_hosted_entry(uint64_t kind, uint64_t fd);

/* Session-aware executables add one explicitly selected loopback invocation. */
#if defined(HOLO_HOSTED_SESSION)
#define HOLO_HOST_USAGE "usage: worker file PATH | stream-fd FD | listen-loopback PORT\n"
#else
#define HOLO_HOST_USAGE "usage: worker file PATH | stream-fd FD\n"
#endif

/*
 * Parse a nonstandard inherited descriptor from decimal text. Return -1
 * for empty, malformed, standard-stream, or nonrepresentable input. No
 * file is acquired during parsing and no conversion can overflow.
 */
static int inherited_descriptor(const char *text) {
    int value = 0;
    /* Every digit advances the input and is checked before the multiply. */
    for (; *text != '\0'; ++text) {
        if (*text < '0' || *text > '9') {
            return -1;
        }
        const int digit = *text - '0';
        if (value > (INT_MAX - digit) / 10) {
            return -1;
        }
        value = value * 10 + digit;
    }
    /* Reject stdio descriptors and empty input, whose initial value is zero. */
    return value >= 3 ? value : -1;
}

/*
 * Run with `file PATH` or `stream-fd FD`. Returns 2 when setup fails and
 * otherwise returns the Holo entry result. The host owns the descriptor
 * from successful open or validated inherited-descriptor selection, closes
 * it once on normal exit, and relies on OS cleanup after process traps.
 */
int main(int argc, char *argv[]) {
    /* Every admitted invocation names one input kind and one locator. */
    if (argc != 3) {
        fputs(HOLO_HOST_USAGE, stderr);
        return 2;
    }
#if defined(HOLO_HOSTED_SESSION)
    /* Listener mode owns and cleans up its own serial stream sessions. */
    if (strcmp(argv[1], "listen-loopback") == 0) {
        return holo_run_listener_experimental(argv[2]);
    }
#endif
    uint64_t kind;
    int fd;
    if (strcmp(argv[1], "file") == 0) {
        kind = HOLO_REFILL_FILE;
        /* Opening a FIFO must never hang before the regular-file check. */
        fd = open(argv[2], O_RDONLY | O_NONBLOCK | O_CLOEXEC | O_NOCTTY);
    } else if (strcmp(argv[1], "stream-fd") == 0) {
        kind = HOLO_REFILL_STREAM;
        fd = inherited_descriptor(argv[2]);
    } else {
        fputs(HOLO_HOST_USAGE, stderr);
        return 2;
    }
    /* An unsuccessful acquisition supplied no descriptor for the host to close. */
    if (fd < 0) {
        fputs("could not acquire hosted input descriptor\n", stderr);
        return 2;
    }
    /* Inspect the kind and mode without consuming input or exposing a Holo value. */
    if (!holo_refill_admit_experimental(kind, (uint64_t)fd)) {
        (void)close(fd);
        fputs("unsupported hosted input descriptor or mode\n", stderr);
        return 2;
    }
#if defined(HOLO_HOSTED_OUTPUT)
    /* Only output-capable builds establish send timeout and signal policy. */
    if (!holo_send_configure_experimental(kind, (uint64_t)fd)) {
        (void)close(fd);
        fputs("unsupported hosted output descriptor or mode\n", stderr);
        return 2;
    }
#endif
    const int status = holo_hosted_entry(kind, (uint64_t)fd);
    /* Close once. A retry after EINTR could close a reused descriptor. */
    (void)close(fd);
    return status;
}
