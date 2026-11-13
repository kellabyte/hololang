# Hosted telemetry worker

This example reads a file or a connected byte stream, counts complete telemetry
frames, and sums the bytes in accepted payloads. Holo keeps the decoder state
in one fixed record. Each read writes into the same 4,096-byte inline slot and
lends its initialized prefix until the decoder call returns.

## Run it

Run these commands from the repository root with matched LLVM tools installed.

```sh
mkdir -p build/examples
python3 -c 'from pathlib import Path; Path("build/examples/telemetry.bin").write_bytes(bytes([165, 0, 1, 0, 42]))'
cargo run --offline -p holo -- build --locked --release \
  --package tests/fixtures/telemetry_worker/holo.toml -o build/examples/telemetry-worker
build/examples/telemetry-worker file build/examples/telemetry.bin
```

This input yields a clean end of input, five bytes in one positive read, one
hit, and a checksum of 42. For a stream, launch the executable with
`stream-fd FD` and pass an already connected nonblocking stream socket with
receive timeout disabled and a descriptor number of at least 3. The worker
process owns and closes its descriptor. Other copies of that descriptor must
leave its mode, timeout, and receive side unchanged and avoid competing reads.
The peer can finish input by closing its write side.
An open, idle peer can keep the worker waiting indefinitely. The hosted
adapter targets macOS and Linux. This example was verified on macOS arm64.

## Wire format

| Offset | Field | Rule |
| ---: | --- | --- |
| 0 | Magic | The octet `0xA5` identifies this format. |
| 1 | Kind | `0` is a hit, `1` a miss, and `2` an error event. |
| 2–3 | Payload length | An unsigned, little-endian length. This worker accepts 0–4,096 bytes. |
| 4 onward | Payload | Exactly the declared number of octets. |

Read boundaries may occur inside any field. The decoder reads a contiguous
length through an exact two-byte view. When a read splits the length, it
retains the low octet and combines it with the high octet on the next read.
The decoder stores copied header octets, the number of body bytes left, and a
provisional checksum. It publishes a frame only after consuming the full body.
Rejected complete frames increment one counter, in priority order of oversized
body, invalid magic, and unknown kind. The worker drains a rejected frame's
declared body and resumes at the next header. At EOF an incomplete suffix
records one truncation and contributes neither a frame nor a checksum. The
counters and sums use unsigned 64-bit wrapping arithmetic.

## Output

The program prints one unsigned decimal value per line, in this order.

| Line | Value |
| ---: | --- |
| 1 | Terminal outcome. Zero is EOF. A terminal native error sets bit 63 and places the errno in the low bits. |
| 2 | Bytes consumed. |
| 3 | Positive reads. This can vary with stream segmentation. |
| 4 | Hits. |
| 5 | Misses. |
| 6 | Error events from the telemetry protocol. |
| 7 | Sum of the payload octets in all accepted complete frames. |
| 8 | Complete frames with invalid magic. |
| 9 | Complete frames with unknown kind. |
| 10 | Complete frames with oversized bodies. |
| 11 | EOF truncation, either zero or one. |

The process exits successfully after reporting a terminal outcome. Check line 1
for input errors and line 11 for EOF truncation. Host admission errors use stderr
and a nonzero exit status. A failure to obtain any initial bounded region traps
before the worker reads input.

## Package layout

The [manifest](holo.toml) selects [`src/main.holo`](src/main.holo) and declares
the local `holo` dependency. [`telemetry.frame`](src/frame/README.md) spans
two files. Its validator calls a namespace-private signature helper from
the decoding file. The decoder selects [`telemetry.wire`](src/wire.holo) with
`using` to combine the octets of a split length. The entry selects
[`holo.bytes.read`](../../../lib/holo/bytes/read/cursor.holo) for its cursor.

`--locked` verifies the selected dependency version and source content
against [`holo.lock`](holo.lock). Refresh the lock after reviewing an intended
dependency change.

```sh
cargo run --offline -p holo -- lock \
  --package tests/fixtures/telemetry_worker/holo.toml
```

## Implementation and verification

`main.holo` owns the decoder state and retry policy. `decode_chunk` takes a
checked readonly reference to the current prefix and an explicit writable
reference to parser state. It resets a reusable cursor for each prefix.
`read.take_u16_le` advances by two bytes when the length field fits. If one
byte remains, `Short(2)` leaves the cursor at the low octet. The decoder
copies that octet and completes the field with the first byte of the next
refill. The decoder and its byte helper carry checked `noalloc` and `noblock`
contracts. `main` retries interruptions and waits for readiness after
would-block. Each refill makes one native read attempt and reuses its input
slot. The worker reads accepted payload bytes in place and drains rejected
bodies by their extents.

`verify.py` compares the executable with an independent whole-wire oracle. It
exercises files, fragmented nonblocking socketpairs, EOF, rejection precedence,
the maximum accepted body, and partial headers and bodies. It also compares the
emitted LLVM with the same authored functions in one source file under each layout.
Pass a new output directory to retain commands, inputs, and results.

```sh
cargo build --offline --release -p holo
python3 -B tests/fixtures/telemetry_worker/verify.py \
  --holo build/cargo/release/holo \
  --output build/examples/telemetry-worker-verification
```

See the [refill proposal](../../../docs/spec.md#synchronous-file-and-stream-refill)
and [package design](../../../docs/packages.md) for the boundaries of the
experimental source forms.
