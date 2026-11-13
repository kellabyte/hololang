/*
 * Trusted native adapter for experimental synchronous hosted input.
 * Host admission fixes the descriptor assumptions before input begins. A
 * successful attempt writes into existing backing once and reports the logical
 * prefix. The adapter performs no ownership transfer, allocation, or staging.
 */

/* Expose metadata-only descriptor flags for the platform admission check. */
#if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
#define _DARWIN_C_SOURCE
#elif defined(__linux__) && !defined(_GNU_SOURCE)
#define _GNU_SOURCE
#endif

#include "refill.h"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <unistd.h>

/* The internal LLVM boundary uses this fixed native descriptor and count cap. */
_Static_assert(INT_MAX == HOLO_REFILL_MAX_COUNT, "32-bit native int required");
_Static_assert(SIZE_MAX >= HOLO_REFILL_MAX_COUNT, "native count must fit size_t");
_Static_assert(SSIZE_MAX >= HOLO_REFILL_MAX_COUNT, "native count must fit ssize_t");

/*
 * Admit a connected byte stream using the previously validated fd and flags.
 * Returns false when a requirement fails. These inspections neither consume
 * input nor modify the socket. The host maintains all properties after admission.
 */
static bool admit_stream(int fd, int flags) {
    /* The prototype admits would-block only for explicit nonblocking mode. */
    if ((flags & O_NONBLOCK) == 0) {
        return false;
    }

    int socket_type = 0;
    socklen_t type_size = sizeof socket_type;
    /* A stream has a byte-oriented EOF. Other socket types fail admission. */
    if (getsockopt(fd, SOL_SOCKET, SO_TYPE, &socket_type, &type_size) == -1 ||
        type_size != sizeof socket_type || socket_type != SOCK_STREAM) {
        return false;
    }

    struct sockaddr_storage peer;
    socklen_t peer_size = sizeof peer;
    /* Require a peer before promising connected stream receive semantics. */
    if (getpeername(fd, (struct sockaddr *)&peer, &peer_size) == -1) {
        return false;
    }

    struct timeval timeout = {0};
    socklen_t timeout_size = sizeof timeout;
    /* With no timeout, the admitted EAGAIN/EWOULDBLOCK means would-block. */
    return getsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, &timeout_size) == 0 &&
           timeout_size == sizeof timeout && timeout.tv_sec == 0 &&
           timeout.tv_usec == 0;
}

/*
 * Inspect kind/fd and native mode once for the host that owns the descriptor.
 * Returns an admission decision without retaining or modifying the descriptor.
 * A false result supplies no trusted handle. These checks are setup work.
 */
bool holo_refill_admit_experimental(uint64_t kind, uint64_t fd) {
    /* Reject unrepresentable descriptors or unknown kinds before OS queries. */
    if (kind > HOLO_REFILL_STREAM || fd > INT_MAX) {
        return false;
    }

    const int flags = fcntl((int)fd, F_GETFL);
    const int access = flags & O_ACCMODE;
    /* Only successful, readable descriptors can supply input. */
    if (flags == -1 || (access != O_RDONLY && access != O_RDWR)) {
        return false;
    }
    /* An explicit execute-only descriptor cannot supply payload bytes. */
#if defined(O_EXEC)
    if ((flags & O_EXEC) != 0) {
        return false;
    }
#endif
    /* Metadata-only handles can report an O_RDONLY-looking access mode. */
#if defined(O_EVTONLY)
    if ((flags & O_EVTONLY) != 0) {
        return false;
    }
#elif defined(O_PATH)
    if ((flags & O_PATH) != 0) {
        return false;
    }
#endif

    /* Socket-specific checks classify connected streams. Files use stat. */
    if (kind == HOLO_REFILL_STREAM) {
        return admit_stream((int)fd, flags);
    }
    struct stat metadata;
    /* Reject pipes, devices and directories from the regular-file profile. */
    return fstat((int)fd, &metadata) == 0 && S_ISREG(metadata.st_mode);
}

/*
 * Read into the already initialized slot from the admitted kind/fd with the
 * checked positive limit. Returns the data count or a scalar outcome tag.
 * Invalid inputs and contradictory native results trap before publication.
 * The adapter issues one native operation and retains no caller storage.
 */
uint64_t holo_refill_experimental(uint64_t kind, uint64_t fd, uint8_t *slot,
                                  uint64_t initialized_extent, uint64_t limit) {
    /* Guard every narrowing and the writable prefix before the native call. */
    if (kind > HOLO_REFILL_STREAM || fd > INT_MAX || slot == NULL || limit == 0 ||
        limit > initialized_extent || limit > HOLO_REFILL_MAX_COUNT) {
        __builtin_trap();
    }

    /* Issue the one bounded operation selected by the host's admitted kind. */
    const ssize_t received = kind == HOLO_REFILL_FILE
                                 ? read((int)fd, slot, (size_t)limit)
                                 : recv((int)fd, slot, (size_t)limit, 0);
    /* Inspect errno only on reported failure and before any intervening call. */
    if (received == -1) {
        const int code = errno;
        /* An interruption publishes no prefix and lets the caller retry. */
        if (code == EINTR) {
            return HOLO_REFILL_INTERRUPTED;
        }
        /* Host admission establishes nonblocking stream mode without timeout. */
        if (kind == HOLO_REFILL_STREAM &&
            (code == EAGAIN || code == EWOULDBLOCK)) {
            return HOLO_REFILL_WOULD_BLOCK;
        }
        /* Native errors must fit in the positive int payload of this tag. */
        if (code <= 0) {
            __builtin_trap();
        }
        return HOLO_REFILL_ERROR_TAG | (uint64_t)code;
    }
    /* Impossible native counts cannot justify a readable prefix. */
    if (received < 0 || (uint64_t)received > limit) {
        __builtin_trap();
    }
    return (uint64_t)received;
}

/*
 * Wait once for the admitted fd to permit another input attempt. Poll never
 * consumes bytes. Interruption and native errors remain explicit, and no
 * staging, allocation of Holo storage, or implicit retry is needed.
 */
uint64_t holo_wait_input_experimental(uint64_t fd) {
    /* Avoid narrowing or passing an ignored negative fd to an infinite wait. */
    if (fd > INT_MAX) {
        __builtin_trap();
    }
    struct pollfd interest = {.fd = (int)fd, .events = POLLIN, .revents = 0};
    const int ready = poll(&interest, 1, -1);
    /* Capture errno immediately. Interruptions are observable to the caller. */
    if (ready == -1) {
        const int code = errno;
        /* Signal interruption supplies its own outcome, while errno stays positive. */
        if (code == EINTR) {
            return HOLO_REFILL_INTERRUPTED;
        }
        if (code <= 0) {
            __builtin_trap();
        }
        return HOLO_REFILL_ERROR_TAG | (uint64_t)code;
    }
    /* With one interest and an infinite timeout only one ready fd is valid. */
    if (ready != 1) {
        __builtin_trap();
    }
    /* An invalid fd cannot justify another receive, even if other bits appear. */
    if ((interest.revents & POLLNVAL) != 0) {
        return HOLO_REFILL_ERROR_TAG | (uint64_t)EBADF;
    }
    /* Let the next read report actual data, EOF, or a more specific error. */
    if ((interest.revents & (POLLIN | POLLHUP | POLLERR)) != 0) {
        return 0;
    }
    __builtin_trap();
}
