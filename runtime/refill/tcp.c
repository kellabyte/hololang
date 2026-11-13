/*
 * Native entry and close boundary for the experimental lexical TCP owner.
 * The host completes admission before transferring each loopback stream to
 * checked Holo. The generated entry has sole close authority for that stream.
 * Serial and concurrent modes keep the listener across independent entries.
 * The concurrent dispatcher optionally parks checked execution while a
 * worker waits, with no application payload backing or close authority
 * transferred back from Holo.
 */

/* Expose the native flags and socket options used by hosted admission. */
#if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
#define _DARWIN_C_SOURCE
#elif defined(__linux__) && !defined(_GNU_SOURCE)
#define _GNU_SOURCE
#endif

#include "refill.h"
#include "tcp_concurrent.h"
#include "tcp_wait.h"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <netinet/in.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

/* The generated entry takes the stream and borrows the optional queue. */
extern int32_t holo_tcp_entry(uint64_t kind, uint64_t fd, uint64_t queue);

/* Enable the optional native readiness backend only where it is available. */
#if defined(__APPLE__)
#define HOLO_TCP_USAGE "usage: worker accept-loopback PORT | accept-loopback-kqueue PORT\n" \
                       "              accept-loopback-many PORT COUNT | accept-loopback-kqueue-many PORT COUNT\n" \
                       "              accept-loopback-concurrent PORT COUNT LIMIT | accept-loopback-kqueue-concurrent PORT COUNT LIMIT\n" \
                       "              accept-loopback-parked PORT COUNT LIVE ACTIVE | accept-loopback-kqueue-parked PORT COUNT LIVE ACTIVE\n"
#else
#define HOLO_TCP_USAGE "usage: worker accept-loopback PORT | accept-loopback-many PORT COUNT\n" \
                       "              accept-loopback-concurrent PORT COUNT LIMIT\n" \
                       "              accept-loopback-parked PORT COUNT LIVE ACTIVE\n"
#endif

/*
 * Read a decimal argument through the given unsigned bound, including zero.
 * Invalid text returns false and leaves the supplied result untouched.
 */
static bool loopback_decimal(const char *text, uint64_t max, uint64_t *result) {
    uint64_t value = 0;
    /* Require at least one digit before accepting an argument. */
    if (*text == '\0') {
        return false;
    }
    /* Consume one digit per iteration and reject a nonrepresentable prefix. */
    for (; *text != '\0'; ++text) {
        if (*text < '0' || *text > '9') {
            return false;
        }
        const unsigned digit = (unsigned)(*text - '0');
        if (value > max / 10 || (value == max / 10 && digit > max % 10)) {
            return false;
        }
        value = value * 10u + digit;
    }
    *result = value;
    return true;
}

/*
 * Preserve native descriptor flags while marking the owned fd close-on-exec.
 * On failure the caller retains cleanup responsibility. This is setup work.
 */
static bool mark_close_on_exec(int fd) {
    const int flags = fcntl(fd, F_GETFD);
    /* An unsuccessful query cannot supply flags to the update. */
    return flags != -1 && fcntl(fd, F_SETFD, flags | FD_CLOEXEC) != -1;
}

/*
 * Acquire an IPv4 loopback listener with the requested pending backlog.
 * On success the caller owns the fd. Failure after acquisition closes it once.
 */
static int open_loopback_listener(uint16_t port, int backlog) {
    const int fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    /* Failed acquisition leaves no descriptor to release. */
    if (fd < 0) {
        return -1;
    }
    const int reuse = 1;
    const struct sockaddr_in address = {
        .sin_family = AF_INET,
        .sin_port = htons(port),
        .sin_addr = {.s_addr = htonl(INADDR_LOOPBACK)},
    };
    /* Do not publish a listener that failed any part of native setup. */
    if (!mark_close_on_exec(fd) ||
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof reuse) != 0 ||
        bind(fd, (const struct sockaddr *)&address, sizeof address) != 0 ||
        listen(fd, backlog) != 0) {
        (void)close(fd);
        return -1;
    }
    return fd;
}

/*
 * Establish nonblocking I/O, disabled timeouts and linger, and the native
 * signal policy. The fd comes from this host's IPv4 TCP listener, so admission
 * does not need to infer a transport from an arbitrary descriptor. False
 * leaves sole cleanup authority with the host. No application bytes move.
 */
static bool admit_accepted_stream(int fd) {
    const int flags = fcntl(fd, F_GETFL);
    /* Keep native mode and ownership valid before the checked entry begins. */
    return flags != -1 && fcntl(fd, F_SETFL, flags | O_NONBLOCK) != -1 &&
           mark_close_on_exec(fd) &&
           holo_refill_admit_experimental(HOLO_REFILL_STREAM, (uint64_t)fd) &&
           holo_send_configure_experimental(HOLO_REFILL_STREAM, (uint64_t)fd);
}

/*
 * Publish an explicitly requested ephemeral loopback port before waiting for
 * its peer. The listener remains host-owned if querying or writing fails.
 */
static bool publish_ephemeral_port(int listener) {
    struct sockaddr_in address = {0};
    socklen_t length = sizeof address;
    /* Publish only a fully initialized port selected by the IPv4 listener. */
    return getsockname(listener, (struct sockaddr *)&address, &length) == 0 &&
           length == sizeof address && address.sin_family == AF_INET &&
           address.sin_port != 0 &&
           printf("%u\n", (unsigned)ntohs(address.sin_port)) > 0 &&
           fflush(stdout) == 0;
}

/*
 * Attempt the terminal release once. Return zero or a positive tagged errno
 * (including EINTR), with no retry and no caller-visible descriptor. Retrying
 * after a native error could target an unrelated descriptor with the same fd.
 */
uint64_t holo_tcp_close_experimental(uint64_t fd) {
    /* Only an admitted, natively representable descriptor can be released. */
    if (fd > INT_MAX) {
        __builtin_trap();
    }
    const int result = close((int)fd);
    /* Capture errno immediately and reject contradictory native results. */
    if (result == 0) {
        return 0;
    }
    const int code = errno;
    if (result != -1 || code <= 0) {
        __builtin_trap();
    }
    return HOLO_REFILL_ERROR_TAG | (uint64_t)code;
}

/*
 * Run the selected serial admission loop, consuming the caller's listener.
 * Each generated entry consumes its own stream. The host keeps the listener
 * only in many mode and never closes a stream after a successful handoff.
 * Return the first failing setup or entry status, or zero after count entries.
 */
static int run_loopback(int listener, bool many, uint64_t count, bool queue_requested) {
    int status = 0;
    int queue = -1;
    /* An entry finishes before the next accept. No stream owner spans turns. */
    for (; count != 0; --count) {
        const int fd = accept(listener, NULL, NULL);
        /* Preserve the single-entry listener lifetime, including accept failure. */
        if (!many) {
            (void)close(listener);
        }
        if (fd < 0) {
            fputs("could not accept TCP loopback connection\n", stderr);
            status = 2;
            break;
        }
        /* Admission failure leaves sole stream close authority with the host. */
        if (!admit_accepted_stream(fd)) {
            (void)close(fd);
            fputs("unsupported accepted TCP descriptor or mode\n", stderr);
            status = 2;
            break;
        }
        /* Create the optional queue once. Closing each stream automatically
         * removes its filters, including when the next accept reuses its fd. */
        if (queue_requested) {
            bool attached;
            if (queue < 0) {
                queue = holo_tcp_queue_open(fd);
                attached = queue >= 0;
            } else {
                attached = holo_tcp_queue_attach(queue, fd) == 0;
            }
            /* A failed attachment precedes handoff and ends the whole sequence. */
            if (!attached) {
                (void)close(fd);
                fputs("could not configure TCP readiness queue\n", stderr);
                status = 2;
                break;
            }
        }
        const uint64_t ready_queue = queue < 0 ? UINT64_MAX : (uint64_t)queue;
        status = holo_tcp_entry(HOLO_REFILL_STREAM, (uint64_t)fd, ready_queue);
        /* Entry cleanup consumed fd on either return status. Do not use it again. */
        if (status != 0) {
            break;
        }
    }
    /* Only the separately owned queue and many-mode listener can remain live. */
    if (queue >= 0) {
        (void)close(queue);
    }
    if (many) {
        (void)close(listener);
    }
    return status;
}

/*
 * Parse an explicit host mode and pass its validated positive bounds to the
 * selected loop. Port zero reports the assigned port once before any accepts.
 * Syntax or setup failures return 2. The host never allocates stream backing.
 */
int main(int argc, char *argv[]) {
    /* Reject other argument counts before reading the mode and port. */
    if (argc < 3 || argc > 6) {
        fputs(HOLO_TCP_USAGE, stderr);
        return 2;
    }
    const bool poll_single = strcmp(argv[1], "accept-loopback") == 0;
    const bool poll_many = strcmp(argv[1], "accept-loopback-many") == 0;
    const bool poll_concurrent = strcmp(argv[1], "accept-loopback-concurrent") == 0;
    const bool poll_parked = strcmp(argv[1], "accept-loopback-parked") == 0;
#if defined(__APPLE__)
    const bool queue_single = strcmp(argv[1], "accept-loopback-kqueue") == 0;
    const bool queue_many = strcmp(argv[1], "accept-loopback-kqueue-many") == 0;
    const bool queue_concurrent = strcmp(argv[1], "accept-loopback-kqueue-concurrent") == 0;
    const bool queue_parked = strcmp(argv[1], "accept-loopback-kqueue-parked") == 0;
#else
    const bool queue_single = false;
    const bool queue_many = false;
    const bool queue_concurrent = false;
    const bool queue_parked = false;
#endif
    const bool many = poll_many || queue_many;
    const bool concurrent = poll_concurrent || queue_concurrent;
    const bool parked = poll_parked || queue_parked;
    const bool threaded = concurrent || parked;
    /* Concurrent modes bound native stacks. Parking also bounds permits. */
    if ((!many && !threaded && !poll_single && !queue_single) ||
        argc != (parked ? 6 : (concurrent ? 5 : (many ? 4 : 3)))) {
        fputs(HOLO_TCP_USAGE, stderr);
        return 2;
    }
    uint64_t port;
    /* Reject invalid text before passing a port to the network stack. */
    if (!loopback_decimal(argv[2], UINT16_MAX, &port)) {
        fputs("invalid loopback port\n", stderr);
        return 2;
    }
    uint64_t count = 1;
    /* Reject zero and overflow before binding so the run is finite and nonempty. */
    if ((many || threaded) &&
        (!loopback_decimal(argv[3], UINT64_MAX, &count) || count == 0)) {
        fputs("invalid loopback connection count\n", stderr);
        return 2;
    }
    uint64_t limit = 0;
    /* Validate the simultaneous stack bound before allocating a listener. */
    if (threaded &&
        (!loopback_decimal(argv[4], HOLO_TCP_CONCURRENT_LIMIT, &limit) || limit == 0)) {
        fputs(parked ? "invalid parked TCP live limit\n" :
                       "invalid concurrent TCP worker limit\n", stderr);
        return 2;
    }
    uint64_t active = 0;
    /* Parking cannot start with zero permits or reserve more than the slots. */
    if (parked && (!loopback_decimal(argv[5], limit, &active) || active == 0)) {
        fputs("invalid parked TCP active limit\n", stderr);
        return 2;
    }
    /* Keep the single backlog. The other modes may queue peers during work. */
    const int listener = open_loopback_listener((uint16_t)port, many || threaded ? SOMAXCONN : 1);
    if (listener < 0) {
        fputs("could not open TCP loopback listener\n", stderr);
        return 2;
    }
    /* Fixed-port invocations keep stdout exclusively for the application. */
    if (port == 0 && !publish_ephemeral_port(listener)) {
        (void)close(listener);
        fputs("could not publish TCP loopback port\n", stderr);
        return 2;
    }
    /* Each loop consumes the listener even if native setup fails. */
    if (parked) {
        return holo_tcp_run_parked(listener, count, (unsigned)limit, (unsigned)active,
                                    queue_parked, admit_accepted_stream);
    }
    if (concurrent) {
        return holo_tcp_run_concurrent(listener, count, (unsigned)limit, queue_concurrent,
                                        admit_accepted_stream);
    }
    return run_loopback(listener, many, count, queue_single || queue_many);
}
