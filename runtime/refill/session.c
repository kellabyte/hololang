/*
 * Optional IPv4 loopback listener for a single hosted Holo entry.
 * Own one listening descriptor and at most one connected nonblocking
 * client. Session state stores only descriptor identities. Protocol bytes,
 * reply storage, and the application store stay in checked Holo regions.
 */

/* Expose native descriptor flags in the same profiles as the read adapter. */
#if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
#define _DARWIN_C_SOURCE
#elif defined(__linux__) && !defined(_GNU_SOURCE)
#define _GNU_SOURCE
#endif

#include "refill.h"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <netinet/in.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>

/* The generated entry borrows the host's listener identity until it returns. */
extern int32_t holo_hosted_entry(uint64_t kind, uint64_t fd);

/* One listener owner for the single-threaded native entry. */
static int listener = -1;
/* Accepted descriptor in the active session, or -1 after close/failure. */
static int current = -1;

/*
 * Parse one nonzero decimal port, checking each multiply before it executes.
 * Return zero for malformed input and no acquired resource.
 */
static uint16_t loopback_port(const char *text) {
    unsigned value = 0;
    /* Every successful iteration consumes exactly one checked digit. */
    for (; *text != '\0'; ++text) {
        if (*text < '0' || *text > '9') {
            return 0;
        }
        const unsigned digit = (unsigned)(*text - '0');
        if (value > (UINT16_MAX - digit) / 10u) {
            return 0;
        }
        value = value * 10u + digit;
    }
    return (uint16_t)value;
}

/*
 * Mark one owned descriptor close-on-exec without changing other flags.
 * If the query or update fails, the caller still owns and closes the fd.
 */
static bool mark_close_on_exec(int fd) {
    const int flags = fcntl(fd, F_GETFD);
    /* A failed lookup cannot supply flags for the update. */
    return flags != -1 && fcntl(fd, F_SETFD, flags | FD_CLOEXEC) != -1;
}

/*
 * Open a blocking TCP listener on the exact IPv4 loopback address.
 * On setup failure, close the acquired fd once and return -1.
 */
static int open_listener(uint16_t port) {
    const int fd = socket(AF_INET, SOCK_STREAM, 0);
    /* An acquisition failure supplies no descriptor to release. */
    if (fd < 0) {
        return -1;
    }
    const int reuse = 1;
    const struct sockaddr_in address = {
        .sin_family = AF_INET,
        .sin_port = htons(port),
        .sin_addr = {.s_addr = htonl(INADDR_LOOPBACK)},
    };
    /* No client becomes active until all listener setup is complete. */
    if (!mark_close_on_exec(fd) ||
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof reuse) != 0 ||
        bind(fd, (const struct sockaddr *)&address, sizeof address) != 0 ||
        listen(fd, 16) != 0) {
        (void)close(fd);
        return -1;
    }
    return fd;
}

/*
 * Release the current client, if any, with one close attempt. Clear host
 * ownership first so neither a later failure nor shutdown closes it twice.
 */
static void close_current(void) {
    const int fd = current;
    current = -1;
    /* An inactive session supplies no owned descriptor. */
    if (fd >= 0) {
        (void)close(fd);
    }
}

/*
 * Report one accept or descriptor-setup failure. Contradictory errno traps.
 * No active client is published when the attempt returns this status.
 */
static uint64_t accept_error(int code) {
    /* EINTR remains explicit and every other errno must be positive. */
    if (code == EINTR) {
        return HOLO_REFILL_INTERRUPTED;
    }
    if (code <= 0) {
        __builtin_trap();
    }
    return HOLO_REFILL_ERROR_TAG | (uint64_t)code;
}

/*
 * Validate that a listener identity names the sole live host owner.
 * This checks only compiler/runtime metadata and performs no native I/O.
 */
static void require_listener(uint64_t fd) {
    /* A closed listener or mismatch cannot be used for I/O. */
    if (listener < 0 || fd != (uint64_t)listener) {
        __builtin_trap();
    }
}

/*
 * Resolve a descriptor for the next synchronous adapter call. The caller
 * must have an active client in listener mode. Holo never owns or retains
 * the resolved integer.
 */
uint64_t holo_session_fd_experimental(uint64_t kind, uint64_t fd) {
    /* The original modes preserve the host's single descriptor. */
    if (kind < HOLO_REFILL_LISTENER && fd <= INT_MAX) {
        return fd;
    }
    if (kind != HOLO_REFILL_LISTENER) {
        __builtin_trap();
    }
    require_listener(fd);
    /* Interrupted or failed accepts have no client to use. */
    if (current < 0) {
        __builtin_trap();
    }
    return (uint64_t)current;
}

/*
 * Close the completed session and perform one blocking accept on the host
 * listener. Only a fully configured connected nonblocking stream is adopted.
 * File and inherited-stream callers observe End without changing ownership.
 */
uint64_t holo_next_session_experimental(uint64_t kind, uint64_t fd) {
    /* The single-descriptor modes have no second session. */
    if (kind < HOLO_REFILL_LISTENER && fd <= INT_MAX) {
        return HOLO_SESSION_END;
    }
    if (kind != HOLO_REFILL_LISTENER) {
        __builtin_trap();
    }
    require_listener(fd);
    close_current();
    const int accepted = accept(listener, NULL, NULL);
    /* A failed accept acquired no descriptor. The caller chooses whether to retry. */
    if (accepted < 0) {
        return accept_error(errno);
    }
    const int flags = fcntl(accepted, F_GETFL);
    /* Accept defaults may differ by platform. Set and inspect the actual mode. */
    if (flags == -1 || fcntl(accepted, F_SETFL, flags | O_NONBLOCK) == -1 ||
        !mark_close_on_exec(accepted)) {
        const int code = errno;
        (void)close(accepted);
        return accept_error(code);
    }
    /* Mode and peer metadata must hold before publishing the client. */
    if (!holo_refill_admit_experimental(HOLO_REFILL_STREAM, (uint64_t)accepted)) {
        (void)close(accepted);
        return HOLO_REFILL_ERROR_TAG | EINVAL;
    }
#if defined(HOLO_HOSTED_OUTPUT)
    /* Output users additionally establish per-socket send policy. */
    if (!holo_send_configure_experimental(HOLO_REFILL_STREAM, (uint64_t)accepted)) {
        (void)close(accepted);
        return HOLO_REFILL_ERROR_TAG | EINVAL;
    }
#endif
    current = accepted;
    return 0;
}

/*
 * Run the loopback listener in one process and lend its first client to the
 * checked entry. A setup failure returns 2. Normal completion closes the
 * remaining client and listener exactly once.
 */
int holo_run_listener_experimental(const char *port_text) {
    const uint16_t port = loopback_port(port_text);
    /* Zero marks malformed or out-of-range input and cannot be bound. */
    if (port == 0) {
        fputs("invalid loopback port\n", stderr);
        return 2;
    }
    listener = open_listener(port);
    if (listener < 0) {
        fputs("could not open loopback listener\n", stderr);
        return 2;
    }
    const uint64_t fd = (uint64_t)listener;
    /* Entry starts with a live client. Setup interruption reports a failure. */
    if (holo_next_session_experimental(HOLO_REFILL_LISTENER, fd) != 0) {
        (void)close(listener);
        listener = -1;
        fputs("could not admit first loopback client\n", stderr);
        return 2;
    }
    const int status = holo_hosted_entry(HOLO_REFILL_LISTENER, fd);
    close_current();
    (void)close(listener);
    listener = -1;
    return status;
}
