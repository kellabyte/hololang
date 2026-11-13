/*
 * Private stream utilities for the macOS socket probes. The caller owns
 * returned descriptors. Transfer buffers stay with their callers, and
 * no allocation is added to the readiness loop.
 */

#ifndef HOLO_MACOS_STREAMS_H
#define HOLO_MACOS_STREAMS_H

#include <netinet/in.h>
#include <stdint.h>

/* End the probe with the current errno and the supplied operation label. */
_Noreturn void fail_system(const char *operation);

/* End the probe when the supplied workload invariant fails. */
_Noreturn void fail_check(const char *problem);

/* Read the monotonic wall clock in nanoseconds or end the probe on failure. */
uint64_t now_ns(void);

/* Set the caller's descriptor nonblocking, retaining other status flags. */
void set_nonblocking(int fd);

/* Return an owned IPv4 loopback listener and its kernel-selected address. */
int open_loopback(struct sockaddr_in *address);

/*
 * Return two owned TCP descriptors through pair. A positive buffer limit
 * configures the writer and reader for the backpressure fixture.
 */
void open_tcp_pair(int listener, const struct sockaddr_in *address,
                   int pair[2], int buffer_limit);

/* Run untimed short-write and writer rearm checks on both readiness controls. */
void verify_tcp_writers(void);

#endif
