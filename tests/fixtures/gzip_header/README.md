# Gzip header inspector

Read the ten-byte fixed prefix of a gzip member and report its timestamp and
flags. The inspector checks the signature, the DEFLATE method, and the reserved
flag bits. It preserves optional-field indicators, compression hints, and OS
identifiers as raw values. A complete fixed prefix says nothing about the
validity of later optional fields, compressed data, or the trailer.
[RFC 1952, member format](https://www.rfc-editor.org/rfc/rfc1952.html#section-2.3.1)

## Run it

Run these commands from the repository root with matched LLVM tools installed.

```sh
mkdir -p build/examples
python3 -c 'import gzip; from pathlib import Path; Path("build/examples/hello.gz").write_bytes(gzip.compress(b"hello", compresslevel=9, mtime=42))'
cargo run --offline -p holo -- build --locked --release \
  --package tests/fixtures/gzip_header/holo.toml -o build/examples/gzip-header
build/examples/gzip-header file build/examples/hello.gz
```

Successful output is five unsigned decimal lines. The first is zero, followed
by the raw modification time in seconds since the Unix epoch, the header flags,
the extra compression flags, and the OS identifier. The sample prints
`0`, `42`, `0`, `2`, and `255`. A zero modification time means the timestamp
is unavailable. The inspector performs no conversion to a calendar or zone.

To inspect a stream, launch the executable with `stream-fd FD` and pass an
already connected nonblocking stream descriptor with receive timeout disabled.
The [hosted runtime guide](../../../runtime/refill/README.md) describes descriptor
admission and ownership. The program stops at the fixed prefix even when the
peer stays open. An idle peer can keep it waiting until the prefix completes.

## Failure output

The parser returns the earliest error in field order. A bad byte already
received takes precedence over an incomplete suffix. Failures print one
unsigned status with no metadata.

| Status | Meaning |
| ---: | --- |
| 1 | EOF before the ten-byte prefix completed. |
| 2 | Invalid gzip signature. |
| 3 | Compression method other than DEFLATE. |
| 4 | A reserved flag bit is set. |
| Bit 63 set | Native read or readiness error. The low bits carry errno. |

The example's native process returns zero after printing an outcome. The host
reports descriptor admission errors separately on stderr and returns nonzero.
Failure to acquire either bounded setup region traps before input is consumed.

## Storage and verification

[main.holo](main.holo) owns the decoder and driver. Each native read is limited
to the remaining prefix and writes into the same ten-byte slot. The decoder
borrows the received bytes and retains scalar fields plus the four timestamp
octets across calls. Once the prefix is complete, it reads the timestamp with
[`holo.bytes.read`](../../../lib/holo/bytes/read/integer.holo). Decoding uses
checked `noalloc` and `noblock` contracts. The driver handles interruption and
readiness outside the decoder. It reads only the fixed header.

[verify.py](verify.py) compares native files and streams with a Python oracle.
It also compares the package's LLVM with a single-file version of the entry
and its imported `holo.bytes.read` namespace. Pass a new output directory to
retain the cases and results.

```sh
cargo build --offline --release -p holo
python3 -B tests/fixtures/gzip_header/verify.py \
  --holo build/cargo/release/holo \
  --output build/examples/gzip-header-verification
```
