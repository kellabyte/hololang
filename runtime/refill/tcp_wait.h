/*
 * Private readiness boundary for the lexical hosted TCP profile. The host
 * owns any event queue, and a wait borrows only its descriptors. The ABI
 * reports the same scalar status as the original single-stream poll waits.
 */

#ifndef HOLO_EXPERIMENTAL_TCP_WAIT_H
#define HOLO_EXPERIMENTAL_TCP_WAIT_H

#include <stdint.h>

/*
 * Create an owned queue and attach its first admitted live stream fd. Return
 * -1 and set errno on setup failure or on an unsupported target. The host
 * closes the queue after all checked entries finish.
 */
int holo_tcp_queue_open(int fd);

/*
 * Attach one live stream after the previous stream was closed and its filters
 * removed. Leave both directions disabled. Return zero or -1 with errno. The
 * caller retains sole ownership of both descriptors, including on failure.
 */
int holo_tcp_queue_attach(int queue, int fd);

/*
 * Wait once for reading (writing = 0) or writing (writing = 1). UINT64_MAX
 * selects the original poll backend, and the concurrent selector chooses a
 * broker wait. A native queue descriptor otherwise belongs to the serial host.
 * Returns zero for readiness, interruption or tagged native errno. The
 * operation may block and retains no buffer.
 */
uint64_t holo_tcp_wait_experimental(uint64_t fd, uint64_t queue, uint64_t writing);

#endif
