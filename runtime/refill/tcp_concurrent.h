/*
 * Private host boundary for bounded concurrent checked TCP entries. Each
 * native worker retains its own Holo ownership and borrows the shared broker
 * only while waiting. The broker handles readiness metadata, not byte ranges.
 */

#ifndef HOLO_EXPERIMENTAL_TCP_CONCURRENT_H
#define HOLO_EXPERIMENTAL_TCP_CONCURRENT_H

#include <stdbool.h>
#include <stdint.h>

/* Distinguish a broker wait from a direct poll or native queue descriptor. */
#define HOLO_TCP_CONCURRENT_SELECTOR (UINT64_MAX - UINT64_C(1))
/* Bound workers and wait-token slots independently of total connections. */
#define HOLO_TCP_CONCURRENT_LIMIT 64u

/*
 * Consume an IPv4 listener and run at most limit reusable checked workers.
 * Admit each fd before handing sole close authority to its entry. A setup or
 * entry failure stops admission and drains already transferred owners.
 * Return 2 for native setup failure or the first nonzero checked status.
 */
int holo_tcp_run_concurrent(int listener, uint64_t count, unsigned limit,
                            bool use_kqueue, bool (*admit)(int));

/*
 * Consume a listener with independent live and checked-execution limits.
 * A matching readiness wait temporarily releases the execution permit.
 * The entry stack, backing, and descriptor remain owned by that worker.
 * Startup, admission, and entry failures follow the concurrent host rules.
 */
int holo_tcp_run_parked(int listener, uint64_t count, unsigned live, unsigned active,
                        bool use_kqueue, bool (*admit)(int));

/*
 * Borrow the calling worker's live fd for one readiness wait. Return zero or
 * tagged native errno. This may sleep on the worker's condition. No buffer is
 * borrowed and no ownership crosses into the dispatcher.
 */
uint64_t holo_tcp_concurrent_wait(int fd, bool writing);

#endif
