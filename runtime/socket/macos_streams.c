/*
 * Stream setup and a backpressure fixture for the standalone macOS readiness
 * probe. Only IPv4 loopback is bound, and both endpoints live until each
 * check finishes. The writer retains its initialized source and enables write
 * notifications only while a suffix remains blocked. These checks are untimed.
 */

#include "macos_streams.h"

#include <errno.h>
#include <fcntl.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/event.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

/* Stop on an OS error with errno intact. Process exit closes all descriptors. */
_Noreturn void fail_system(const char *operation)
{
    perror(operation);
    exit(EXIT_FAILURE);
}

/* Stop on failed validation so a broken I/O sample never reports a timing. */
_Noreturn void fail_check(const char *problem)
{
    fprintf(stderr, "%s\n", problem);
    exit(EXIT_FAILURE);
}

/* Read a monotonic wall clock in nanoseconds without allocating storage. */
uint64_t now_ns(void)
{
    struct timespec now;
    /* Timing requires a valid monotonic sample. */
    if (clock_gettime(CLOCK_MONOTONIC, &now) == -1) {
        fail_system("clock_gettime");
    }
    return (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
}

/* Make one descriptor nonblocking before sharing it with the selected waiter. */
void set_nonblocking(int fd)
{
    int flags = fcntl(fd, F_GETFL);
    /* Keep any current status flags, and reject setup if either call fails. */
    if (flags == -1 || fcntl(fd, F_SETFL, flags | O_NONBLOCK) == -1) {
        fail_system("fcntl O_NONBLOCK");
    }
}

/*
 * Own an ephemeral IPv4 loopback listener and expose its assigned address to
 * the connecting side. A failed setup stops before any stream is used.
 */
int open_loopback(struct sockaddr_in *address)
{
    *address = (struct sockaddr_in){
        .sin_family = AF_INET,
        .sin_addr.s_addr = htonl(INADDR_LOOPBACK),
    };
    int listener = socket(AF_INET, SOCK_STREAM, 0);
    /* Bind only loopback and get the kernel-selected port before connecting. */
    socklen_t length = sizeof(*address);
    if (listener == -1 ||
        bind(listener, (const struct sockaddr *)address, sizeof(*address)) == -1 ||
        listen(listener, 1) == -1 ||
        getsockname(listener, (struct sockaddr *)address, &length) == -1) {
        fail_system("IPv4 loopback listener");
    }
    return listener;
}

/*
 * Set one socket option before admitting the stream. The caller retains the
 * descriptor. The fixture stops if the kernel rejects its configuration.
 */
static void socket_option(int fd, int level, int option, int value)
{
    /* Reject failure so the timed backends use the same transport settings. */
    if (setsockopt(fd, level, option, &value, sizeof(value)) == -1) {
        fail_system("setsockopt");
    }
}

/*
 * Open the two ends of one TCP stream and transfer both descriptors to pair.
 * Optional small socket buffers induce backpressure in the writer fixture.
 * Connect and accept are synchronous setup, outside the I/O interval.
 */
void open_tcp_pair(int listener, const struct sockaddr_in *address,
                          int pair[2], int buffer_limit)
{
    pair[0] = socket(AF_INET, SOCK_STREAM, 0);
    /* No connected descriptor exists on failure. */
    if (pair[0] == -1) {
        fail_system("TCP client socket");
    }
    /* The capped client send buffer makes a large first write short. */
    if (buffer_limit > 0) {
        socket_option(pair[0], SOL_SOCKET, SO_SNDBUF, buffer_limit);
    }
    /* Connect only to the exact local listener, then accept the queued peer. */
    if (connect(pair[0], (const struct sockaddr *)address, sizeof(*address)) == -1) {
        fail_system("TCP connect");
    }
    pair[1] = accept(listener, NULL, NULL);
    /* Both descriptors must be live before configuring either for transfers. */
    if (pair[1] == -1) {
        fail_system("TCP accept");
    }
    /* Bound the receiver's local queue only for the backpressure fixture. */
    if (buffer_limit > 0) {
        socket_option(pair[1], SOL_SOCKET, SO_RCVBUF, buffer_limit);
    }
    socket_option(pair[0], IPPROTO_TCP, TCP_NODELAY, 1);
    socket_option(pair[1], IPPROTO_TCP, TCP_NODELAY, 1);
}

enum {
    /* Use separate poll entries and event bits for each end of the stream. */
    read_slot,
    write_slot,
    direction_count,
    /* Larger than the configured socket buffers so the writer cannot finish early. */
    writer_bytes = 256 * 1024,
    writer_socket_bytes = 4 * 1024,
};

/* Own backend readiness state while the writer fixture owns the stream. */
struct writer_watch {
    struct pollfd directions[direction_count]; /* Descriptors and current interests. */
    int queue;                                 /* Kqueue descriptor, or -1 for poll. */
};

/* Count actual transfer and notification outcomes rather than inferred blocks. */
struct writer_checks {
    unsigned short_writes; /* Successful sends accepting less than requested. */
    unsigned blocks;       /* Sends reporting would-block while data remained. */
    unsigned wakes;        /* Writer notifications consumed after each arm. */
};

/*
 * Register the receiver and initially disable writer notifications. The
 * fixture retains both sockets. Poll needs no queue descriptor.
 */
static struct writer_watch open_writer_watch(const int pair[2], int use_kqueue)
{
    struct writer_watch watch = {
        .directions = {{.fd = pair[1], .events = POLLIN}, {.fd = pair[0]}},
        .queue = -1,
    };
    /* The writer conformance test qualifies ordinary kqueue against poll. */
    if (!use_kqueue) {
        return watch;
    }
    watch.queue = kqueue();
    /* The new queue has no registrations before this fixture owns it. */
    if (watch.queue == -1) {
        fail_system("writer kqueue");
    }
    struct kevent events[direction_count];
    EV_SET(&events[read_slot], (uintptr_t)pair[1], EVFILT_READ, EV_ADD, 0, 0, NULL);
    EV_SET(&events[write_slot], (uintptr_t)pair[0], EVFILT_WRITE,
           EV_ADD | EV_DISABLE, 0, 0, NULL);
    /* Apply both registrations without retrieving a readiness event. */
    if (kevent(watch.queue, events, direction_count, NULL, 0, NULL) == -1) {
        fail_system("writer registration");
    }
    return watch;
}

/*
 * Change writer interest only on an actual blocked/unblocked transition.
 * Poll carries the new mask into the next wait. Kqueue keeps its persistent
 * registration and changes only its enabled state.
 */
static void set_writer_interest(struct writer_watch *watch, int enable)
{
    struct pollfd *writer = &watch->directions[write_slot];
    /* Double arms or redundant disarms would hide a missed state transition. */
    if ((writer->events == POLLOUT) == enable) {
        fail_check("redundant writer interest change");
    }
    /* In kqueue mode the local mask also tracks the interest the kernel sees. */
    if (watch->queue != -1) {
        struct kevent change;
        EV_SET(&change, (uintptr_t)writer->fd, EVFILT_WRITE,
               enable ? EV_ENABLE : EV_DISABLE, 0, 0, NULL);
        /* A rejected change means the fixture cannot check the interest contract. */
        if (kevent(watch->queue, &change, 1, NULL, 0, NULL) == -1) {
            fail_system("writer interest");
        }
    }
    writer->events = enable ? POLLOUT : 0;
}

/*
 * Return a bit per ready stream end, or zero when the bounded wait expires.
 * Validate descriptor, direction and current writer interest for each event.
 */
static unsigned writer_ready(struct writer_watch *watch, int wait)
{
    /* Initialize the output array so malformed counts cannot expose stale stack data. */
    struct kevent events[direction_count] = {0};
    struct timespec timeout = {.tv_sec = wait};
    int count;
    /* Retry signals. Both controls wait for the same directions and duration. */
    do {
        count = watch->queue == -1
            ? poll(watch->directions, direction_count, wait * 1000)
            : kevent(watch->queue, NULL, 0, events, direction_count, &timeout);
    } while (count == -1 && errno == EINTR);
    /* A failed system call cannot establish a valid notification. */
    if (count == -1) {
        fail_system("wait for TCP backpressure");
    }
    /* The native result must fit the exact supplied event capacity. */
    if (count < 0 || count > direction_count) {
        fail_check("invalid readiness event count during TCP backpressure");
    }

    unsigned ready = 0;
    /* Poll must inspect both slots. Kqueue returns only the signaled filters. */
    if (watch->queue == -1) {
        for (int slot = 0; slot < direction_count; ++slot) {
            short flags = watch->directions[slot].revents;
            /* No flags are legal beyond the currently requested direction. */
            if (flags & ~watch->directions[slot].events) {
                fail_check("unexpected poll event during TCP backpressure");
            }
            /* Translate native readiness to the common stream direction bits. */
            if (flags != 0) {
                ready |= 1u << slot;
            }
        }
    } else {
        for (int i = 0; i < count; ++i) {
            /* The two known filters each have one expected, live descriptor. */
            int slot = events[i].filter == EVFILT_READ ? read_slot : write_slot;
            unsigned bit = 1u << slot;
            /* Repeated events, EOF, and events for disarmed writes are invalid here. */
            if ((events[i].filter != EVFILT_READ && events[i].filter != EVFILT_WRITE) ||
                events[i].ident != (uintptr_t)watch->directions[slot].fd ||
                events[i].flags & (EV_ERROR | EV_EOF) ||
                watch->directions[slot].events == 0 || (ready & bit) != 0) {
                fail_check("unexpected kqueue event during TCP backpressure");
            }
            ready |= bit;
        }
    }
    return ready;
}

/* Derive an oracle byte from its stream offset, including embedded zeros. */
static unsigned char writer_byte(size_t offset)
{
    return (unsigned char)(offset * 131u ^ (offset >> 8) ^ (offset >> 16));
}

/*
 * Advance the writer's cursor until the initialized suffix is sent or the
 * socket would block. A positive short write is recorded before continuing.
 */
static size_t send_available(int fd, const unsigned char payload[writer_bytes],
                             size_t offset, struct writer_checks *checks)
{
    /* Each positive send advances the same suffix. No prior bytes are resent. */
    while (offset < writer_bytes) {
        size_t remaining = writer_bytes - offset;
        ssize_t count = send(fd, payload + offset, remaining, 0);
        /* A signal keeps the cursor. A full send queue ends this attempt. */
        if (count == -1 && errno == EINTR) {
            continue;
        }
        if (count == -1 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            ++checks->blocks;
            break;
        }
        /* Other errors and impossible progress invalidate the fixture. */
        if (count == -1) {
            fail_system("send TCP backpressure data");
        }
        if (count == 0 || (size_t)count > remaining) {
            fail_check("invalid TCP send progress");
        }
        if ((size_t)count < remaining) {
            ++checks->short_writes;
        }
        offset += (size_t)count;
    }
    return offset;
}

/*
 * Consume the available TCP prefix into bounded scratch and check each byte
 * against its stream offset. The caller keeps its cursor after would-block.
 */
static size_t drain_available(int fd, size_t offset, size_t sent)
{
    unsigned char buffer[writer_socket_bytes];
    /* Keep reading until TCP goes quiet or all expected bytes are verified. */
    while (offset < writer_bytes) {
        ssize_t count = recv(fd, buffer, sizeof(buffer), 0);
        /* A signal supplies no bytes. Would-block leaves the checked prefix intact. */
        if (count == -1 && errno == EINTR) {
            continue;
        }
        if (count == -1 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            break;
        }
        /* EOF or data beyond the sender's cursor violates the workload. */
        if (count == -1) {
            fail_system("recv TCP backpressure data");
        }
        if (count == 0 || (size_t)count > sent - offset) {
            fail_check("invalid TCP receive progress");
        }
        /* Exact bytes and order are checked independently of TCP read boundaries. */
        for (size_t i = 0; i < (size_t)count; ++i) {
            if (buffer[i] != writer_byte(offset + i)) {
                fail_check("incorrect TCP backpressure data");
            }
        }
        offset += (size_t)count;
    }
    return offset;
}

/*
 * Exercise positive short sends, blocked writes, and repeated rearming on
 * one TCP stream. No timings from this fixture enter the benchmark.
 */
static void verify_writer(int use_kqueue)
{
    struct sockaddr_in address;
    int listener = open_loopback(&address);
    int pair[2];
    open_tcp_pair(listener, &address, pair, writer_socket_bytes);
    close(listener);
    set_nonblocking(pair[0]);
    set_nonblocking(pair[1]);
    struct writer_watch watch = open_writer_watch(pair, use_kqueue);
    struct writer_checks checks = {0};
    unsigned char payload[writer_bytes];
    /* Initialize the full owned source once and reuse its suffix on every wake. */
    for (size_t i = 0; i < sizeof(payload); ++i) {
        payload[i] = writer_byte(i);
    }
    /* An idle writable socket must not generate an unrequested notification. */
    if (writer_ready(&watch, 0) != 0) {
        fail_check("idle TCP stream signaled before sending");
    }
    size_t sent = send_available(pair[0], payload, 0, &checks);
    /* An idle receiver and capped buffers must force a positive short send. */
    if (sent == 0 || sent == sizeof(payload) || checks.short_writes == 0 || checks.blocks != 1) {
        fail_check("TCP writer fixture did not create backpressure");
    }
    set_writer_interest(&watch, 1);
    size_t received = 0;
    const uint64_t deadline = now_ns() + UINT64_C(10000000000);
    /* Either peer can make progress. Continue only while there is data to verify. */
    while (received < sizeof(payload)) {
        unsigned ready = writer_ready(&watch, 1);
        /* Reject a stalled fixture instead of spinning on a lost notification. */
        if (ready == 0 || now_ns() >= deadline) {
            fail_check("TCP writer fixture stalled");
        }
        /* Draining may release another portion of the peer's send buffer. */
        if (ready & (1u << read_slot)) {
            received = drain_available(pair[1], received, sent);
        }
        /* One wake consumes the current interest. Re-arm only on would-block. */
        if (ready & (1u << write_slot)) {
            ++checks.wakes;
            set_writer_interest(&watch, 0);
            sent = send_available(pair[0], payload, sent, &checks);
            if (sent < sizeof(payload)) {
                set_writer_interest(&watch, 1);
            }
        }
    }
    /* Require multiple completed re-arms and no idle write event after draining. */
    if (sent != sizeof(payload) || checks.wakes < 2 || checks.blocks != checks.wakes ||
        watch.directions[write_slot].events != 0 || writer_ready(&watch, 0) != 0) {
        fail_check("TCP writer interest did not return to idle");
    }
    /* Only the kqueue control owns an extra descriptor. */
    if (watch.queue != -1) {
        close(watch.queue);
    }
    close(pair[0]);
    close(pair[1]);
    fprintf(stderr, "TCP writer %s verified %zu bytes, %u short writes, %u blocks, %u wakes\n",
            use_kqueue ? "kevent" : "poll", sizeof(payload), checks.short_writes, checks.blocks, checks.wakes);
}

/* Exercise the same checked writer protocol under both readiness controls. */
void verify_tcp_writers(void)
{
    verify_writer(0);
    verify_writer(1);
}
