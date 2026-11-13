/*
 * Qualify native macOS readiness APIs using connected nonblocking streams.
 * All backends deliver and validate the same numbered byte records. Socket
 * storage and registration are outside the timed I/O loop. The probe owns all
 * descriptors for each sample, with no descriptor reuse during a sample. The
 * TCP mode uses only IPv4 loopback. A separate untimed fixture exercises
 * backpressure and writer interest against bounded socket buffers.
 */

#include "macos_streams.h"

#include <errno.h>
#include <inttypes.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/event.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

enum {
    /* Registration, payload checks, and event output use bounded arrays. */
    max_sockets = 128,
    /* A TCP burst is bounded independently of the number of open streams. */
    max_burst = 16,
    /* Each backend runs first, second, and third equally often. */
    repetitions = 12,
};

/* The readiness API is the only changed part of each I/O sample. */
enum backend {
    backend_poll,
    backend_kevent,
    backend_kevent64,
    backend_count,
};

/* Human-readable readiness names in the same order as enum backend. */
static const char *const backend_names[] = {"poll", "kevent", "kevent64"};

/* One fixed-size record goes over the stream for each active socket. */
struct record {
    uint64_t sequence; /* Unique round and slot within the sample. */
    uint64_t witness;  /* Extra checked bytes derived from the sequence. */
};

_Static_assert(sizeof(struct record) == 16, "records must have no padding");

/* Amount of work per sample, shared by all readiness backends. */
struct workload {
    const char *name; /* Stable identifier in raw CSV. */
    int sockets;     /* Total simultaneously watched socket pairs. */
    int active;      /* Number of pairs receiving one record per round. */
    int rounds;      /* Number of independent write/wait/drain rounds. */
    int burst;       /* Records per active stream per TCP round. */
};

/* A sample chooses one transport before opening any descriptors. */
enum transport {
    transport_unix,
    transport_tcp,
};

/* Each sample owns its nonblocking streams and its readiness storage. */
struct waiter {
    enum backend backend; /* Chosen before setup and fixed for the sample. */
    int sockets;         /* Prefix of the arrays with initialized descriptors. */
    int queue;           /* Kqueue descriptor for either kqueue backend. */
    int senders[max_sockets]; /* The peer used to feed each monitored endpoint. */
    struct pollfd readers[max_sockets]; /* Read descriptors and poll output. */
    union {
        struct kevent ordinary[max_sockets];  /* Compact kqueue records. */
        struct kevent64_s wide[max_sockets];   /* Explicit 64-bit user data. */
    } events; /* Holds registration input during setup, event output afterward. */
};

/*
 * Construct quiet connected peers, then register every reader once for kqueue.
 * Poll reuses the same descriptor array on each wait. Allocation by the OS for
 * socket and queue resources happens through the setup syscalls.
 */
static void open_waiter(struct waiter *waiter, enum backend backend, int sockets,
                        enum transport transport)
{
    waiter->backend = backend;
    waiter->sockets = sockets;
    struct sockaddr_in address;
    /* A sample owns one listener only until all of its TCP peers are connected. */
    int listener = transport == transport_tcp ? open_loopback(&address) : -1;
    /* Open each pair fully before adding the reader to the fixed array prefix. */
    for (int slot = 0; slot < sockets; ++slot) {
        int pair[2];
        /* Continue only after the kernel has connected both local endpoints. */
        if (listener != -1) {
            open_tcp_pair(listener, &address, pair, 0);
        } else if (socketpair(AF_UNIX, SOCK_STREAM, 0, pair) == -1) {
            fail_system("socketpair");
        }
        set_nonblocking(pair[0]);
        set_nonblocking(pair[1]);
        waiter->senders[slot] = pair[0];
        waiter->readers[slot] = (struct pollfd){.fd = pair[1], .events = POLLIN};
    }
    /* The monitored data path owns only connected descriptors. */
    if (listener != -1) {
        close(listener);
    }

    /* Poll needs no event queue. Both kqueue APIs use a dedicated queue. */
    if (backend == backend_poll) {
        return;
    }
    waiter->queue = kqueue();
    /* Register level-triggered read readiness only after kqueue succeeds. */
    if (waiter->queue == -1) {
        fail_system("kqueue");
    }

    /* Select the registration record type once before encoding the batch. */
    if (backend == backend_kevent) {
        /* The compact Darwin ABI carries an opaque pointer-sized slot token. */
        for (int slot = 0; slot < sockets; ++slot) {
            EV_SET(&waiter->events.ordinary[slot], (uintptr_t)waiter->readers[slot].fd,
                   EVFILT_READ, EV_ADD, 0, 0, (void *)(uintptr_t)(slot + 1));
        }
        /* An empty event output requests registration without waiting. */
        if (kevent(waiter->queue, waiter->events.ordinary, sockets, NULL, 0, NULL) == -1) {
            fail_system("kevent registration");
        }
    } else {
        /* The wide ABI carries the same identity in its integer udata field. */
        for (int slot = 0; slot < sockets; ++slot) {
            EV_SET64(&waiter->events.wide[slot], (uint64_t)waiter->readers[slot].fd,
                     EVFILT_READ, EV_ADD, 0, 0, (uint64_t)(slot + 1), 0, 0);
        }
        /* An empty event output requests registration without waiting. */
        if (kevent64(waiter->queue, waiter->events.wide, sockets, NULL, 0, 0, NULL) == -1) {
            fail_system("kevent64 registration");
        }
    }
}

/* Release queue and connected descriptors after the sample and EOF check. */
static void close_waiter(const struct waiter *waiter)
{
    /* Only kqueue backends own the queue descriptor. */
    if (waiter->backend != backend_poll) {
        close(waiter->queue);
    }
    /* The sample never shares its descriptors with another sample. */
    for (int slot = 0; slot < waiter->sockets; ++slot) {
        close(waiter->readers[slot].fd);
        close(waiter->senders[slot]);
    }
}

/*
 * Wait for a nonempty readiness batch in the backend's native storage. Peers
 * have already written before this call, so a timeout rejects a broken run.
 */
static int wait_for_readers(struct waiter *waiter)
{
    const struct timespec timeout = {.tv_sec = 1};
    int count;
    /* Retry a signal interruption, and limit each actual readiness wait. */
    do {
        /* Poll scans the descriptor set. Kqueue returns only ready records. */
        switch (waiter->backend) {
        case backend_poll:
            count = poll(waiter->readers, (nfds_t)waiter->sockets, 1000);
            break;
        case backend_kevent:
            count = kevent(waiter->queue, NULL, 0, waiter->events.ordinary,
                           waiter->sockets, &timeout);
            break;
        case backend_kevent64:
            count = kevent64(waiter->queue, NULL, 0, waiter->events.wide,
                             waiter->sockets, 0, &timeout);
            break;
        default:
            fail_check("unknown readiness backend");
            return 0;
        }
    } while (count == -1 && errno == EINTR);
    /* A negative count is an OS error. Zero means no writer reached the reader. */
    if (count == -1) {
        fail_system("readiness wait");
    }
    if (count == 0) {
        fail_check("timed out waiting for a prefilled socket");
    }
    return count;
}

/*
 * Resolve a native event to its registered slot. A poll entry without an
 * event returns -1. Reject an unknown descriptor or unexpected flags.
 */
static int ready_slot(const struct waiter *waiter, int index)
{
    /* Poll stores readiness in the original slot, including quiet entries. */
    if (waiter->backend == backend_poll) {
        short flags = waiter->readers[index].revents;
        /* Ignore quiet entries and reject invalid or unrelated events. */
        if (flags == 0) {
            return -1;
        }
        if (flags & (POLLNVAL | POLLERR) || !(flags & (POLLIN | POLLHUP))) {
            fail_check("poll returned an invalid reader event");
        }
        return index;
    }

    uint64_t token;
    uint64_t fd;
    unsigned flags;
    short filter;
    /* Decode the ABI used for registration. Both carry the same slot token. */
    if (waiter->backend == backend_kevent) {
        const struct kevent *event = &waiter->events.ordinary[index];
        token = (uintptr_t)event->udata;
        fd = event->ident;
        flags = event->flags;
        filter = event->filter;
    } else {
        const struct kevent64_s *event = &waiter->events.wide[index];
        token = event->udata;
        fd = event->ident;
        flags = event->flags;
        filter = event->filter;
    }
    /* A bad token could index past the registered descriptor prefix. */
    if (token == 0 || token > (uint64_t)waiter->sockets) {
        fail_check("kqueue returned an unknown slot token");
    }
    int slot = (int)token - 1;
    /* EOF can carry unread data. An error or wrong descriptor is unexpected. */
    if ((flags & EV_ERROR) || filter != EVFILT_READ ||
        fd != (uint64_t)waiter->readers[slot].fd) {
        fail_check("kqueue returned an invalid reader event");
    }
    return slot;
}

/* Produce deterministic record bytes for a particular logical write. */
static struct record make_record(uint64_t sequence)
{
    return (struct record){
        .sequence = sequence,
        .witness = sequence * UINT64_C(0x9e3779b185ebca87) ^ UINT64_C(0xc6a4a7935bd1e995),
    };
}

/*
 * Transfer one record, preserving partial progress and retrying interruptions.
 * The probe sends one short record to an empty peer and reads it after readiness.
 * EOF and WouldBlock during that transfer reject the workload's assumptions.
 */
static struct record transfer_record(int fd, struct record record, int writing)
{
    unsigned char *bytes = (unsigned char *)&record;
    size_t offset = 0;
    /* Each successful call advances the offset toward the full record size. */
    while (offset < sizeof(record)) {
        /* Pass the remaining byte range directly to the socket call. */
        ssize_t count = writing
            ? send(fd, bytes + offset, sizeof(record) - offset, 0)
            : recv(fd, bytes + offset, sizeof(record) - offset, 0);
        /* EINTR retains the offset. Other errors invalidate the measurement. */
        if (count == -1 && errno == EINTR) {
            continue;
        }
        if (count == -1) {
            fail_system(writing ? "send record" : "recv record");
        }
        /* Zero progress would otherwise spin or accept a truncated record. */
        if (count == 0) {
            fail_check("unexpected end of stream during a record");
        }
        offset += (size_t)count;
    }
    return record;
}

/* Consume one complete record and reject any corrupted or misrouted bytes. */
static void receive_record(int fd, uint64_t sequence)
{
    struct record expected = make_record(sequence);
    struct record actual = transfer_record(fd, (struct record){0}, 0);
    /* The sequence identifies the round and descriptor independently of event order. */
    if (actual.sequence != expected.sequence || actual.witness != expected.witness) {
        fail_check("received incorrect record");
    }
}

/*
 * Run full write/wait/read rounds. Each active slot gets exactly one record,
 * confirmed independently of event order or how readiness is batched.
 */
static void exchange_records(struct waiter *waiter, const struct workload *workload)
{
    unsigned char seen[max_sockets] = {0};
    /* Round numbers form disjoint sequence ranges and rotate the active set. */
    for (int round = 0; round < workload->rounds; ++round) {
        int first = round % waiter->sockets;
        uint64_t base = (uint64_t)round * max_sockets + 1;
        /* Reset and prefill exactly the slots that should become ready. */
        for (int i = 0; i < workload->active; ++i) {
            int slot = (first + i) % waiter->sockets;
            seen[slot] = 0;
            transfer_record(waiter->senders[slot], make_record(base + (uint64_t)slot), 1);
        }

        int remaining = workload->active;
        /* Incomplete readiness batches are consumed until all sends are verified. */
        while (remaining > 0) {
            int count = wait_for_readers(waiter);
            int limit = waiter->backend == backend_poll ? waiter->sockets : count;
            /* Poll requires scanning registered descriptors. Kqueue visits its batch. */
            for (int i = 0; i < limit; ++i) {
                int slot = ready_slot(waiter, i);
                /* Skip slots that poll visited without an event. */
                if (slot < 0) {
                    continue;
                }
                int distance = (slot + waiter->sockets - first) % waiter->sockets;
                /* A quiet slot is unexpected, and only active slots have reset state. */
                if (distance >= workload->active || seen[slot]) {
                    fail_check("received readiness for an inactive or consumed slot");
                }
                receive_record(waiter->readers[slot].fd, base + (uint64_t)slot);
                seen[slot] = 1;
                --remaining;
            }
        }
    }
}

/* A live TCP receive keeps the initialized prefix of one bounded burst. */
struct burst_progress {
    struct record wire[max_burst]; /* Receive bytes, checked only when complete. */
    size_t filled;                  /* Initialized prefix accumulated by recv. */
};

/*
 * Encode a deterministic burst and send it to one quiet connected stream.
 * The stream may accept a short prefix. The caller's previous burst is
 * already drained, so would-block is a failed workload assumption here.
 */
static void send_burst(int fd, int burst, uint64_t first)
{
    struct record records[max_burst];
    /* Every record has a distinct sequence, including records within a burst. */
    for (int i = 0; i < burst; ++i) {
        records[i] = make_record(first + (uint64_t)i);
    }
    const unsigned char *bytes = (const unsigned char *)records;
    size_t length = (size_t)burst * sizeof(records[0]);
    size_t offset = 0;
    /* A successful send advances toward the exact encoded burst length. */
    while (offset < length) {
        ssize_t count = send(fd, bytes + offset, length - offset, 0);
        /* EINTR gives no progress. Other errors invalidate the sample. */
        if (count == -1 && errno == EINTR) {
            continue;
        }
        if (count == -1) {
            fail_system("send TCP burst");
        }
        /* Zero cannot advance a positive byte request. */
        if (count == 0) {
            fail_check("TCP burst send made no progress");
        }
        offset += (size_t)count;
    }
}

/*
 * Read one available prefix into the caller's burst state. TCP may split a
 * record anywhere. Return completion only after checking every byte of the
 * full expected burst. EOF or an I/O error rejects the workload.
 */
static int receive_burst(int fd, struct burst_progress *progress, int burst,
                         uint64_t first)
{
    size_t length = (size_t)burst * sizeof(progress->wire[0]);
    unsigned char *bytes = (unsigned char *)progress->wire;
    ssize_t count;
    /* EINTR preserves the cursor. The request covers exactly its unfilled suffix. */
    do {
        count = recv(fd, bytes + progress->filled, length - progress->filled, 0);
    } while (count == -1 && errno == EINTR);
    /* A stale readiness hint permits another wait and publishes no progress. */
    if (count == -1 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
        return 0;
    }
    /* An unexpected socket error or an incomplete EOF invalidates the sample. */
    if (count == -1) {
        fail_system("recv TCP burst");
    }
    if (count == 0) {
        fail_check("TCP stream ended inside a burst");
    }
    progress->filled += (size_t)count;
    /* Partial TCP input persists in this slot across readiness waits. */
    if (progress->filled != length) {
        return 0;
    }
    /* Validate the complete records without depending on event or read boundaries. */
    for (int i = 0; i < burst; ++i) {
        struct record expected = make_record(first + (uint64_t)i);
        struct record actual = progress->wire[i];
        /* A corrupt, duplicated or misrouted record invalidates the sample. */
        if (actual.sequence != expected.sequence || actual.witness != expected.witness) {
            fail_check("TCP burst contained incorrect record");
        }
    }
    return 1;
}

/*
 * Time matched TCP write/wait/drain rounds. The active set rotates, and the
 * backends share the same prefill, partial-read cursors, and payload validation.
 * A bounded stack array avoids allocating per event or round.
 */
static void exchange_tcp(struct waiter *waiter, const struct workload *workload)
{
    struct burst_progress progress[max_sockets] = {0};
    size_t length = (size_t)workload->burst * sizeof(struct record);
    /* Each round reserves disjoint sequence ranges for all potential active slots. */
    for (int round = 0; round < workload->rounds; ++round) {
        int first = round % waiter->sockets;
        uint64_t base = (uint64_t)round * max_sockets * max_burst + 1;
        /* Reset only live slots and finish issuing all sends before any wait. */
        for (int i = 0; i < workload->active; ++i) {
            int slot = (first + i) % waiter->sockets;
            progress[slot].filled = 0;
            send_burst(waiter->senders[slot], workload->burst,
                       base + (uint64_t)slot * max_burst);
        }

        int remaining = workload->active;
        /* Drain every active burst even when readiness arrives in smaller batches. */
        while (remaining > 0) {
            int count = wait_for_readers(waiter);
            int limit = waiter->backend == backend_poll ? waiter->sockets : count;
            /* Poll visits its registered set. Kqueue visits delivered records only. */
            for (int i = 0; i < limit; ++i) {
                int slot = ready_slot(waiter, i);
                /* Skip quiet poll entries and reject unexpected active identities. */
                if (slot < 0) {
                    continue;
                }
                int distance = (slot + waiter->sockets - first) % waiter->sockets;
                if (distance >= workload->active || progress[slot].filled == length) {
                    fail_check("TCP readiness did not identify a pending burst");
                }
                /* Only a fully checked burst removes the slot from the pending set. */
                remaining -= receive_burst(waiter->readers[slot].fd, &progress[slot],
                                           workload->burst, base + (uint64_t)slot * max_burst);
            }
        }
    }
}

/*
 * Verify a peer half-close with queued data. The event can announce EOF while
 * the record is still readable. Only a zero return from a positive-size recv
 * confirms stream EOF.
 */
static void verify_half_close(struct waiter *waiter, enum transport transport)
{
    transfer_record(waiter->senders[0], make_record(UINT64_MAX), 1);
    /* Preserve the connected reader while closing only the peer's write side. */
    if (shutdown(waiter->senders[0], SHUT_WR) == -1) {
        fail_system("shutdown");
    }
    struct burst_progress progress = {0};
    /* Drain the final record before interpreting EOF. TCP can split the record. */
    for (;;) {
        int count = wait_for_readers(waiter);
        /* Exactly one peer had queued data. It remains readable after shutdown. */
        if (count != 1 || ready_slot(waiter, 0) != 0) {
            fail_check("unexpected readiness batch for half-close");
        }
        /* Unix receives its prefilled record. TCP tracks any partial prefix. */
        if (transport == transport_unix) {
            receive_record(waiter->readers[0].fd, UINT64_MAX);
            break;
        }
        if (receive_burst(waiter->readers[0].fd, &progress, 1, UINT64_MAX)) {
            break;
        }
    }
    /* TCP may report readable data before the FIN has arrived. Wait for it. */
    if (transport == transport_tcp &&
        (wait_for_readers(waiter) != 1 || ready_slot(waiter, 0) != 0)) {
        fail_check("unexpected readiness after TCP drain");
    }
    unsigned char byte;
    ssize_t count_read;
    /* Require confirmed EOF after draining data, retrying interrupted reads. */
    do {
        count_read = recv(waiter->readers[0].fd, &byte, sizeof(byte), 0);
    } while (count_read == -1 && errno == EINTR);
    if (count_read != 0) {
        fail_check("drained half-closed peer did not reach EOF");
    }
}

/* Measure setup and I/O independently, then emit a validated raw CSV row. */
static void run_sample(const struct workload *workload, enum backend backend, int sample,
                       int position, enum transport transport)
{
    struct waiter waiter;
    uint64_t start = now_ns();
    open_waiter(&waiter, backend, workload->sockets, transport);
    uint64_t ready = now_ns();
    /* The Unix control uses whole records. TCP preserves arbitrary byte prefixes. */
    if (transport == transport_unix) {
        exchange_records(&waiter, workload);
    } else {
        exchange_tcp(&waiter, workload);
    }
    uint64_t finished = now_ns();
    verify_half_close(&waiter, transport);
    close_waiter(&waiter);
    printf("%s,%s,%d,%d,%d,%d,%d,", workload->name,
           backend_names[backend], sample, position, workload->sockets, workload->active,
           workload->rounds);
    /* Record TCP burst geometry while preserving the Unix CSV schema. */
    if (transport == transport_tcp) {
        printf("%d,", workload->burst);
    }
    printf("%" PRIu64 ",%" PRIu64 "\n", ready - start, finished - ready);
}

/*
 * Run the chosen transport and workloads with balanced backend order. The
 * shorter TCP mode validates the same data path and both writer backends.
 */
int main(int argc, char **argv)
{
    enum transport transport = transport_unix;
    int check_only = 0;
    /* Keep the historical Unix invocation, and admit only the two TCP modes. */
    if (argc == 2 && (strcmp(argv[1], "--tcp") == 0 || strcmp(argv[1], "--tcp-check") == 0)) {
        transport = transport_tcp;
        check_only = strcmp(argv[1], "--tcp-check") == 0;
    } else if (argc != 1) {
        fail_check("usage: socket-macos [--tcp | --tcp-check]");
    }
    const struct workload unix_workloads[] = {
        {.name = "single", .sockets = 1, .active = 1, .rounds = 25000},
        {.name = "sparse", .sockets = max_sockets, .active = 1, .rounds = 25000},
        {.name = "dense", .sockets = max_sockets, .active = max_sockets, .rounds = 512},
    };
    const struct workload tcp_workloads[] = {
        {.name = "single", .sockets = 1, .active = 1, .rounds = 12500, .burst = 1},
        {.name = "sparse", .sockets = max_sockets, .active = 1, .rounds = 12500, .burst = 1},
        {.name = "dense", .sockets = max_sockets, .active = max_sockets, .rounds = 256, .burst = 1},
        {.name = "burst", .sockets = max_sockets, .active = 8, .rounds = 2048, .burst = max_burst},
    };
    const enum backend orders[][backend_count] = {
        {backend_poll, backend_kevent, backend_kevent64},
        {backend_kevent, backend_kevent64, backend_poll},
        {backend_kevent64, backend_poll, backend_kevent},
        {backend_poll, backend_kevent64, backend_kevent},
        {backend_kevent64, backend_kevent, backend_poll},
        {backend_kevent, backend_poll, backend_kevent64},
    };
    size_t order_count = sizeof(orders) / sizeof(orders[0]);

    /* Keep untimed backpressure and writer interest checks outside the timed run. */
    if (check_only) {
        verify_tcp_writers();
    }
    const struct workload *workloads = transport == transport_unix ? unix_workloads : tcp_workloads;
    size_t workload_count = transport == transport_unix
        ? sizeof(unix_workloads) / sizeof(unix_workloads[0])
        : sizeof(tcp_workloads) / sizeof(tcp_workloads[0]);
    /* Only TCP includes variable-sized record batches in the report. */
    puts(transport == transport_unix
        ? "workload,backend,sample,position,sockets,active,rounds,setup_ns,io_ns"
        : "workload,backend,sample,position,sockets,active,rounds,records_per_peer,setup_ns,io_ns");
    /* Compare one workload at a time, rotating all permutations between samples. */
    for (size_t work = 0; work < workload_count; ++work) {
        struct workload workload = workloads[work];
        /* The conformance mode opens the same peers and runs a shorter matrix. */
        if (check_only) {
            workload.rounds = 8;
        }
        /* Each paired sample reopens descriptors to give every backend fresh peers. */
        for (int sample = 0; sample < (check_only ? 1 : repetitions); ++sample) {
            /* Backends execute sequentially to keep I/O work from competing. */
            for (int position = 0; position < backend_count; ++position) {
                run_sample(&workload, orders[(size_t)sample % order_count][position],
                           sample, position, transport);
            }
        }
    }
    return EXIT_SUCCESS;
}
