# TCP echo

The basic demo listens on IPv4 loopback, receives one byte from a client, echoes
it, and closes the connection. It uses the same initialized byte for receiving
and sending. The demo is local and plaintext. It has no TLS or application
protocol, and the basic entry handles one byte per connection.

## Run it

From the repository root, build with matched LLVM `opt` and `clang` and a system
linker. [CLI setup](../../cli/README.md)

```sh
mkdir -p build/examples
demo_dir=$(mktemp -d build/examples/tcp-echo.XXXXXX)
cargo run --offline --locked -p holo -- build --strict-names \
  --locked --release --package examples/tcp_echo/holo.toml -o "$demo_dir/tcp-echo"
"$demo_dir/tcp-echo" accept-loopback 6381
```

In another terminal, run `printf A | nc 127.0.0.1 6381`. The client receives
`A`, and the server prints the send count and close status. Use port `0` to
let the host choose and print an ephemeral port. To serve three successive
clients in the same process, use `accept-loopback-many 6381 3`. macOS also
supports the experimental `accept-loopback-kqueue` and
`accept-loopback-kqueue-many` modes.

## Streaming and backpressure

[`stream.holo`](stream.holo) echoes a binary stream until the peer closes
its write side. It keeps one bounded receive window and resumes short writes
from the unsent suffix. Build the standalone source to try concurrent clients.

```sh
cargo run --offline --locked -p holo -- build --release \
  examples/tcp_echo/stream.holo -o "$demo_dir/tcp-stream"
"$demo_dir/tcp-stream" accept-loopback-concurrent 6381 8 4
```

This serves eight clients with at most four checked entries active. The
`accept-loopback-parked 6381 8 4 1` mode gives four live slots one execution
permit, so a stream waiting for readiness can yield its permit to another
entry. macOS also offers kqueue variants. [Hosted TCP specification](../../docs/packages.md#owned-tcp-capabilities) · [Readiness parking](../../runtime/refill/README.md#readiness-parking)

The remaining workers exercise short writes and backpressure.

- [`slow_reader.holo`](slow_reader.holo) sends a 4 MiB reply to a tagged request
  and reports short-write and would-block counts.
- [`mixed_readers.holo`](mixed_readers.holo) serves either a 1 KiB fast reply or
  a 4 MiB slow reply from the same entry.
- [`streaming_readers.holo`](streaming_readers.holo) serves the same replies
  using a reusable 64 KiB window for the slow reply.
