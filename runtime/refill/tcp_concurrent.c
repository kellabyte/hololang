/*
 * Bounded experimental TCP worker pool and shared readiness dispatcher.
 * A worker runs one checked entry at a time and owns its socket through
 * lexical cleanup. The dispatcher borrows only waiting fd identities.
 * Both backends share admission, worker synchronization and a wake pipe.
 * The parking mode also bounds checked execution while leaving each
 * admitted native stack and its Holo backing in place. The tables are
 * fixed size. Native stacks and kernel descriptors remain live costs.
 */

#if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
#define _DARWIN_C_SOURCE
#elif defined(__linux__) && !defined(_GNU_SOURCE)
#define _GNU_SOURCE
#endif

#include "tcp_concurrent.h"
#include "tcp_permits.h"
#include "refill.h"

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <pthread.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>

#if defined(__APPLE__)
#include <sys/event.h>
#endif

/* Reuse the checked entry ABI. The selector is not a native descriptor. */
extern int32_t holo_tcp_entry(uint64_t kind, uint64_t fd, uint64_t queue);

/* The low token bits identify a fixed worker slot. The rest never wrap. */
#define SLOT_BITS 6u
_Static_assert(HOLO_TCP_CONCURRENT_LIMIT == (1u << SLOT_BITS), "token layout");

struct dispatch;

/* One reusable host worker and one outstanding readiness request at most. */
struct slot {
    struct dispatch *owner;  /* Fixed host context, valid until worker join. */
    unsigned index;          /* Immutable index into the host's slot table. */
    pthread_t thread;        /* Joinable worker created before admission. */
    pthread_cond_t changed;  /* Assignment, readiness result, or shutdown. */
    int fd;                  /* -1 if no entry owns this slot's descriptor. */
    uintptr_t token;         /* Unique arm identity including slot index. */
    uint64_t result;         /* Copied readiness status, under the mutex. */
    bool writing;            /* Direction of this slot's outstanding wait. */
};

/* Host-owned fixed resources. Mutable worker state is guarded by mutex. */
struct dispatch {
    pthread_mutex_t mutex;     /* Serializes wait publication and dispatch. */
    pthread_cond_t capacity;   /* An entry released its slot or failed. */
    pthread_t broker;          /* Sole consumer of kernel readiness events. */
    struct slot slots[HOLO_TCP_CONCURRENT_LIMIT]; /* Bounded worker table. */
    unsigned limit;            /* Prepared slots, fixed before startup. */
    struct holo_tcp_permits permits; /* Optional ready-entry execution bound. */
    uint64_t waiting;          /* Slots with one live wait request. */
    uint64_t unarmed;          /* Published Darwin requests to install. */
    uintptr_t sequence;        /* Monotonic arm generation for both backends. */
    int wake[2];               /* Coalesced worker-to-broker notifications. */
    int stop_accept[2];        /* Failure wakes the admission loop. */
    int queue;                /* Darwin queue, or -1 for shared poll. */
    int failure;              /* First process failure, zero while admitting. */
    int broker_error;         /* Terminal readiness error for all later waits. */
    bool stop_workers;        /* Idle workers exit after the final assignment. */
    bool stop_broker;         /* Set only once all workers have joined. */
};

/* The compiler entry accesses only its own worker slot through the C adapter. */
static _Thread_local struct slot *current_slot;

/* A failure of an already initialized synchronization object is an invariant
 * violation. Setup failures are handled separately before any fd transfer. */
static void sync_ok(int status)
{
    if (status != 0) {
        __builtin_trap();
    }
}

/* One valid slot index always fits in the bounded bitset. */
static uint64_t slot_bit(unsigned index)
{
    return UINT64_C(1) << index;
}

/* Test permission under the dispatcher mutex. Ordinary concurrent mode
 * bypasses the execution table, including for newly assigned entries. */
static bool may_run(const struct slot *slot)
{
    const struct holo_tcp_permits *const permits = &slot->owner->permits;
    return permits->limit == 0 || (permits->held & slot_bit(slot->index)) != 0;
}

/* Wake the single worker whose permission changed under the mutex. */
static void signal_grant(struct dispatch *owner, unsigned granted)
{
    if (granted != HOLO_TCP_NO_PERMIT) {
        sync_ok(pthread_cond_signal(&owner->slots[granted].changed));
    }
}

/* Publish admission or matched readiness. In parking mode the ready slot
 * joins the FIFO at publication, independent of its native thread schedule. */
static void ready_slot(struct slot *slot)
{
    struct dispatch *const owner = slot->owner;
    if (owner->permits.limit == 0) {
        sync_ok(pthread_cond_signal(&slot->changed));
    } else {
        signal_grant(owner, holo_tcp_permit_request(&owner->permits, slot->index));
    }
}

/* Release checked execution on a real explicit wait or after entry return.
 * The waiter or final cleanup still retains its original stream ownership. */
static void release_slot(struct slot *slot)
{
    struct dispatch *const owner = slot->owner;
    if (owner->permits.limit != 0) {
        signal_grant(owner, holo_tcp_permit_release(&owner->permits, slot->index));
    }
}

/* Mark both ends of a new control pipe nonblocking and close-on-exec. On
 * error release both descriptors while retaining the native failure. */
static bool control_pipe(int endpoints[2])
{
    if (pipe(endpoints) != 0) {
        return false;
    }
    /* Each descriptor belongs exclusively to this host during setup. */
    for (unsigned end = 0; end < 2; ++end) {
        if (fcntl(endpoints[end], F_SETFL, O_NONBLOCK) == -1 ||
            fcntl(endpoints[end], F_SETFD, FD_CLOEXEC) == -1) {
            const int code = errno;
            (void)close(endpoints[0]);
            (void)close(endpoints[1]);
            errno = code;
            return false;
        }
    }
    return true;
}

/* Coalesce control notifications. A full pipe already guarantees a wake.
 * Both ends remain open until the consuming thread has joined. */
static void notify(int fd)
{
    const char byte = 0;
    ssize_t written;
    /* A signal may interrupt the notification before adding a byte. */
    do {
        written = write(fd, &byte, 1);
    } while (written == -1 && errno == EINTR);
    if (written != 1 && !(written == -1 && (errno == EAGAIN || errno == EWOULDBLOCK))) {
        __builtin_trap();
    }
}

/* Drain previous wake bytes while holding the mutex. A publisher arriving
 * after the next snapshot writes its own byte before the broker sleeps. */
static void drain(int fd)
{
    char bytes[256];
    for (;;) {
        const ssize_t count = read(fd, bytes, sizeof bytes);
        /* Positive reads and interruptions can leave unread wake bytes. */
        if (count > 0 || (count == -1 && errno == EINTR)) {
            continue;
        }
        if (count == -1 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            return;
        }
        __builtin_trap();
    }
}

/* Under the mutex, preserve the first failed status and wake both possible
 * admission waits. A failed worker never closes another worker's stream. */
static void mark_failure(struct dispatch *owner, int status)
{
    if (owner->failure == 0) {
        owner->failure = status;
        notify(owner->stop_accept[1]);
        sync_ok(pthread_cond_signal(&owner->capacity));
    }
}

/* Publish one matching request's copied result under the mutex. Removing
 * both bits precedes wakeup, so old events cannot complete this arm again. */
static void finish_wait(struct slot *slot, uint64_t status)
{
    struct dispatch *const owner = slot->owner;
    const uint64_t bit = slot_bit(slot->index);
    owner->waiting &= ~bit;
    owner->unarmed &= ~bit;
    slot->result = status;
    ready_slot(slot);
}

/* Fail the broker and wake every outstanding worker with a tagged error.
 * Future waits observe the same error without publishing another arm. */
static void fail_broker(struct dispatch *owner, int code)
{
    owner->broker_error = code;
    mark_failure(owner, 2);
    /* Finishing a wait removes its bit and reduces the outstanding set. */
    while (owner->waiting != 0) {
        const unsigned index = (unsigned)__builtin_ctzll(owner->waiting);
        finish_wait(&owner->slots[index], HOLO_REFILL_ERROR_TAG | (uint64_t)code);
    }
}

/* Build one bounded descriptor snapshot, then wait without the mutex.
 * Results match the captured arm and fd before authorizing any progress. */
static int poll_step(struct dispatch *owner)
{
    struct pollfd fds[HOLO_TCP_CONCURRENT_LIMIT + 1];
    uintptr_t tokens[HOLO_TCP_CONCURRENT_LIMIT + 1];
    nfds_t used = 1;
    fds[0] = (struct pollfd){.fd = owner->wake[0], .events = POLLIN};
    /* Snapshot the live set under the mutex. No worker can mutate an arm
     * without either consuming its result or publishing another wake. */
    for (uint64_t bits = owner->waiting; bits != 0; bits &= bits - 1) {
        const struct slot *const slot = &owner->slots[__builtin_ctzll(bits)];
        fds[used] = (struct pollfd){.fd = slot->fd,
                                   .events = slot->writing ? POLLOUT : POLLIN};
        tokens[used++] = slot->token;
    }
    sync_ok(pthread_mutex_unlock(&owner->mutex));
    int ready;
    /* A broker interruption is not a signal to an individual Holo worker. */
    do {
        ready = poll(fds, used, -1);
    } while (ready == -1 && errno == EINTR);
    const int code = ready == -1 ? errno : 0;
    sync_ok(pthread_mutex_lock(&owner->mutex));
    /* No stream result can be read after a failed kernel wait. */
    if (code != 0) {
        return code;
    }
    /* Ignore the pipe row. The next iteration drains its notification. */
    for (nfds_t i = 1; i < used; ++i) {
        const short observed = fds[i].revents;
        if (observed != 0) {
            const unsigned index = (unsigned)(tokens[i] & (HOLO_TCP_CONCURRENT_LIMIT - 1u));
            struct slot *const slot = &owner->slots[index];
            if ((owner->waiting & slot_bit(index)) != 0 &&
                slot->token == tokens[i] && slot->fd == fds[i].fd) {
                /* HUP and ERR still permit the exact socket operation to
                 * observe EOF or errno. POLLNVAL names an invalid fd. */
                if ((observed & POLLNVAL) != 0) {
                    finish_wait(slot, HOLO_REFILL_ERROR_TAG | EBADF);
                } else if ((observed & (fds[i].events | POLLHUP | POLLERR)) != 0) {
                    finish_wait(slot, 0);
                } else {
                    finish_wait(slot, HOLO_REFILL_ERROR_TAG | EIO);
                }
            }
        }
    }
    return 0;
}

#if defined(__APPLE__)
/* Install only fresh waits, with per-change receipts. The zero timeout
 * keeps the mutex held only across registration, never readiness waiting. */
static int queue_arm(struct dispatch *owner)
{
    struct kevent changes[HOLO_TCP_CONCURRENT_LIMIT];
    struct kevent receipts[HOLO_TCP_CONCURRENT_LIMIT];
    int used = 0;
    uint64_t missing = owner->unarmed;
    /* A new wait supersedes no live arm. EV_ONESHOT removes it on delivery. */
    for (uint64_t bits = missing; bits != 0; bits &= bits - 1) {
        const struct slot *const slot = &owner->slots[__builtin_ctzll(bits)];
        EV_SET(&changes[used], (uintptr_t)slot->fd,
               slot->writing ? EVFILT_WRITE : EVFILT_READ,
               EV_ADD | EV_ONESHOT | EV_RECEIPT, 0, 0, (void *)slot->token);
        ++used;
    }
    const struct timespec immediate = {0, 0};
    int count;
    /* Replaying the same add is idempotent while the table is locked. */
    do {
        count = kevent(owner->queue, changes, used, receipts, used, &immediate);
    } while (count == -1 && errno == EINTR);
    if (count != used) {
        return count == -1 ? errno : EIO;
    }
    /* Require one receipt for each installed identity, independent of order. */
    for (int i = 0; i < count; ++i) {
        const uintptr_t token = (uintptr_t)receipts[i].udata;
        const unsigned index = (unsigned)(token & (HOLO_TCP_CONCURRENT_LIMIT - 1u));
        struct slot *const slot = &owner->slots[index];
        const uint64_t bit = slot_bit(index);
        if ((missing & bit) == 0 || slot->token != token ||
            (uintptr_t)slot->fd != receipts[i].ident ||
            receipts[i].filter != (slot->writing ? EVFILT_WRITE : EVFILT_READ) ||
            (receipts[i].flags & EV_ERROR) == 0 ||
            receipts[i].data < 0 || receipts[i].data > INT32_MAX) {
            return EIO;
        }
        missing &= ~bit;
        if (receipts[i].data != 0) {
            finish_wait(slot, HOLO_REFILL_ERROR_TAG | (uint64_t)receipts[i].data);
        }
    }
    owner->unarmed = 0;
    return 0;
}

/* Consume a batch of events from one process queue. A captured event can
 * authorize progress only while its exact slot, arm, fd and direction match. */
static int queue_step(struct dispatch *owner)
{
    /* Changes must complete before the broker drops the mutex. */
    if (owner->unarmed != 0) {
        const int code = queue_arm(owner);
        if (code != 0) {
            return code;
        }
    }
    struct kevent events[HOLO_TCP_CONCURRENT_LIMIT + 1];
    sync_ok(pthread_mutex_unlock(&owner->mutex));
    int count;
    /* Broker-directed interruption does not interrupt a worker's condition. */
    do {
        count = kevent(owner->queue, NULL, 0, events, HOLO_TCP_CONCURRENT_LIMIT + 1, NULL);
    } while (count == -1 && errno == EINTR);
    const int code = count == -1 ? errno : 0;
    sync_ok(pthread_mutex_lock(&owner->mutex));
    if (code != 0) {
        return code;
    }
    /* Token zero belongs to the control pipe. Unknown or old generations
     * cannot wake the current client, even at the same integer descriptor. */
    for (int i = 0; i < count; ++i) {
        const struct kevent *const event = &events[i];
        const uintptr_t token = (uintptr_t)event->udata;
        const unsigned index = (unsigned)(token & (HOLO_TCP_CONCURRENT_LIMIT - 1u));
        struct slot *const slot = &owner->slots[index];
        if (token != 0 && (owner->waiting & slot_bit(index)) != 0 &&
            (owner->unarmed & slot_bit(index)) == 0 && slot->token == token &&
            (uintptr_t)slot->fd == event->ident &&
            event->filter == (slot->writing ? EVFILT_WRITE : EVFILT_READ)) {
            if ((event->flags & EV_ERROR) != 0) {
                const uint64_t code_word = event->data > 0 && event->data <= INT32_MAX
                                               ? (uint64_t)event->data : EIO;
                finish_wait(slot, HOLO_REFILL_ERROR_TAG | code_word);
            } else {
                /* EOF is readiness too. The worker still needs recv/send. */
                finish_wait(slot, 0);
            }
        }
    }
    return 0;
}

/* Create an independent close-on-exec kqueue and attach its fixed wake pipe.
 * Failure closes the queue before any worker receives ownership. */
static int open_queue(int wake)
{
    const int queue = kqueue();
    if (queue < 0) {
        return -1;
    }
    struct kevent event;
    EV_SET(&event, (uintptr_t)wake, EVFILT_READ, EV_ADD, 0, 0, NULL);
    if (fcntl(queue, F_SETFD, FD_CLOEXEC) == -1 ||
        kevent(queue, &event, 1, NULL, 0, NULL) == -1) {
        const int code = errno;
        (void)close(queue);
        errno = code;
        return -1;
    }
    return queue;
}
#endif

/* Run until shutdown or permanent readiness failure. The broker is the
 * only consumer of its pipe and queue, and never calls the Holo entry. */
static void *broker_main(void *argument)
{
    struct dispatch *const owner = argument;
    sync_ok(pthread_mutex_lock(&owner->mutex));
    while (!owner->stop_broker) {
        drain(owner->wake[0]);
#if defined(__APPLE__)
        const int code = owner->queue >= 0 ? queue_step(owner) : poll_step(owner);
#else
        const int code = poll_step(owner);
#endif
        if (code != 0) {
            fail_broker(owner, code);
            break;
        }
    }
    sync_ok(pthread_mutex_unlock(&owner->mutex));
    return NULL;
}

/* Reuse one worker stack across client entries. Assignment transfers sole
 * close authority. On return only the slot metadata may be accessed. */
static void *worker_main(void *argument)
{
    struct slot *const slot = argument;
    struct dispatch *const owner = slot->owner;
    current_slot = slot;
    sync_ok(pthread_mutex_lock(&owner->mutex));
    for (;;) {
        /* Shutdown still allows a final assigned stream to run and close.
         * An assigned parked entry waits for its FIFO permit before it starts. */
        while ((slot->fd < 0 && !owner->stop_workers) ||
               (slot->fd >= 0 && !may_run(slot))) {
            sync_ok(pthread_cond_wait(&slot->changed, &owner->mutex));
        }
        if (slot->fd < 0) {
            break;
        }
        const int fd = slot->fd;
        sync_ok(pthread_mutex_unlock(&owner->mutex));
        const int status = holo_tcp_entry(HOLO_REFILL_STREAM, (uint64_t)fd,
                                          HOLO_TCP_CONCURRENT_SELECTOR);
        sync_ok(pthread_mutex_lock(&owner->mutex));
        slot->fd = -1;
        release_slot(slot);
        if (status != 0) {
            mark_failure(owner, status);
        }
        sync_ok(pthread_cond_signal(&owner->capacity));
    }
    sync_ok(pthread_mutex_unlock(&owner->mutex));
    current_slot = NULL;
    return NULL;
}

/* The exported synchronous wait borrows no byte range and cannot outlive
 * the worker's current entry. Publish the arm before issuing its wake. */
uint64_t holo_tcp_concurrent_wait(int fd, bool writing)
{
    struct slot *const slot = current_slot;
    /* The selector is valid only while an admitted worker runs checked code. */
    if (slot == NULL) {
        __builtin_trap();
    }
    struct dispatch *const owner = slot->owner;
    const uint64_t bit = slot_bit(slot->index);
    sync_ok(pthread_mutex_lock(&owner->mutex));
    if (slot->fd != fd || (owner->waiting & bit) != 0 || !may_run(slot)) {
        __builtin_trap();
    }
    /* Exhaustion is explicit and cannot wrap into a stale generation. */
    const int failure = owner->broker_error != 0 ? owner->broker_error :
                        (owner->sequence == (UINTPTR_MAX >> SLOT_BITS) ? EOVERFLOW : 0);
    if (failure != 0) {
        sync_ok(pthread_mutex_unlock(&owner->mutex));
        return HOLO_REFILL_ERROR_TAG | (uint64_t)failure;
    }
    slot->token = (++owner->sequence << SLOT_BITS) | slot->index;
    slot->writing = writing;
    owner->waiting |= bit;
    if (owner->queue >= 0) {
        owner->unarmed |= bit;
    }
    notify(owner->wake[1]);
    release_slot(slot);
    /* Matching readiness or error first clears the live arm and makes the
     * slot eligible. Holo resumes only after it also regains a permit. */
    while ((owner->waiting & bit) != 0 || !may_run(slot)) {
        sync_ok(pthread_cond_wait(&slot->changed, &owner->mutex));
    }
    const uint64_t result = slot->result;
    sync_ok(pthread_mutex_unlock(&owner->mutex));
    return result;
}

/* Reserve admission capacity by observing a free slot. Only the calling
 * accept loop assigns slots, so it remains free until that loop publishes. */
static struct slot *await_capacity(struct dispatch *owner)
{
    sync_ok(pthread_mutex_lock(&owner->mutex));
    while (owner->failure == 0) {
        /* Bounded search over currently configured worker slots. */
        for (unsigned i = 0; i < owner->limit; ++i) {
            if (owner->slots[i].fd < 0) {
                sync_ok(pthread_mutex_unlock(&owner->mutex));
                return &owner->slots[i];
            }
        }
        sync_ok(pthread_cond_wait(&owner->capacity, &owner->mutex));
    }
    sync_ok(pthread_mutex_unlock(&owner->mutex));
    return NULL;
}

/* Wait for the nonblocking listener or failure notification. Retry
 * transient accept results without consuming a connection credit. */
static int accept_client(int listener, int stop_accept)
{
    struct pollfd fds[2] = {{.fd = listener, .events = POLLIN},
                            {.fd = stop_accept, .events = POLLIN}};
    for (;;) {
        const int ready = poll(fds, 2, -1);
        if (ready == -1 && errno == EINTR) {
            continue;
        }
        if (ready == -1 || fds[1].revents != 0) {
            return -1;
        }
        const int fd = accept(listener, NULL, NULL);
        if (fd != -1 || (errno != EINTR && errno != EAGAIN && errno != EWOULDBLOCK)) {
            return fd;
        }
    }
}

/* Transfer each admitted fd to a worker while the host still accepts
 * work. Closing a never-transferred fd is the host's responsibility. */
static void accept_entries(struct dispatch *owner, int listener, uint64_t count,
                           bool (*admit)(int))
{
    for (; count != 0; --count) {
        struct slot *const slot = await_capacity(owner);
        if (slot == NULL) {
            break;
        }
        const int fd = accept_client(listener, owner->stop_accept[0]);
        const bool valid = fd >= 0 && admit(fd);
        sync_ok(pthread_mutex_lock(&owner->mutex));
        if (owner->failure != 0 || !valid) {
            if (owner->failure == 0) {
                mark_failure(owner, 2);
                fputs(fd < 0 ? "could not accept concurrent TCP connection\n" :
                              "unsupported accepted TCP descriptor or mode\n", stderr);
            }
            sync_ok(pthread_mutex_unlock(&owner->mutex));
            if (fd >= 0) {
                (void)close(fd);
            }
            break;
        }
        slot->fd = fd;
        ready_slot(slot);
        sync_ok(pthread_mutex_unlock(&owner->mutex));
    }
}

/* After admission ends, allow transferred owners to finish naturally.
 * Keep the broker and pipes live until every worker has joined. */
static void stop_threads(struct dispatch *owner, unsigned started)
{
    sync_ok(pthread_mutex_lock(&owner->mutex));
    owner->stop_workers = true;
    for (unsigned i = 0; i < started; ++i) {
        sync_ok(pthread_cond_signal(&owner->slots[i].changed));
    }
    sync_ok(pthread_mutex_unlock(&owner->mutex));
    for (unsigned i = 0; i < started; ++i) {
        sync_ok(pthread_join(owner->slots[i].thread, NULL));
    }
    sync_ok(pthread_mutex_lock(&owner->mutex));
    owner->stop_broker = true;
    notify(owner->wake[1]);
    sync_ok(pthread_mutex_unlock(&owner->mutex));
    sync_ok(pthread_join(owner->broker, NULL));
}

/* Consume the listener and configure its optional execution permits. Each
 * setup stage has a matching release path, including partial worker startup. */
static int run_concurrent(int listener, uint64_t count, unsigned limit,
                          unsigned active, bool use_kqueue, bool (*admit)(int))
{
    struct dispatch owner = {.queue = -1};
    const int listener_flags = fcntl(listener, F_GETFL);
    int status = 2;
    unsigned prepared = 0;
    bool started_all = false;
    /* Validate bounds before starting threads or transferring ownership. */
    if (count == 0 || limit == 0 || limit > HOLO_TCP_CONCURRENT_LIMIT || active > limit ||
        listener_flags == -1 || fcntl(listener, F_SETFL, listener_flags | O_NONBLOCK) == -1 ||
        pthread_mutex_init(&owner.mutex, NULL) != 0) {
        goto listener_only;
    }
    owner.limit = count < limit ? (unsigned)count : limit;
    owner.permits.limit = active < owner.limit ? active : owner.limit;
    if (pthread_cond_init(&owner.capacity, NULL) != 0) {
        goto mutex_only;
    }
    if (!control_pipe(owner.wake)) {
        goto capacity_only;
    }
    if (!control_pipe(owner.stop_accept)) {
        goto wake_only;
    }
    if (use_kqueue) {
#if defined(__APPLE__)
        owner.queue = open_queue(owner.wake[0]);
        if (owner.queue < 0) {
            goto pipes_only;
        }
#else
        goto pipes_only;
#endif
    }
    /* Prepare a fully initialized fixed table before launching any thread. */
    for (; prepared < owner.limit; ++prepared) {
        struct slot *const slot = &owner.slots[prepared];
        slot->owner = &owner;
        slot->index = prepared;
        slot->fd = -1;
        if (pthread_cond_init(&slot->changed, NULL) != 0) {
            goto slots_only;
        }
    }
    if (pthread_create(&owner.broker, NULL, broker_main, &owner) != 0) {
        goto slots_only;
    }
    unsigned started = 0;
    /* On partial startup, stop exactly the threads successfully started. */
    for (; started < owner.limit; ++started) {
        struct slot *const slot = &owner.slots[started];
        if (pthread_create(&slot->thread, NULL, worker_main, slot) != 0) {
            break;
        }
    }
    started_all = started == owner.limit;
    if (started_all) {
        accept_entries(&owner, listener, count, admit);
    }
    (void)close(listener);
    listener = -1;
    stop_threads(&owner, started);
    if (started_all) {
        status = owner.failure;
    }
slots_only:
    /* Destroy only initialized objects after the last thread has joined. */
    for (unsigned i = 0; i < prepared; ++i) {
        sync_ok(pthread_cond_destroy(&owner.slots[i].changed));
    }
    if (owner.queue >= 0) {
        (void)close(owner.queue);
    }
pipes_only:
    (void)close(owner.stop_accept[0]);
    (void)close(owner.stop_accept[1]);
wake_only:
    (void)close(owner.wake[0]);
    (void)close(owner.wake[1]);
capacity_only:
    sync_ok(pthread_cond_destroy(&owner.capacity));
mutex_only:
    sync_ok(pthread_mutex_destroy(&owner.mutex));
listener_only:
    if (listener >= 0) {
        (void)close(listener);
    }
    if (!started_all) {
        fputs("could not start concurrent TCP host\n", stderr);
    }
    return status;
}

/* Run the original concurrent worker pool without a permit limit. */
int holo_tcp_run_concurrent(int listener, uint64_t count, unsigned limit,
                            bool use_kqueue, bool (*admit)(int))
{
    return run_concurrent(listener, count, limit, 0, use_kqueue, admit);
}

/* Run the parked profile with separate bounds for live and executing entries.
 * The caller transfers its listener even if the bounds are invalid. */
int holo_tcp_run_parked(int listener, uint64_t count, unsigned live, unsigned active,
                        bool use_kqueue, bool (*admit)(int))
{
    /* A zero permit budget cannot admit any checked work. */
    if (active == 0) {
        (void)close(listener);
        return 2;
    }
    return run_concurrent(listener, count, live, active, use_kqueue, admit);
}
