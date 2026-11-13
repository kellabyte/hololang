/*
 * Deterministic native syscall witnesses for the hosted input adapter. Build
 * the production adapter with its read/recv/poll symbols redirected here.
 * The witnesses observe destination identity, request extent, and call count.
 * No test payload is staged before the adapter writes into the caller's slot.
 */

#include "refill.h"

#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stddef.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

/* Known slot address and bytes beyond the requested prefix. */
static uint8_t slot[4] = {97, 97, 97, 97};
/* Simulated syscall result, errno, and the count of witnessed invocations. */
static ssize_t result;
static int error_code;
static int calls;
/* Input kind expected to select read for a file and recv for a stream. */
static uint64_t expected_kind;
/* Readiness event flags supplied by the deterministic poll witness. */
static short events;

/*
 * Record one input attempt; return the scenario's result and errno. On the
 * successful case write the sole new byte into the original caller's buffer.
 */
static ssize_t attempt(int fd, void *buffer, size_t length) {
    assert(fd == 15 && buffer == slot && length == 2);
    ++calls;
    /* Only the successful one-byte scenario initializes new content. */
    if (result == 1) {
        *(uint8_t *)buffer = 13;
    }
    errno = error_code;
    return result;
}

/* Supply the deterministic file outcome, using the production ABI. */
ssize_t holo_test_read(int fd, void *buffer, size_t length) {
    assert(expected_kind == HOLO_REFILL_FILE);
    return attempt(fd, buffer, length);
}

/* Supply the deterministic stream outcome without a retry or wait flag. */
ssize_t holo_test_recv(int fd, void *buffer, size_t length, int flags) {
    assert(expected_kind == HOLO_REFILL_STREAM);
    assert(flags == 0);
    return attempt(fd, buffer, length);
}

/*
 * Supply one readiness outcome and the requested event bits. Assert that the
 * production code waits on precisely one descriptor and issues no timed poll.
 */
int holo_test_poll(struct pollfd *interest, nfds_t count, int timeout) {
    assert(count == 1 && timeout == -1);
    assert(interest->fd == 15 && interest->events == POLLIN);
    ++calls;
    interest->revents = events;
    errno = error_code;
    return (int)result;
}

/*
 * Execute one controlled input outcome and compare status, syscall count,
 * and untouched spare bytes. The native adapter receives the original slot.
 */
static void try_refill(uint64_t kind, ssize_t received, int code, uint64_t expected) {
    calls = 0;
    expected_kind = kind;
    result = received;
    error_code = code;
    assert(holo_refill_experimental(kind, 15, slot, sizeof slot, 2) == expected);
    assert(calls == 1 && slot[1] == 97 && slot[2] == 97 && slot[3] == 97);
}

/*
 * Execute one controlled poll outcome and compare its status and syscall
 * count. A retry would change the count even if it returned the same result.
 */
static void try_wait(ssize_t ready, short flags, int code, uint64_t expected) {
    calls = 0;
    result = ready;
    events = flags;
    error_code = code;
    assert(holo_wait_input_experimental(15) == expected);
    assert(calls == 1);
}

/*
 * Confirm that admission inspects real descriptors without consuming data or
 * modifying their mode. The test closes the opened socket pairs.
 */
static void check_admission(void) {
    int descriptors[2];
    assert(socketpair(AF_UNIX, SOCK_STREAM, 0, descriptors) == 0);
    const int receiver = descriptors[0];
    assert(!holo_refill_admit_experimental(HOLO_REFILL_STREAM, (uint64_t)receiver));
    assert(fcntl(receiver, F_SETFL, O_NONBLOCK) == 0);
    assert(holo_refill_admit_experimental(HOLO_REFILL_STREAM, (uint64_t)receiver));
    assert(!holo_refill_admit_experimental(HOLO_REFILL_FILE, (uint64_t)receiver));
    const struct timeval timeout = {.tv_sec = 1, .tv_usec = 0};
    assert(setsockopt(receiver, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof timeout) == 0);
    assert(!holo_refill_admit_experimental(HOLO_REFILL_STREAM, (uint64_t)receiver));
    assert(close(descriptors[0]) == 0 && close(descriptors[1]) == 0);

    assert(socketpair(AF_UNIX, SOCK_DGRAM, 0, descriptors) == 0);
    assert(fcntl(descriptors[0], F_SETFL, O_NONBLOCK) == 0);
    assert(!holo_refill_admit_experimental(HOLO_REFILL_STREAM, (uint64_t)descriptors[0]));
    assert(close(descriptors[0]) == 0 && close(descriptors[1]) == 0);

    const int unconnected = socket(AF_UNIX, SOCK_STREAM, 0);
    assert(unconnected >= 0);
    assert(fcntl(unconnected, F_SETFL, O_NONBLOCK) == 0);
    assert(!holo_refill_admit_experimental(HOLO_REFILL_STREAM, (uint64_t)unconnected));
    assert(close(unconnected) == 0);
}

/*
 * Default scenario checks all recoverable outcome classes. Named scenarios
 * intentionally supply impossible native results and must terminate by trap.
 */
int main(int argc, char *argv[]) {
    /* All invalid cases share an otherwise valid admitted ABI request. */
    if (argc == 2) {
        if (strcmp(argv[1], "oversize") == 0) {
            result = 3;
        } else if (strcmp(argv[1], "negative") == 0) {
            result = -3;
        } else {
            assert(strcmp(argv[1], "bad-errno") == 0);
            result = -1;
            error_code = 0;
        }
        (void)holo_refill_experimental(HOLO_REFILL_FILE, 15, slot, sizeof slot, 2);
        return 0;
    }
    assert(argc == 1);
    check_admission();
    try_refill(HOLO_REFILL_FILE, 1, 0, 1);
    assert(slot[0] == 13);
    try_refill(HOLO_REFILL_FILE, 0, 0, HOLO_REFILL_END);
    try_refill(HOLO_REFILL_FILE, -1, EINTR, HOLO_REFILL_INTERRUPTED);
    try_refill(HOLO_REFILL_FILE, -1, EAGAIN, HOLO_REFILL_ERROR_TAG | EAGAIN);
    try_refill(HOLO_REFILL_STREAM, -1, EAGAIN, HOLO_REFILL_WOULD_BLOCK);
    try_refill(HOLO_REFILL_STREAM, -1, EBADF, HOLO_REFILL_ERROR_TAG | EBADF);
    try_wait(1, POLLIN, 0, 0);
    try_wait(1, POLLHUP, 0, 0);
    try_wait(1, POLLERR, 0, 0);
    try_wait(1, POLLNVAL, 0, HOLO_REFILL_ERROR_TAG | EBADF);
    try_wait(-1, 0, EINTR, HOLO_REFILL_INTERRUPTED);
    try_wait(-1, 0, EIO, HOLO_REFILL_ERROR_TAG | EIO);
    return 0;
}
