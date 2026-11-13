/*
 * Trusted native adapter for experimental synchronous stream output.
 * Host setup establishes per-descriptor output policy. The send path borrows
 * one initialized byte range for one native attempt and returns progress or
 * a scalar error. It retains no payload, descriptor, or temporary buffer.
 */

/* Expose platform socket option and signal-control definitions. */
#if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
#define _DARWIN_C_SOURCE
#elif defined(__linux__) && !defined(_GNU_SOURCE)
#define _GNU_SOURCE
#endif

#include "refill.h"

#include <errno.h>
#include <limits.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

/* The same ABI cap fits platform descriptor and native size conversions. */
_Static_assert(INT_MAX == HOLO_REFILL_MAX_COUNT, "32-bit native int required");
_Static_assert(SIZE_MAX >= HOLO_REFILL_MAX_COUNT, "native count must fit size_t");
_Static_assert(SSIZE_MAX >= HOLO_REFILL_MAX_COUNT, "native count must fit ssize_t");

/*
 * Inspect the admitted socket's output timeout and close policy. Darwin also
 * needs per-socket SIGPIPE suppression. A rejected handle is not published.
 * This setup is outside send attempts and never touches the byte range.
 */
bool holo_send_configure_experimental(uint64_t kind, uint64_t fd) {
    /* A file carries no stream output authority. Reject invalid ABI inputs. */
    if (kind > HOLO_REFILL_STREAM || fd > INT_MAX) {
        return false;
    }
    if (kind == HOLO_REFILL_FILE) {
        return true;
    }

    struct timeval timeout = {0};
    socklen_t timeout_size = sizeof timeout;
    /* Keep timeout distinct from EAGAIN/EWOULDBLOCK on the admitted stream. */
    if (getsockopt((int)fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, &timeout_size) != 0 ||
        timeout_size != sizeof timeout || timeout.tv_sec != 0 || timeout.tv_usec != 0) {
        return false;
    }
    struct linger linger = {0};
    socklen_t linger_size = sizeof linger;
    /* Descriptor release must not opt into waiting for unsent payload. */
    if (getsockopt((int)fd, SOL_SOCKET, SO_LINGER, &linger, &linger_size) != 0 ||
        linger_size != sizeof linger || linger.l_onoff != 0) {
        return false;
    }
#if defined(__APPLE__)
    /* Suppress only signals originating from this socket. */
    const int enabled = 1;
    return setsockopt((int)fd, SOL_SOCKET, SO_NOSIGPIPE, &enabled, sizeof enabled) == 0;
#elif defined(__linux__)
    /* Linux requests signal suppression on each send call. */
    return true;
#else
#error Unsupported hosted socket output target
#endif
}

/*
 * Encode a positive errno from a failed output attempt. EINTR and the
 * nonblocking send outcomes have their own scalar tags. No retry occurs.
 */
static uint64_t output_error(int code, bool can_would_block) {
    /* Reject contradictory failure values before exposing a status. */
    if (code <= 0) {
        __builtin_trap();
    }
    if (code == EINTR) {
        return HOLO_REFILL_INTERRUPTED;
    }
    if (can_would_block && (code == EAGAIN || code == EWOULDBLOCK)) {
        return HOLO_REFILL_WOULD_BLOCK;
    }
    return HOLO_REFILL_ERROR_TAG | (uint64_t)code;
}

/*
 * Perform one send from a checked initialized field suffix. A positive
 * result describes only the bytes accepted by this attempt. The caller
 * keeps the unsent suffix live and chooses how to retry.
 */
uint64_t holo_send_experimental(uint64_t kind, uint64_t fd, const uint8_t *bytes,
                                uint64_t initialized_extent, uint64_t count) {
    /* Validate every narrowing and the readable count before native access. */
    if (kind > HOLO_REFILL_STREAM || fd > INT_MAX || bytes == NULL || count == 0 ||
        count > initialized_extent || count > HOLO_REFILL_MAX_COUNT) {
        __builtin_trap();
    }
    /* File mode never writes the input descriptor or standard streams. */
    if (kind == HOLO_REFILL_FILE) {
        return HOLO_REFILL_ERROR_TAG | ENOTSOCK;
    }
#if defined(__linux__)
    const int flags = MSG_NOSIGNAL;
#else
    /* Darwin host setup established SO_NOSIGPIPE on this socket. */
    const int flags = 0;
#endif
    const ssize_t sent = send((int)fd, bytes, (size_t)count, flags);
    /* Capture errno before any intervening call. Zero cannot report progress
     * for a positive request under this connected byte-stream profile. */
    if (sent == -1) {
        return output_error(errno, true);
    }
    if (sent <= 0 || (uint64_t)sent > count) {
        __builtin_trap();
    }
    return (uint64_t)sent;
}

/*
 * Wait once for a writable event on the admitted socket. HUP/ERR let the
 * next send report the actual disconnection. The call has no byte extent.
 */
uint64_t holo_wait_output_experimental(uint64_t kind, uint64_t fd) {
    /* Do not pass a negative or ignored fd to an infinite poll. */
    if (kind > HOLO_REFILL_STREAM || fd > INT_MAX) {
        __builtin_trap();
    }
    if (kind == HOLO_REFILL_FILE) {
        return HOLO_REFILL_ERROR_TAG | ENOTSOCK;
    }
    struct pollfd interest = {.fd = (int)fd, .events = POLLOUT, .revents = 0};
    const int ready = poll(&interest, 1, -1);
    /* Poll does not use the send-specific would-block status. */
    if (ready == -1) {
        return output_error(errno, false);
    }
    if (ready != 1) {
        __builtin_trap();
    }
    /* Invalid descriptors are reported separately from readiness events. */
    if ((interest.revents & POLLNVAL) != 0) {
        return HOLO_REFILL_ERROR_TAG | EBADF;
    }
    if ((interest.revents & (POLLOUT | POLLHUP | POLLERR)) != 0) {
        return 0;
    }
    __builtin_trap();
}
