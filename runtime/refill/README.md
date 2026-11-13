# Experimental hosted I/O

This directory owns the process and POSIX boundary for checked hosted Holo
entry code. Ordinary Holo executables retain their existing direct entry.

- [host.c](host.c) accepts `file PATH` or `stream-fd FD`, admits one descriptor,
  lends it to the generated entry, and closes it on normal completion. Setup
  errors exit with status 2. File opens request nonblocking mode so an invalid
  FIFO cannot hang before admission. A regular file read may still block.
- [refill.c](refill.c) admits readable regular files and connected nonblocking
  byte-stream sockets with zero receive timeout. Each refill issues one bounded
  `read` or `recv` directly into the existing initialized slot. Each explicit
  wait issues one `poll` on that descriptor and reads no bytes.
- [refill.h](refill.h) declares the internal native ABI and status encoding.
- [send.c](send.c) adds one bounded `send` and one explicit writable wait.
  Output builds pass `-DHOLO_HOSTED_OUTPUT` to compile the adapter and enable
  the extra stream setup in the host.
- [session.c](session.c) owns one IPv4 loopback listener and one admitted
  client at a time. Session-aware builds pass `-DHOLO_HOSTED_SESSION` to
  compile the adapter and enable the host's `listen-loopback PORT` mode.
- [tcp.c](tcp.c) provides a separate native entry that admits IPv4 loopback
  TCP connections and transfers each one's close authority to the checked
  entry. It also exposes the terminal close status.
- [tcp_wait.c](tcp_wait.c) and [tcp_wait.h](tcp_wait.h) provide the accepted
  TCP profile's explicit waits. Poll remains the default and macOS permits
  an explicitly selected kqueue.
- [tcp_concurrent.c](tcp_concurrent.c) and [tcp_concurrent.h](tcp_concurrent.h)
  own the opt-in worker pool and shared readiness broker.

The launcher keeps the inherited descriptor stable for the session. Duplicates
may share the same open file description. They must leave its mode, receive
timeout, and local receive half unchanged and refrain from competing reads.
Peer closure can report EOF after queued bytes have been read.

A positive result publishes the exact fresh extent in the data arm. Zero
means EOF for refill or readiness for wait. The highest two unsigned values
represent interruption and stream would-block respectively. Other recoverable
failures encode a positive native errno under bit 63. Readiness can race. Poll
hangup and error wake the caller so the next input attempt can report the actual
result. The caller chooses when to wait or retry. Synchronous I/O borrows the
existing checked range in place and keeps retries explicit. The shipped
`holo.net` helpers decode the native statuses in manifest packages.

An output-capable entry uses `let sent = core.send(reply.bytes, offset, count)`.
It sends the checked positive subrange of an initialized field through the
current nonblocking stream. A positive status is exact progress, and the caller
keeps the source live while sending any remaining suffix. `core.wait_output()`
performs one explicit `POLLOUT` wait. A ready status permits another attempt,
including after hangup or error. Both calls on a file report tagged
`ENOTSOCK`. There is no staging, implicit retry, or stdout fallback.

The output profile requires disabled send timeout and linger. The host sets
`SO_NOSIGPIPE` on Darwin, and Linux sends use `MSG_NOSIGNAL`. A peer closure
can therefore report `EPIPE` as a status. Descriptor aliases must preserve
admitted properties and refrain from competing writes. [Apple socket options](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/getsockopt.2.html) · [Linux send](https://man7.org/linux/man-pages/man2/sendmsg.2.html)

`listen-loopback PORT` binds only IPv4 `127.0.0.1`. The native host accepts
one client before entering Holo and configures each accepted descriptor as
nonblocking and close-on-exec. After the refill data arm has ended,
`let next = core.next_session()` closes the current client and attempts one
blocking accept. Zero admits the next client, interruption and errno remain
explicit, and code must advance successfully before another listener I/O
attempt. File and inherited-stream modes return 1 with no ownership change.
Holo controls protocol state and storage across sessions. The host closes the
last client and listener at normal entry return.

The experimental lexical TCP entry uses `accept-loopback PORT`. It opens a
real TCP listener on `127.0.0.1`, accepts once, and closes the listener. The
accepted socket must satisfy the existing nonblocking, timeout, linger, and
output signal checks before ownership moves to Holo. Setup failure exits
with status 2 while the host owns cleanup. With explicit port zero, the host
prints and flushes one line containing the assigned port before accepting.
A fixed-port invocation leaves stdout to the application.

`accept-loopback-many PORT COUNT` retains the listener across a finite
sequence of checked entry calls. `COUNT` is a positive decimal `u64`.
The next accept follows the previous entry's return and cleanup. Each call
has fresh entry-local bindings and storage. A setup or admission failure
stops the sequence with status 2. Other entry statuses stop the host with
that status. The host keeps the listener and releases it after the loop.

On macOS, `accept-loopback-kqueue PORT` selects the experimental queue
while preserving the same admission and byte-range checks. The host owns
the readiness queue independently. `accept-loopback-kqueue-many PORT COUNT`
keeps one queue across admissions and attaches each stream before transferring
close authority. Darwin removes a stream's filters at close. Waits check
the returned descriptor, filter, and a process-unique, nonwrapping arm token.
The token advances across clients, including when accept reuses a descriptor
number. Delivery disables the direction until the next explicit wait. Poll
remains the default. Both readiness paths share the synchronous `send` and
`recv` adapters.
[Apple kqueue manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/kqueue.2.html)

The checked owner releases only the accepted descriptor. Explicit close
returns zero or tagged positive errno, including a reported interruption,
and consumes the owner for either outcome. Implicit normal-exit cleanup
keeps the selected process status. Native close is attempted once because
an error can arrive after the descriptor number becomes available for reuse.
[Lexical ownership specification](../../docs/packages.md#owned-tcp-capabilities)

## Concurrent loopback

`accept-loopback-concurrent PORT COUNT LIMIT` uses shared poll.
Darwin also offers `accept-loopback-kqueue-concurrent PORT COUNT LIMIT`.
Both modes accept a finite positive connection count and a limit from 1
through 64. The host keeps the listener, one broker thread, and at most
`min(COUNT, LIMIT)` native worker threads. Each active worker invokes
the same checked Holo entry with one stream. It consumes that stream
before the worker slot is reused. The host owns admission and the broker.
There is no shared Holo entry state or transfer of a stream between entries.

Each explicit wait submits one synchronous read or write interest to
the shared broker. It sleeps on its worker condition until the broker
publishes readiness or an error. The broker wakes on the control channel,
routes only events matching the current descriptor and arm, and borrows
no byte range. Send, receive, and close still run through the checked
entry's ordinary native adapters. This profile adds native thread stacks,
mutex/condition synchronization, and broker work.

The broker drains a nonblocking control pipe under its mutex. Poll takes
a snapshot of the pending arms. Darwin installs one-shot filters and
checks registration receipts before waiting. The broker waits for readiness
outside the mutex. An interruption of the broker syscall is retried by
the broker. A terminal broker error stops admission and returns a tagged
error to pending workers. Entries already handed a stream retain close
authority and can finish. Ordinary entry output can interleave across workers.
[Concurrent ownership boundary](../../docs/packages.md#concurrent-hosted-entries)

## Readiness parking

`accept-loopback-parked PORT COUNT LIVE ACTIVE` uses shared poll. Darwin
also offers `accept-loopback-kqueue-parked PORT COUNT LIVE ACTIVE`.
These experimental modes require `1 <= ACTIVE <= LIVE <= 64`. The host
prepares `min(COUNT, LIVE)` native worker stacks, as in concurrent mode.
The new bound caps permits for checked execution independently of how
many connections are live. The other host modes bypass this permit table.

The [permit table](tcp_permits.c) reserves permits in FIFO order of
admission or matched readiness publication. A worker holds one before
entering checked Holo. At an explicit wait, the worker publishes its
readiness arm and relinquishes its permit under the same broker mutex.
It sleeps until the exact arm resolves and it regains a permit. This
keeps the lexical call stack, Holo backing, and sole stream owner on
the same native thread. The broker carries no payload pointer or
destructor authority. A completed entry releases its permit only after
its normal lexical cleanup has run.

The bound covers admitted entries running between explicit waits or
completion, including reservations to newly runnable workers. It
does not bound process thread count below `LIVE`, reclaim sleeping
native stacks, or yield a permit for arbitrary computation or another
native operation. It adds FIFO and permit work at admission, waits,
and return. Broker or admission failure follows the concurrent
host's stop-and-drain behavior. The checked entry ABI is unchanged.

Run `cargo test --offline --test hosted_refill` from the repository root for
deterministic call/result witnesses and checked-source validation. To run the
two-client TCP test on a host allowed to bind loopback, use
`cargo test --offline --test hosted_refill hosted_listener_replaces_client_and_keeps_holo_state -- --ignored`.
Use `cargo test --offline --test hosted_tcp` for lexical lifecycle,
mid-sequence admission failure, and stale event injection. Use
`cargo test --offline --test hosted_tcp -- --ignored` on a host that permits
loopback bind to exercise successive clients, concurrent slot reuse, and
short writes. The parked witnesses use one permit to keep multiple owners
live and to complete a fast reply while a slow reply remains unread. The
reset witness repeats real abortive and orderly closes across parked slots.
