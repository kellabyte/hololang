# macOS socket backend probes

These standalone C probes compare readiness mechanisms for the experimental
[Holo socket library](../../docs/packages.md#socket-library). Holo programs
use the separate [hosted runtime](../refill/README.md). Run the probes directly
with the commands below.

| Source | Scope |
|---|---|
| [macos.c](macos.c) | Unix and IPv4 loopback TCP comparisons of `poll`, persistent `kqueue` with `kevent`, and persistent `kqueue` with `kevent64`. |
| [macos_streams.c](macos_streams.c), [macos_streams.h](macos_streams.h) | Private loopback setup, timing support, and an untimed TCP short-write and writer-interest fixture. |
| [macos_churn.c](macos_churn.c) | Untimed TCP descriptor-number reuse and stale readiness rejection with persistent kqueue and poll controls. |

## macOS

The default mode opens nonblocking Unix socketpairs. TCP mode creates
connected pairs on an ephemeral IPv4 loopback listener with `TCP_NODELAY`.
Each workload sends and checks the same numbered 16-byte records across
the three readiness backends. TCP mode also rotates bursts of 16 records
on eight active streams out of 128 and assembles fragmented TCP input before
checking each record. Socket creation and registration are measured
separately. The steady interval includes send, readiness, receive, and
validation. Kqueue registrations are persistent. Poll revisits its
descriptor array each wait. Readiness permits another attempt, and only
a zero-byte receive confirms input EOF after queued bytes have been read.

```sh
mkdir -p build/benchmarks/socket-macos-qualification
xcrun clang -std=c17 -D_DARWIN_C_SOURCE -O3 -Wall -Wextra -Wpedantic -Werror \
  runtime/socket/macos.c runtime/socket/macos_streams.c \
  -o build/benchmarks/socket-macos-qualification/socket-macos
build/benchmarks/socket-macos-qualification/socket-macos \
  > build/benchmarks/socket-macos-qualification/unix.csv
build/benchmarks/socket-macos-qualification/socket-macos --tcp-check
build/benchmarks/socket-macos-qualification/socket-macos --tcp \
  > build/benchmarks/socket-macos-qualification/tcp.csv
```

TCP modes require loopback bind permission. Both transports need room for
256 stream endpoints and the control descriptors. Keep competing measurements
idle. Each row validates its I/O phase and checks half-close after the clock
stops. The shorter `--tcp-check` also induces write backpressure in a separate
fixture. It retains the unsent suffix, enables writer interest on would-block,
and verifies exact delivery and interest removal after progress.
[Apple kqueue manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/kqueue.2.html) ·
[TCP qualification](../../evidence/socket-tcp-qualification-20261112/README.md) ·
[Initial Unix qualification](../../evidence/socket-library-20261112/README.md)

### Descriptor reuse and writer rearm

The churn fixture captures native read and write readiness, releases the
stream, and opens a replacement at the same descriptor number. A saved
event must match the live stream generation and current write arm. The
fixture also runs the shared short-write oracle with both backends.
It requires ephemeral IPv4 loopback permission.

```sh
mkdir -p build/benchmarks/socket-macos-churn-qualification
xcrun clang -std=c17 -D_DARWIN_C_SOURCE -O3 -Wall -Wextra -Wpedantic -Werror \
  runtime/socket/macos_churn.c runtime/socket/macos_streams.c \
  -o build/benchmarks/socket-macos-churn-qualification/socket-macos-churn
build/benchmarks/socket-macos-churn-qualification/socket-macos-churn
```

The separate checked [hosted TCP entry](../refill/README.md#experimental-hosted-io)
can use one queue across successive clients with one live owner at a time.
Its maintained tests exercise forced reuse and stale event rejection through
the compiler's checked boundary. The standalone churn fixture remains an
independent control.
