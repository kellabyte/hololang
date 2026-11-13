/*
 * Native socket-output witnesses for the checked synchronous send adapter.
 * Redirect only send and poll for deterministic status and call counts.
 * A real socketpair separately checks direct bytes and disconnect handling.
 */

#include "refill.h"

#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stddef.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

/* The test redirect changes the header declaration. Recover the original
 * symbol for the witness that exercises the actual platform send path. */
#undef send
extern ssize_t send(int fd, const void *bytes, size_t length, int flags);

/* The requested suffix includes NUL and high bytes with known identities. */
static const uint8_t source[] = {1, 0, 255, 3};
/* Deterministic result, errno, event bits, and number of issued operations. */
static ssize_t result;
static int error_code;
static short events;
static int calls;
/* Route real socketpair sends to the host only while its witness is active. */
static bool real_socket;

/*
 * Observe one send and its exact pointer, count, and platform flags.
 * The controlled call leaves the byte range unchanged and sets errno.
 */
ssize_t holo_test_send(int fd, const void *bytes, size_t length, int flags) {
    assert(bytes == source + 1 && length == 2);
#if defined(__linux__)
    assert(flags == MSG_NOSIGNAL);
#else
    assert(flags == 0);
#endif
    ++calls;
    /* Only the real socketpair witness issues a platform call. */
    if (real_socket) {
        return send(fd, bytes, length, flags);
    }
    assert(fd == 15);
    errno = error_code;
    return result;
}

/* Observe one indefinite wait on exactly one writable descriptor. */
int holo_test_poll(struct pollfd *interest, nfds_t count, int timeout) {
    assert(count == 1 && timeout == -1);
    assert(interest->fd == 15 && interest->events == POLLOUT);
    ++calls;
    interest->revents = events;
    errno = error_code;
    return (int)result;
}

/* Compare status and invocation count. A file must never reach send. */
static void try_send(uint64_t kind, ssize_t sent, int code, uint64_t expected) {
    calls = 0;
    result = sent;
    error_code = code;
    assert(holo_send_experimental(kind, 15, source + 1, 3, 2) == expected);
    assert(calls == (kind == HOLO_REFILL_STREAM));
}

/* Compare writable wait status and prove the absence of an implicit retry. */
static void try_wait(uint64_t kind, ssize_t ready, short flags, int code, uint64_t expected) {
    calls = 0;
    result = ready;
    events = flags;
    error_code = code;
    assert(holo_wait_output_experimental(kind, 15) == expected);
    assert(calls == (kind == HOLO_REFILL_STREAM));
}

/*
 * Check output admission and peer closure on a real socketpair. Disabling
 * process SIGPIPE suppression ensures the tested socket policy owns the result.
 */
static void check_socket(void) {
    int descriptors[2];
    assert(signal(SIGPIPE, SIG_DFL) != SIG_ERR);
    assert(socketpair(AF_UNIX, SOCK_STREAM, 0, descriptors) == 0);
    const int fd = descriptors[0];
    assert(fcntl(fd, F_SETFL, O_NONBLOCK) == 0);
    const struct timeval timeout = {.tv_sec = 1, .tv_usec = 0};
    assert(setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof timeout) == 0);
    assert(!holo_send_configure_experimental(HOLO_REFILL_STREAM, (uint64_t)fd));
    const struct timeval disabled = {0};
    assert(setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &disabled, sizeof disabled) == 0);
    const struct linger linger = {.l_onoff = 1, .l_linger = 1};
    assert(setsockopt(fd, SOL_SOCKET, SO_LINGER, &linger, sizeof linger) == 0);
    assert(!holo_send_configure_experimental(HOLO_REFILL_STREAM, (uint64_t)fd));
    const struct linger no_linger = {0};
    assert(setsockopt(fd, SOL_SOCKET, SO_LINGER, &no_linger, sizeof no_linger) == 0);
    assert(holo_send_configure_experimental(HOLO_REFILL_STREAM, (uint64_t)fd));
    real_socket = true;
    assert(holo_send_experimental(HOLO_REFILL_STREAM, (uint64_t)fd, source + 1, 3, 2) == 2);
    uint8_t received[2];
    assert(recv(descriptors[1], received, sizeof received, MSG_WAITALL) == 2);
    assert(memcmp(received, source + 1, sizeof received) == 0);
    assert(close(descriptors[1]) == 0);
    const uint64_t closed =
        holo_send_experimental(HOLO_REFILL_STREAM, (uint64_t)fd, source + 1, 3, 2);
    assert(closed == (HOLO_REFILL_ERROR_TAG | EPIPE));
    assert(close(fd) == 0);
}

/* Run recoverable cases or one invalid result expected to trap. */
int main(int argc, char *argv[]) {
    /* Isolate each contradiction in its own process and assert its trap. */
    if (argc == 2) {
        if (strcmp(argv[1], "oversize") == 0) {
            result = 3;
        } else if (strcmp(argv[1], "zero") == 0) {
            result = 0;
        } else if (strcmp(argv[1], "negative") == 0) {
            result = -3;
        } else {
            assert(strcmp(argv[1], "bad-errno") == 0);
            result = -1;
            error_code = 0;
        }
        (void)holo_send_experimental(HOLO_REFILL_STREAM, 15, source + 1, 3, 2);
        return 0;
    }
    assert(argc == 1);
    try_send(HOLO_REFILL_FILE, 2, 0, HOLO_REFILL_ERROR_TAG | ENOTSOCK);
    try_send(HOLO_REFILL_STREAM, 1, 0, 1);
    try_send(HOLO_REFILL_STREAM, 2, 0, 2);
    try_send(HOLO_REFILL_STREAM, -1, EINTR, HOLO_REFILL_INTERRUPTED);
    try_send(HOLO_REFILL_STREAM, -1, EWOULDBLOCK, HOLO_REFILL_WOULD_BLOCK);
    try_send(HOLO_REFILL_STREAM, -1, EPIPE, HOLO_REFILL_ERROR_TAG | EPIPE);
    try_wait(HOLO_REFILL_FILE, 1, POLLOUT, 0, HOLO_REFILL_ERROR_TAG | ENOTSOCK);
    try_wait(HOLO_REFILL_STREAM, 1, POLLOUT, 0, 0);
    try_wait(HOLO_REFILL_STREAM, 1, POLLHUP, 0, 0);
    try_wait(HOLO_REFILL_STREAM, 1, POLLERR, 0, 0);
    try_wait(HOLO_REFILL_STREAM, 1, POLLNVAL, 0, HOLO_REFILL_ERROR_TAG | EBADF);
    try_wait(HOLO_REFILL_STREAM, -1, 0, EINTR, HOLO_REFILL_INTERRUPTED);
    try_wait(HOLO_REFILL_STREAM, -1, 0, EIO, HOLO_REFILL_ERROR_TAG | EIO);
    check_socket();
    return 0;
}
