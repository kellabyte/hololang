/*
 * Deterministic fault witness for the bounded TCP broker. The real runtime
 * links with substituted thread creation, join, and poll calls in its own
 * object only. A pipe lends a listener-shaped descriptor without a bind.
 * Both failure paths must return without handing a stream to checked Holo,
 * in both held and permit-limited profiles.
 */

#include "tcp_concurrent.h"
#include "tcp_permits.h"

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <string.h>
#include <unistd.h>

/* Immutable setup read by the accept and broker threads. */
static int listener;
/* Select a native failure once listener polling has begun. */
static bool fail_broker;
/* Publish listener polling before injecting the broker failure. */
static atomic_bool admission_entered;
/* Count thread lifecycle calls in the native admission thread. */
static unsigned creates;
static unsigned joins;

/* A broken witness must terminate even when no peer can connect. */
static void require(bool condition)
{
    if (!condition) {
        __builtin_trap();
    }
}

/* Fail the second worker creation after starting the broker and worker one. */
int tcp_fault_create(pthread_t *thread, const pthread_attr_t *attr,
                     void *(*entry)(void *), void *argument)
{
    ++creates;
    if (!fail_broker && creates == 3) {
        return EAGAIN;
    }
    return pthread_create(thread, attr, entry, argument);
}

/* Count joins of the real worker or broker that the native host started. */
int tcp_fault_join(pthread_t thread, void **result)
{
    ++joins;
    return pthread_join(thread, result);
}

/* Inject a broker failure only once the main thread is at the idle
 * listener. The real admission poll then waits for the stop channel. */
int tcp_fault_poll(struct pollfd *fds, nfds_t count, int timeout)
{
    if (fail_broker) {
        if (fds[0].fd == listener) {
            require(count == 2);
            atomic_store(&admission_entered, true);
        } else {
            require(count == 1);
            while (!atomic_load(&admission_entered)) {
                (void)sched_yield();
            }
            errno = EIO;
            return -1;
        }
    }
    return poll(fds, count, timeout);
}

/* An invalid handoff would fail before any fixture data entered Holo. */
int32_t holo_tcp_entry(uint64_t kind, uint64_t fd, uint64_t queue)
{
    (void)kind;
    (void)fd;
    (void)queue;
    __builtin_trap();
}

/* No accept is expected from either failure case. */
static bool admit(int fd)
{
    (void)fd;
    __builtin_trap();
}

/* Exercise a full permit table repeatedly, including wrap and reuse. */
static void check_permit_fifo(void)
{
    struct holo_tcp_permits permits = {.limit = 2};
    /* Leave a nonzero head so each following full round crosses the ring end. */
    require(holo_tcp_permit_request(&permits, 0) == 0);
    require(holo_tcp_permit_release(&permits, 0) == HOLO_TCP_NO_PERMIT);
    for (unsigned round = 0; round < 3; ++round) {
        /* Reserve two permits and enqueue the remaining live slot indices. */
        for (unsigned slot = 0; slot < HOLO_TCP_CONCURRENT_LIMIT; ++slot) {
            require(holo_tcp_permit_request(&permits, slot) ==
                    (slot < 2 ? slot : HOLO_TCP_NO_PERMIT));
            require(permits.active <= 2);
        }
        /* Each release transfers its permit to the next oldest pending slot. */
        for (unsigned slot = 0; slot < HOLO_TCP_CONCURRENT_LIMIT; ++slot) {
            const unsigned next = slot + 2;
            require(holo_tcp_permit_release(&permits, slot) ==
                    (next < HOLO_TCP_CONCURRENT_LIMIT ? next : HOLO_TCP_NO_PERMIT));
        }
        require(permits.active == 0 && permits.pending == 0 && permits.held == 0 && permits.queued == 0);
    }
}

/* Create a quiet pipe whose read descriptor the host owns. Check bounded
 * failure, exact joins, and terminal release of that descriptor. The permit
 * mode instead runs the bounded table witness without creating threads. */
int main(int argc, char **argv)
{
    if (argc == 2 && strcmp(argv[1], "permit-fifo") == 0) {
        check_permit_fifo();
        return 0;
    }
    require(argc == 3);
    fail_broker = strcmp(argv[1], "broker") == 0;
    require(fail_broker || strcmp(argv[1], "startup") == 0);
    const bool parked = strcmp(argv[2], "parked") == 0;
    require(parked || strcmp(argv[2], "held") == 0);
    (void)alarm(5);
    int endpoints[2];
    require(pipe(endpoints) == 0);
    listener = endpoints[0];
    /* Use the same native faults with one permit and two prepared slots. */
    const int status = parked ? holo_tcp_run_parked(listener, 2, 2, 1, false, admit) :
                                holo_tcp_run_concurrent(listener, 2, 2, false, admit);
    require(status == 2);
    require(creates == 3 && joins == (fail_broker ? 3u : 2u));
    require(!fail_broker || atomic_load(&admission_entered));
    require(fcntl(listener, F_GETFD) == -1 && errno == EBADF);
    require(close(endpoints[1]) == 0);
    return 0;
}
