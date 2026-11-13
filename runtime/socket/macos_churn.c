/*
 * Validate macOS TCP readiness identity across descriptor reuse and rearm.
 * The single-threaded fixture keeps one bounded watch and one event snapshot
 * per direction. It captures actual readiness, defers dispatch until the
 * watch changes, and rejects any event that names the previous operation.
 * All sockets are IPv4 loopback. This standalone check takes no timings.
 */

#include "macos_streams.h"

#include <errno.h>
#include <limits.h>
#include <poll.h>
#include <stdbool.h>
#include <stdio.h>
#include <sys/event.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

/* Distinguish the direction in a captured event from its opaque identity. */
enum direction {
    read_ready,
    write_ready,
    direction_count,
};

/* Use bounded arm identities whose packed values fit in a pointer-sized token. */
enum arm {
    disarmed,
    read_arm,
    first_write_arm,
    next_write_arm,
    arm_count,
};

/* A caller-owned watch has one admitted descriptor and a unique generation. */
struct watch {
    int fd;                         /* Connected endpoint owned by this round. */
    uintptr_t generation;           /* Never reused in the same readiness run. */
    enum arm arms[direction_count]; /* Current operation per direction. */
};

/* Value returned by the wait. It remains valid data after its socket closes. */
struct snapshot {
    int fd;                        /* Observed native descriptor number. */
    enum direction direction;      /* Observed read or write filter. */
    uintptr_t token;                /* Observed generation and operation arm. */
};

/* Encode a generation and bounded arm for both backends. Never dereference it. */
static uintptr_t identity(const struct watch *watch, enum arm arm)
{
    return watch->generation * arm_count + (uintptr_t)arm;
}

/* Check whether an observed event still authorizes I/O on this live operation. */
static bool matches(const struct watch *watch, struct snapshot event)
{
    enum arm arm = watch->arms[event.direction];
    return arm != disarmed && event.fd == watch->fd &&
           event.token == identity(watch, arm);
}

/*
 * Replace an operation arm in the caller's watch. A persistent kqueue also
 * records the change and exact token. The fixture stops on rejected changes.
 */
static void set_arm(int queue, struct watch *watch, enum direction direction,
                    enum arm arm)
{
    /* One active direction makes every returned observation unambiguous. */
    enum direction other = direction == read_ready ? write_ready : read_ready;
    if (watch->arms[direction] == arm ||
        (arm != disarmed && watch->arms[other] != disarmed)) {
        fail_check("invalid readiness arm transition");
    }
    /* Poll builds interest from the local watch when the next wait begins. */
    if (queue != -1) {
        struct kevent change;
        short filter = direction == read_ready ? EVFILT_READ : EVFILT_WRITE;
        /* Re-adding a filter updates its token, including on writer rearm. */
        EV_SET(&change, (uintptr_t)watch->fd, filter,
               arm == disarmed ? EV_DISABLE : EV_ADD | EV_ENABLE,
               0, 0, (void *)identity(watch, arm));
        if (kevent(queue, &change, 1, NULL, 0, NULL) == -1) {
            fail_system("update churn readiness arm");
        }
    }
    watch->arms[direction] = arm;
}

/*
 * Wait for one observed direction or return false on timeout. Kqueue returns
 * its registration's opaque token. Poll snapshots the caller's arms before
 * polling because its returned bits carry no user data.
 */
static bool observe(int queue, const struct watch *watch, int timeout_ms,
                    struct snapshot *result)
{
    /* Preserve the native identity even when callers later mutate the watch. */
    if (queue == -1) {
        struct watch captured = *watch;
        struct pollfd fd = {
            .fd = captured.fd,
            .events = (captured.arms[read_ready] == disarmed ? 0 : POLLIN) |
                      (captured.arms[write_ready] == disarmed ? 0 : POLLOUT),
        };
        int count = poll(&fd, 1, timeout_ms);
        /* Interruption and unexpected flags fail this bounded quiet fixture. */
        if (count == -1) {
            fail_system("poll churn readiness");
        }
        if (count == 0) {
            return false;
        }
        if (count != 1 || (fd.revents != POLLIN && fd.revents != POLLOUT)) {
            fail_check("unexpected poll churn event");
        }
        enum direction direction = fd.revents == POLLIN ? read_ready : write_ready;
        *result = (struct snapshot){
            fd.fd, direction, identity(&captured, captured.arms[direction]),
        };
        return true;
    }

    struct timespec timeout = {
        .tv_sec = timeout_ms / 1000,
        .tv_nsec = (long)(timeout_ms % 1000) * 1000000L,
    };
    struct kevent event;
    int count = kevent(queue, NULL, 0, &event, 1, &timeout);
    /* A timeout has no event. Errors, EOF, and unknown filters fail the check. */
    if (count == -1) {
        fail_system("kevent churn readiness");
    }
    if (count == 0) {
        return false;
    }
    if (count != 1 || event.ident > INT_MAX || event.flags & (EV_ERROR | EV_EOF) ||
        (event.filter != EVFILT_READ && event.filter != EVFILT_WRITE)) {
        fail_check("unexpected kqueue churn event");
    }
    *result = (struct snapshot){
        (int)event.ident,
        event.filter == EVFILT_READ ? read_ready : write_ready,
        (uintptr_t)event.udata,
    };
    return true;
}

/* Capture a real ready event and verify that its live operation can use it. */
static struct snapshot require_ready(int queue, const struct watch *watch,
                                     enum direction direction)
{
    struct snapshot result;
    /* A one-second limit catches a missing rearm without an indefinite wait. */
    if (!observe(queue, watch, 1000, &result) ||
        result.direction != direction || !matches(watch, result)) {
        fail_check("current readiness was missing or named another operation");
    }
    return result;
}

/* Require an empty observation without consuming bytes or closing the stream. */
static void require_quiet(int queue, const struct watch *watch)
{
    struct snapshot unused;
    /* The zero timeout samples pending events without waiting for traffic. */
    if (observe(queue, watch, 0, &unused)) {
        fail_check("quiet TCP stream had an unexpected readiness event");
    }
}

/* Assert that fd-and-direction matching would accept this stale event. */
static void require_stale(const struct watch *watch, struct snapshot event)
{
    /* The active replacement arm must distinguish a real numeric fd collision. */
    if (watch->fd != event.fd || watch->arms[event.direction] == disarmed ||
        matches(watch, event)) {
        fail_check("a stale snapshot was not rejected by operation identity");
    }
}

/* Send one witness byte on the fixture's otherwise quiet nonblocking stream. */
static void send_witness(int fd, unsigned char byte)
{
    ssize_t sent = send(fd, &byte, sizeof(byte), 0);
    /* Partial, interrupted, or failed work invalidates this one-byte witness. */
    if (sent != (ssize_t)sizeof(byte)) {
        fail_check("TCP churn witness was not sent exactly once");
    }
}

/* Consume the current read's witness and validate it against the sender. */
static void receive_witness(int fd, unsigned char expected)
{
    unsigned char actual;
    ssize_t received = recv(fd, &actual, sizeof(actual), 0);
    /* An old event must never substitute data for the current connection. */
    if (received != (ssize_t)sizeof(actual) || actual != expected) {
        fail_check("replacement TCP stream did not deliver its witness");
    }
}

/* Close one owned descriptor once. Any unexpected status fails the fixture. */
static void release(int fd)
{
    /* Never retry close by descriptor number, which is reusable after release. */
    if (close(fd) == -1) {
        fail_system("release churn descriptor");
    }
}

/*
 * Reuse the same listener and wait mechanism across connection churn. In
 * this single-threaded process, closing both old endpoints lets the next
 * client and accept calls reuse their descriptor numbers without dup2.
 */
static void verify_churn(bool use_kqueue)
{
    enum { rounds = 128 };
    struct sockaddr_in address;
    int listener = open_loopback(&address);
    int queue = use_kqueue ? kqueue() : -1;
    /* Poll needs only local state. Kqueue must persist across every close. */
    if (use_kqueue && queue == -1) {
        fail_system("create persistent churn kqueue");
    }

    /* Every round exercises a fresh pair of logical connection generations. */
    for (uintptr_t round = 0; round < rounds; ++round) {
        int old_pair[2];
        open_tcp_pair(listener, &address, old_pair, 0);
        set_nonblocking(old_pair[0]);
        set_nonblocking(old_pair[1]);
        struct watch old = {.fd = old_pair[1], .generation = 2 * round + 1};

        /* Collect both real events before closing the original stream. */
        set_arm(queue, &old, read_ready, read_arm);
        send_witness(old_pair[0], 0xff);
        struct snapshot old_read = require_ready(queue, &old, read_ready);
        set_arm(queue, &old, read_ready, disarmed);
        set_arm(queue, &old, write_ready, first_write_arm);
        struct snapshot old_write = require_ready(queue, &old, write_ready);

        /* Leave the old writer registered and ready. Last close must remove it. */
        release(old_pair[1]);
        release(old_pair[0]);
        int fresh_pair[2];
        open_tcp_pair(listener, &address, fresh_pair, 0);
        set_nonblocking(fresh_pair[0]);
        set_nonblocking(fresh_pair[1]);
        struct watch current = {.fd = fresh_pair[1], .generation = 2 * round + 2};
        /* This is a real recycled numeric descriptor, without a synthetic alias. */
        if (current.fd != old.fd) {
            fail_check("TCP churn did not reuse the monitored descriptor number");
        }
        require_quiet(queue, &current);

        /* The stale read has a current fd and arm but the previous generation. */
        set_arm(queue, &current, read_ready, read_arm);
        require_stale(&current, old_read);
        require_quiet(queue, &current);
        unsigned char witness = (unsigned char)round;
        send_witness(fresh_pair[0], witness);
        (void)require_ready(queue, &current, read_ready);
        receive_witness(current.fd, witness);
        set_arm(queue, &current, read_ready, disarmed);

        /* Same numeric fd and direction also collide on a stale writer event. */
        set_arm(queue, &current, write_ready, first_write_arm);
        require_stale(&current, old_write);
        struct snapshot first_write = require_ready(queue, &current, write_ready);
        set_arm(queue, &current, write_ready, disarmed);
        require_quiet(queue, &current);

        /* Rearm on the same live connection. Its prior operation is now stale. */
        set_arm(queue, &current, write_ready, next_write_arm);
        require_stale(&current, first_write);
        (void)require_ready(queue, &current, write_ready);
        set_arm(queue, &current, write_ready, disarmed);
        require_quiet(queue, &current);
        release(fresh_pair[1]);
        release(fresh_pair[0]);
    }

    /* Poll has no owned queue descriptor. */
    if (use_kqueue) {
        release(queue);
    }
    release(listener);
    printf("%s verified %d TCP descriptor reuses, %d stale generations, %d stale writer arms\n",
           use_kqueue ? "kevent" : "poll", rounds, 2 * rounds, rounds);
}

/* Run both controls and the existing short-write fixture without timing. */
int main(void)
{
    verify_churn(false);
    verify_churn(true);
    verify_tcp_writers();
    return 0;
}
