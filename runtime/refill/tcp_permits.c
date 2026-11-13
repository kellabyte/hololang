/*
 * FIFO execution reservations used only by the optional TCP parking profile.
 * One request or release can change available capacity by at most one. The
 * existing dispatcher mutex serializes these operations. This table allocates
 * no memory and stores slot indices, never descriptors or Holo addresses.
 */

#include "tcp_permits.h"

/* The fixed capacity is a power of two so wrapping needs only a mask. */
#define FIFO_MASK (HOLO_TCP_CONCURRENT_LIMIT - 1u)
_Static_assert((HOLO_TCP_CONCURRENT_LIMIT & FIFO_MASK) == 0, "FIFO capacity");
_Static_assert(HOLO_TCP_CONCURRENT_LIMIT <= 64u, "permit bitset capacity");

/* Guard the shift and reject a corrupt slot before changing the table. */
static uint64_t bit_for(unsigned slot)
{
    if (slot >= HOLO_TCP_CONCURRENT_LIMIT) {
        __builtin_trap();
    }
    return UINT64_C(1) << slot;
}

/* Reserve the oldest pending permit, if any. Each operation leaves either
 * all permits in use or the FIFO empty, so a single grant suffices. */
static unsigned grant(struct holo_tcp_permits *permits)
{
    if (permits->pending == 0 || permits->active == permits->limit) {
        return HOLO_TCP_NO_PERMIT;
    }
    const unsigned slot = permits->fifo[permits->head];
    const uint64_t bit = bit_for(slot);
    permits->head = (permits->head + 1u) & FIFO_MASK;
    --permits->pending;
    permits->queued &= ~bit;
    ++permits->active;
    permits->held |= bit;
    return slot;
}

/* Enqueue one ready entry and return any reservation made by this request. */
unsigned holo_tcp_permit_request(struct holo_tcp_permits *permits, unsigned slot)
{
    const uint64_t bit = bit_for(slot);
    /* A duplicate request or disabled table contradicts host protocol. */
    if (permits->limit == 0 || permits->pending >= HOLO_TCP_CONCURRENT_LIMIT ||
        ((permits->held | permits->queued) & bit) != 0) {
        __builtin_trap();
    }
    permits->fifo[(permits->head + permits->pending) & FIFO_MASK] = slot;
    ++permits->pending;
    permits->queued |= bit;
    return grant(permits);
}

/* Drop one active reservation and hand it to the oldest ready slot. */
unsigned holo_tcp_permit_release(struct holo_tcp_permits *permits, unsigned slot)
{
    const uint64_t bit = bit_for(slot);
    /* Never allow a parked entry to release capacity it does not own. */
    if ((permits->held & bit) == 0 || permits->active == 0) {
        __builtin_trap();
    }
    permits->held &= ~bit;
    --permits->active;
    return grant(permits);
}
