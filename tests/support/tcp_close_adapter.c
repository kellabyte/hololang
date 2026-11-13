/*
 * Link-only syscall witnesses for the lexical TCP owner's native close.
 * These tests use real Unix stream pairs and substitute only the listening
 * syscalls needed to reach deterministic admission without bind. The
 * injected EINTR follows an actual close and exact descriptor reuse.
 * On Darwin, a second client receives a saved real event from the first
 * client on the same descriptor number.
 * No test hooks are installed in the production compiler or runtime.
 */

/* Expose native nonblocking receive flags and descriptor APIs. */
#if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
#define _DARWIN_C_SOURCE
#elif defined(__linux__) && !defined(_GNU_SOURCE)
#define _GNU_SOURCE
#endif

#include "refill.h"

#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>
#if defined(__APPLE__)
#include <sys/event.h>
#endif

/* Compile-time substitutions applied to the real adapter only. */
extern int holo_fixture_host_main(int argc, char *argv[]);
extern int32_t holo_tcp_entry(uint64_t kind, uint64_t fd, uint64_t queue);

/* Exact descriptor whose sole close authority the fixture witnesses. */
static int owned_stream = -1;
/* Opposite socketpair endpoint, always kept by the fixture. */
static int peer_stream = -1;
/* Actual descriptor acquired by the production host before mocked bind. */
static int listener = -1;
/* Number of close calls targeting the accepted descriptor's integer. */
static unsigned stream_releases;
/* Number of close calls targeting the simulated listener. */
static unsigned listener_releases;
/* Count streams that reached the host admission check. */
static unsigned admissions;
/* Select a terminal error after successful release and descriptor reuse. */
static bool interrupt_close;
/* Many mode retains the listener and counts complete handoffs. */
static bool many;
/* Fail the selected admission or zero to admit every stream. */
static unsigned reject_at;
/* Number of descriptors accepted, including a rejected admission. */
static unsigned accepted;
/* First stream number, forced again on every later connection. */
static int first_stream = -1;
#if defined(__APPLE__)
/* Inject a captured event into the second checked wait. */
static bool stale_event;
/* Reject the second stream's attachment before publishing its owner. */
static bool fail_attachment;
/* Stable queue descriptor observed in the first real wait. */
static int wait_queue = -1;
/* Exactly one close of the queue must follow the last session. */
static unsigned queue_releases;
/* First wait's actual kernel event, including its old token. */
static struct kevent first_event;
#endif

/* Stop the child immediately if a fixture or ownership invariant fails. */
static void require(bool condition, const char *message) {
    /* A failed witness should never proceed or report a successful process. */
    if (!condition) {
        fputs(message, stderr);
        fputc('\n', stderr);
        _Exit(90);
    }
}

/* Acquire two real connected descriptors, each kept by one known test owner. */
static void make_stream_pair(void) {
    int pair[2];
    require(owned_stream < 0 && peer_stream < 0, "stream fixture reused");
    require(socketpair(AF_UNIX, SOCK_STREAM, 0, pair) == 0, "socketpair failed");
    owned_stream = pair[0];
    peer_stream = pair[1];
}

/* Move the owned endpoint to the first connection's exact fd number.
 * The fixture never overwrites a live listener, queue, or peer. */
static void reuse_stream_number(void) {
    if (first_stream == -1) {
        first_stream = owned_stream;
    } else if (peer_stream == first_stream) {
        peer_stream = owned_stream;
        owned_stream = first_stream;
    } else if (owned_stream != first_stream) {
        require(fcntl(first_stream, F_GETFD) == -1 && errno == EBADF,
                "the previous stream number is still occupied");
        require(dup2(owned_stream, first_stream) == first_stream,
                "could not force accepted fd reuse");
        require(close(owned_stream) == 0, "could not release duplicate");
        owned_stream = first_stream;
    }
}

/*
 * Release the tracked resource once. An injected stream failure places an
 * unrelated open file at exactly the original fd before reporting EINTR.
 */
int holo_fixture_close(int fd) {
    /* Only the real host and its close adapter call this substituted symbol. */
    if (fd == owned_stream) {
        require(++stream_releases == 1, "reused descriptor closed twice");
        require(close(fd) == 0, "stream close failed");
        if (interrupt_close) {
            const int replacement = open("/dev/null", O_RDONLY);
            require(replacement >= 0, "replacement open failed");
            /* Enforce exact reuse even if a lower descriptor was available. */
            if (replacement != fd) {
                require(dup2(replacement, fd) == fd, "descriptor reuse failed");
                require(close(replacement) == 0, "replacement alias close failed");
            }
            errno = EINTR;
            return -1;
        }
        return 0;
    }
    /* An early host failure still owns and releases its actual listener. */
    if (fd == listener) {
        require(++listener_releases == 1, "listener closed twice");
    }
#if defined(__APPLE__)
    /* The queue is independent of each accepted stream's release. */
    if (fd == wait_queue) {
        require(++queue_releases == 1 && accepted == (fail_attachment ? 2u : 3u) &&
                stream_releases == 1,
                "queue closed outside its host lifetime");
    }
#endif
    return close(fd);
}

/*
 * Validate the real host's requested IPv4 loopback address without binding.
 * Keep the actual acquired descriptor for the listener close witness.
 */
int holo_fixture_bind(int fd, const struct sockaddr *address, socklen_t size) {
    require(listener < 0, "listener bound twice");
    require(size == sizeof(struct sockaddr_in) && address->sa_family == AF_INET,
            "host did not select IPv4");
    const struct sockaddr_in *ipv4 = (const struct sockaddr_in *)address;
    require(ipv4->sin_addr.s_addr == htonl(INADDR_LOOPBACK),
            "host did not select loopback");
    listener = fd;
    return 0;
}

/* A successful synthetic listen publishes the already tracked descriptor. */
int holo_fixture_listen(int fd, int backlog) {
    require(fd == listener && backlog > 0, "invalid listen attempt");
    return 0;
}

/* Check the previous Holo owner released its fd before the next accept. */
static void verify_stream_release(void);

/* Supply one real byte stream per admission and make read readiness observable. */
int holo_fixture_accept(int fd, struct sockaddr *address, socklen_t *size) {
    require(fd == listener && address == NULL && size == NULL, "invalid accept attempt");
    if (accepted != 0) {
        require(many && listener_releases == 0, "listener ended before the next accept");
        verify_stream_release();
    }
    ++accepted;
    make_stream_pair();
    if (many) {
        reuse_stream_number();
        require(shutdown(peer_stream, SHUT_WR) == 0, "could not produce read EOF");
    }
    return owned_stream;
}

/* Witness the host's listener lifetime before each checked admission. */
bool holo_fixture_admit(uint64_t kind, uint64_t fd) {
    require(++admissions == accepted && listener_releases == (many ? 0u : 1u),
            "unexpected listener state at admission");
    require(kind == HOLO_REFILL_STREAM && fd == (uint64_t)owned_stream,
            "admission used the wrong stream");
    return admissions != reject_at;
}

#if defined(__APPLE__)
/* Submit all real queue operations. At the second read, replace only the
 * returned event with the first client's snapshot at the same fd and filter. */
int holo_fixture_kevent(int queue, const struct kevent *changes, int count,
                        struct kevent *events, int capacity, const struct timespec *timeout) {
    /* Fail only the next attachment, after the first entry finished. */
    if (fail_attachment && accepted == 2 && capacity == 0) {
        require(queue == wait_queue, "attachment used a different queue");
        errno = ENOMEM;
        return -1;
    }
    const int result = kevent(queue, changes, count, events, capacity, timeout);
    if ((stale_event || fail_attachment) && capacity != 0) {
        require(result == 1 && events[0].ident == (uintptr_t)first_stream &&
                events[0].filter == EVFILT_READ, "real stream read event missing");
        if (accepted == 1) {
            wait_queue = queue;
            first_event = events[0];
        } else {
            require(queue == wait_queue && events[0].udata != first_event.udata,
                    "persistent queue reused a previous arm");
            if (accepted == 2) {
                events[0] = first_event;
            }
        }
    }
    return result;
}
#endif

/*
 * Check stream release with actual peer EOF. Preserve the replacement file
 * long enough to show that neither Holo nor the host attempted another close.
 */
static void verify_stream_release(void) {
    require(stream_releases == 1, "stream was not released once");
    const int flags = fcntl(owned_stream, F_GETFD);
    /* A failed close retained the unrelated replacement, never the old socket. */
    if (interrupt_close) {
        require(flags >= 0, "reused descriptor is not live");
        require(close(owned_stream) == 0, "replacement cleanup failed");
    } else {
        require(flags == -1 && errno == EBADF, "released stream fd remains open");
    }
    char byte;
    require(recv(peer_stream, &byte, 1, MSG_DONTWAIT) == 0, "peer did not see EOF");
    require(close(peer_stream) == 0, "peer cleanup failed");
    owned_stream = -1;
    peer_stream = -1;
    stream_releases = 0;
}

/*
 * Exercise the checked entry directly with a trusted descriptor, or exercise
 * the real admission sequence with deterministic listening substitutes.
 * Return the original host or checked entry status after verifying cleanup.
 */
int main(int argc, char *argv[]) {
    require(argc == 2, "expected one fixture mode");
    const bool reject = strcmp(argv[1], "reject") == 0;
    const bool reject_second = strcmp(argv[1], "reject-second") == 0;
    const bool repeated = strcmp(argv[1], "many") == 0;
#if defined(__APPLE__)
    stale_event = strcmp(argv[1], "stale") == 0;
    fail_attachment = strcmp(argv[1], "fail-attachment") == 0;
    many = repeated || reject_second || stale_event || fail_attachment;
#else
    many = repeated || reject_second;
#endif
    reject_at = reject ? 1u : (reject_second ? 2u : 0u);
    interrupt_close = strcmp(argv[1], "interrupt") == 0;
    require(reject || many || interrupt_close || strcmp(argv[1], "normal") == 0,
            "unknown fixture mode");
    int status;
    /* Rejected admission and successive owners both use the real host. */
    if (reject || many) {
        char command[] = "tcp-close-fixture";
        char single_mode[] = "accept-loopback";
        char many_mode[] = "accept-loopback-many";
        char *mode = many ? many_mode : single_mode;
#if defined(__APPLE__)
        char queue_mode[] = "accept-loopback-kqueue-many";
        if (stale_event || fail_attachment) {
            mode = queue_mode;
        }
#endif
        char port[] = "42000";
        char count[] = "3";
        char *arguments[] = {command, mode, port, many ? count : NULL, NULL};
        status = holo_fixture_host_main(many ? 4 : 3, arguments);
        bool stopped_after_two = reject_second;
#if defined(__APPLE__)
        stopped_after_two = stopped_after_two || fail_attachment;
#endif
        const unsigned expected = many ? (stopped_after_two ? 2u : 3u) : 1u;
        require(accepted == expected && admissions == expected && listener_releases == 1,
                "host admission or listener lifetime did not match the sequence");
#if defined(__APPLE__)
        if (stale_event || fail_attachment) {
            require(wait_queue >= 0 && queue_releases == 1 &&
                    fcntl(wait_queue, F_GETFD) == -1 && errno == EBADF,
                    "persistent queue remained live after the host returned");
        }
#endif
    } else {
        make_stream_pair();
        status = holo_tcp_entry(HOLO_REFILL_STREAM, (uint64_t)owned_stream, UINT64_MAX);
    }
    verify_stream_release();
    return status;
}
