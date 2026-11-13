/*
 * Optional native readiness for the single-threaded lexical TCP entry.
 * Each Darwin queue watches one admitted stream at a time. The serial
 * host can register its next client after the prior checked close removes
 * the old filters. A wait changes one direction and lends no byte range.
 * Other invocations reuse the existing poll adapter and its scalar results.
 */

/* Expose Darwin's persistent dispatch filter flags before including headers. */
#if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
#define _DARWIN_C_SOURCE
#endif

#include "tcp_wait.h"
#include "tcp_concurrent.h"
#include "refill.h"

#include <errno.h>
#include <limits.h>

#if defined(__APPLE__)
#include <fcntl.h>
#include <sys/event.h>
#include <unistd.h>

/*
 * Unique readiness-arm token in the single-threaded checked host. It spans
 * stream generations, so reusing a descriptor cannot reuse a prior token.
 * The host never publishes a queue to another thread or reenters a wait
 * from a signal handler. Only opt-in waits touch this counter.
 */
static uintptr_t next_arm;

/*
 * Release a failed setup's unpublished queue once and retain the setup errno
 * for the caller. No stream authority was transferred to this function.
 */
static int queue_failure(int queue, int code)
{
    (void)close(queue);
    errno = code;
    return -1;
}

/*
 * Return the wait ABI's interruption or tagged positive errno. A nonpositive
 * native error contradicts the wait contract and traps.
 */
static uint64_t wait_error(int code)
{
    /* Guard native event data before exposing it as a public status. */
    if (code <= 0) {
        __builtin_trap();
    }
    return code == EINTR ? HOLO_REFILL_INTERRUPTED :
           HOLO_REFILL_ERROR_TAG | (uint64_t)code;
}

/*
 * Wait on this queue's sole stream. Re-enable only the chosen direction and
 * update its token before reading an event. Dispatch disables it on delivery.
 * An interrupted wait leaves no buffer borrowed, and the next wait explicitly
 * replaces its registration state before observing more readiness.
 */
static uint64_t queue_wait(int fd, int queue, bool writing)
{
    /* Never reuse a token that could belong to an already observed event. */
    if (next_arm == UINTPTR_MAX) {
        return wait_error(EOVERFLOW);
    }
    const uintptr_t token = ++next_arm;
    const short requested = writing ? EVFILT_WRITE : EVFILT_READ;
    const short other = writing ? EVFILT_READ : EVFILT_WRITE;
    struct kevent changes[2];
    /* One syscall disables any prior direction, rearms this direction, and waits. */
    EV_SET(&changes[0], (uintptr_t)fd, other, EV_DISABLE, 0, 0, (void *)token);
    EV_SET(&changes[1], (uintptr_t)fd, requested, EV_ADD | EV_ENABLE | EV_DISPATCH,
           0, 0, (void *)token);
    struct kevent event;
    const int ready = kevent(queue, changes, 2, &event, 1, NULL);
    /* No retry occurs here. The caller sees signal interruption directly. */
    if (ready == -1) {
        return wait_error(errno);
    }
    /* A stale event cannot authorize progress on a reused stream number. */
    if (ready != 1 || event.ident != (uintptr_t)fd ||
        (uintptr_t)event.udata != token) {
        return wait_error(EIO);
    }
    /* Change failures can name either submitted filter and carry native errno. */
    if ((event.flags & EV_ERROR) != 0) {
        if ((event.filter != requested && event.filter != other) ||
            event.data <= 0 || event.data > INT_MAX) {
            return wait_error(EIO);
        }
        return wait_error((int)event.data);
    }
    /* EOF is also a readiness hint. The next send or receive gives the outcome. */
    return event.filter == requested ? 0 : wait_error(EIO);
}
#endif

/*
 * Register one admitted stream on an owned Darwin queue. Start with both
 * filters disabled and lend no storage or descriptor ownership. On partial
 * setup failure the caller releases the untransferred stream and queue.
 */
int holo_tcp_queue_attach(int queue, int fd)
{
#if defined(__APPLE__)
    /* The readiness queue and accepted stream must be distinct live fds. */
    if (queue < 0 || fd < 0 || queue == fd) {
        errno = EBADF;
        return -1;
    }
    struct kevent changes[2];
    /* No filter is eligible until a wait assigns a fresh arm token. */
    EV_SET(&changes[0], (uintptr_t)fd, EVFILT_READ, EV_ADD | EV_DISABLE | EV_DISPATCH,
           0, 0, NULL);
    EV_SET(&changes[1], (uintptr_t)fd, EVFILT_WRITE, EV_ADD | EV_DISABLE | EV_DISPATCH,
           0, 0, NULL);
    return kevent(queue, changes, 2, NULL, 0, NULL);
#else
    /* Other targets never select the optional queue. */
    (void)queue;
    (void)fd;
    errno = ENOTSUP;
    return -1;
#endif
}

/* Acquire and configure a Darwin queue, or report an unsupported backend. */
int holo_tcp_queue_open(int fd)
{
#if defined(__APPLE__)
    /* Validate the trusted adapter's descriptor before acquiring resources. */
    if (fd < 0) {
        errno = EBADF;
        return -1;
    }
    const int queue = kqueue();
    /* A failed acquisition supplied no queue to release. */
    if (queue < 0) {
        return -1;
    }
    const int flags = fcntl(queue, F_GETFD);
    /* Keep queue ownership local to the checked host invocation. */
    if (flags == -1 || fcntl(queue, F_SETFD, flags | FD_CLOEXEC) == -1) {
        return queue_failure(queue, errno);
    }

    /* A queue whose first attachment failed must never reach the Holo entry. */
    if (holo_tcp_queue_attach(queue, fd) == -1) {
        return queue_failure(queue, errno);
    }
    return queue;
#else
    /* Keep backend selection explicit on hosts without Darwin kqueue. */
    (void)fd;
    errno = ENOTSUP;
    return -1;
#endif
}

/* Dispatch one explicit wait with the same readiness status on both paths. */
uint64_t holo_tcp_wait_experimental(uint64_t fd, uint64_t queue, uint64_t writing)
{
    /* Check native descriptor narrowing and direction before any OS access. */
    if (fd > INT_MAX || writing > 1) {
        __builtin_trap();
    }
    /* The broker's selector is distinct from both a native queue and poll. */
    if (queue == HOLO_TCP_CONCURRENT_SELECTOR) {
        return holo_tcp_concurrent_wait((int)fd, writing != 0);
    }
    /* The sentinel reuses the admitted stream's existing poll operation. */
    if (queue == UINT64_MAX) {
        return writing ? holo_wait_output_experimental(HOLO_REFILL_STREAM, fd) :
                         holo_wait_input_experimental(fd);
    }
    if (queue > INT_MAX || queue == fd) {
        __builtin_trap();
    }
#if defined(__APPLE__)
    return queue_wait((int)fd, (int)queue, writing != 0);
#else
    /* A foreign caller cannot request a queue on an unsupported target. */
    return HOLO_REFILL_ERROR_TAG | ENOTSUP;
#endif
}
