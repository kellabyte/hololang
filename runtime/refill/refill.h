/*
 * Internal C boundary for the experimental hosted I/O profile.
 * A host lends one admitted descriptor and one already initialized writable
 * slot. Each attempt returns a scalar outcome and uses the slot in place.
 */

/* Keep repeated fixture and runtime includes compatible. */
#ifndef HOLO_EXPERIMENTAL_REFILL_H
#define HOLO_EXPERIMENTAL_REFILL_H

#include <stdbool.h>
#include <stdint.h>

/* Admitted readable regular file. */
#define HOLO_REFILL_FILE UINT64_C(0)
/* Admitted connected nonblocking byte-stream socket with disabled timeout. */
#define HOLO_REFILL_STREAM UINT64_C(1)
/* Host-owned IPv4 loopback listener with a single current client. */
#define HOLO_REFILL_LISTENER UINT64_C(2)
/* Terminal next-session result from a file or inherited stream. */
#define HOLO_SESSION_END UINT64_C(1)
/* Largest admitted native read or receive count on this profile. */
#define HOLO_REFILL_MAX_COUNT UINT64_C(2147483647)

/* Successful positive-size attempt with a native result of zero. */
#define HOLO_REFILL_END UINT64_C(0)
/* Signal interruption reported by the native read or receive. */
#define HOLO_REFILL_INTERRUPTED UINT64_MAX
/* Nonblocking stream would block according to the native result. */
#define HOLO_REFILL_WOULD_BLOCK (UINT64_MAX - UINT64_C(1))
/* Base tag OR-ed with a positive native int errno to report other errors. */
#define HOLO_REFILL_ERROR_TAG (UINT64_C(1) << 63)

/*
 * Validate a trusted host's proposed descriptor kind and readable mode.
 * Returns true on admission and false for an unsupported kind, descriptor,
 * or mode or a failed inspection. This may call the OS several times without
 * consuming input or taking ownership. The host keeps the admitted descriptor
 * open and its type, mode, peer and timeout valid for the session.
 */
bool holo_refill_admit_experimental(uint64_t kind, uint64_t fd);

/*
 * Try a single synchronous read into slot[0..limit) for the admitted kind/fd.
 * The caller provides the physical initialized_extent and keeps that storage
 * writable for this call. A count in 1..limit exposes exactly that fresh
 * prefix. Other legal outcomes use the tags above and expose no view. Invalid
 * arguments and impossible native results trap. The function lends no pointer
 * and retains neither slot nor fd. It stages no payload and acquires no Holo
 * backing. The underlying native call may block on a regular file.
 */
uint64_t holo_refill_experimental(uint64_t kind, uint64_t fd, uint8_t *slot,
                                  uint64_t initialized_extent, uint64_t limit);

/*
 * Call poll once on the admitted descriptor without consuming input. Returns
 * zero for an event that permits another read, Interrupted for EINTR, or
 * tagged errno. This is an explicit potentially blocking operation.
 */
uint64_t holo_wait_input_experimental(uint64_t fd);

/*
 * Verify additional output assumptions for an admitted descriptor and set
 * the per-socket Darwin SIGPIPE policy at setup. A file needs no setup.
 * False keeps the host from publishing the descriptor to Holo.
 */
bool holo_send_configure_experimental(uint64_t kind, uint64_t fd);

/*
 * Attempt one send of count bytes from the initialized suffix and return
 * exact positive progress, interruption, would-block, or tagged errno.
 * The synchronous call retains no storage. A file receives ENOTSOCK and
 * supplies no bytes to the OS. Invalid arguments/results trap.
 */
uint64_t holo_send_experimental(uint64_t kind, uint64_t fd, const uint8_t *bytes,
                                uint64_t initialized_extent, uint64_t count);

/*
 * Poll once for writable readiness on an admitted stream. Return zero for
 * an event that permits another send, interruption, or tagged errno.
 * This explicitly blocking call does not read or send bytes.
 */
uint64_t holo_wait_output_experimental(uint64_t kind, uint64_t fd);

/*
 * Perform one terminal release of an admitted lexical TCP descriptor.
 * Returns zero or tagged positive errno, including EINTR. The caller must
 * consume its owner before the call and never retry or reuse the fd.
 */
uint64_t holo_tcp_close_experimental(uint64_t fd);

/*
 * Resolve the current descriptor for a synchronous operation. The original
 * descriptor is returned for a file or inherited stream. Invalid state traps.
 */
uint64_t holo_session_fd_experimental(uint64_t kind, uint64_t fd);

/*
 * Close the active client, if any, and attempt one blocking accept. Listener
 * mode returns zero on success, Interrupted, or a tagged errno. File/stream
 * modes return End with their descriptor still held by the entry host.
 */
uint64_t holo_next_session_experimental(uint64_t kind, uint64_t fd);

/*
 * Own an IPv4 loopback listener for decimal port_text and run checked Holo
 * after the first client is admitted. Close all owned descriptors on return.
 * Setup failures return 2. No application payload is read by the listener.
 */
int holo_run_listener_experimental(const char *port_text);

#endif
