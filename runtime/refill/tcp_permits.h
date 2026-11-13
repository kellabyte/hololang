/*
 * Bounded FIFO execution permits for readiness-parked TCP entries.
 * The concurrent host guards this table with its existing broker mutex.
 * Entries retain their own stack and stream while waiting for a permit.
 */

#ifndef HOLO_EXPERIMENTAL_TCP_PERMITS_H
#define HOLO_EXPERIMENTAL_TCP_PERMITS_H

#include "tcp_concurrent.h"

/* A request or release grants at most one slot. This sentinel grants none. */
#define HOLO_TCP_NO_PERMIT HOLO_TCP_CONCURRENT_LIMIT

/* Fixed table for all potentially live worker slots. Zero limit is disabled. */
struct holo_tcp_permits {
    unsigned fifo[HOLO_TCP_CONCURRENT_LIMIT]; /* Ready slots in publication order. */
    unsigned head;      /* First occupied position if pending is nonzero. */
    unsigned pending;   /* Queued requests, excluding already held permits. */
    unsigned active;    /* Permits reserved for checked entries. */
    unsigned limit;     /* Positive bound in parked mode, zero otherwise. */
    uint64_t queued;    /* Duplicate-request guard for the FIFO. */
    uint64_t held;      /* Permit remains reserved until its owner releases it. */
};

/* Mark a newly ready slot and reserve the next available permit in FIFO order.
 * Caller holds the broker mutex and passes a valid slot with no existing
 * request or permit. Returns a granted slot to signal, or the sentinel. */
unsigned holo_tcp_permit_request(struct holo_tcp_permits *permits, unsigned slot);

/* Relinquish a currently held permit under the broker mutex. If any ready
 * slot is waiting, reserve the released permit for its oldest request. */
unsigned holo_tcp_permit_release(struct holo_tcp_permits *permits, unsigned slot);

#endif
