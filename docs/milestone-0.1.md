# Holo 0.1 milestone

**Planned experimental preview.** The flagship application is a bounded
in-memory key-value server that a user can try with `redis-cli`. It gives the
language and standard library a shared test of binary parsing, checked storage,
typed errors, and socket I/O.

The [checked region core](implementation-plan.md#m1-acceptance-decision--november-10)
and [first typed error slice](implementation-plan.md#experimental-error-handling-milestone-complete)
are accepted under their recorded scope. The [hosted input runtime](../runtime/refill/README.md)
already reads a file or an inherited connected socket into reusable storage.
The [bounded RESP2 application](../examples/redis_server/README.md) keeps parsing,
storage, and reply formatting in Holo. Its verifiers compare request extents,
table outcomes, and reply bytes against independent oracles. The
[public preview gate](#public-preview-gate) also covers release packaging.

The macOS arm64 implementation passes offline parsing, bounded store
conformance, inherited-stream replies, and the loopback `redis-cli -2`
check. The listener keeps the Holo entry and its table alive across clients.
[Preview verification](releases/unreleased.md#verification)

## What 0.1 should demonstrate

Implement the [RESP2 wire protocol](https://redis.io/docs/latest/develop/reference/protocol-spec/)
for plain [`GET key`](https://redis.io/docs/latest/commands/get/) and
[`SET key value`](https://redis.io/docs/latest/commands/set/). A successful `SET`
creates or replaces a value and replies `+OK`. `GET` returns a bulk string or a
nil reply. An empty value is a present key with a zero-length bulk string. Treat
keys and values as arbitrary bytes and document limits on key length, value
length, and entry count. Keep the initial listener on loopback, with one client
served at a time. Defer expiration, persistence, authentication, RESP3, and the
broader Redis command set.

The first network demo runs both commands in a single `redis-cli -2` session.
The end-to-end release check runs them in separate sessions against the same
server process to verify that state survives reconnects. The following is the
target interaction once the listener is available.

```sh
redis-cli -2 -p 6380 SET language Holo
redis-cli -2 -p 6380 GET language
redis-cli -2 -p 6380 GET missing
```

Expect `OK`, `Holo`, and nil in that order. Record the tested client version and
its startup behavior. The release claim is a bounded RESP2 GET/SET server tested
with `redis-cli`.

## Work and acceptance

### 1. Parse RESP2 in Holo

**Implemented.** The same parser is compiled into the offline verifier and
the server. It publishes a copied request descriptor only on success and
returns a typed outcome. The session loop handles the named cases directly.
The replay verifier reads the same outcomes through a numeric adapter.

Start with offline request replay using the existing input path and the
[fragmented frame reader](../tests/fixtures/typed_stream_reader.holo) as a checked
storage precedent. Recognize the command and its argument extents only after
validating the length prefixes and delimiters. Test arbitrary fragmentation,
multiple requests in one read, embedded NUL and CRLF, empty payloads, oversized
lengths, arithmetic overflow, malformed commands, and truncated input. Use an
independent oracle for command identity, consumed extent, and payload bytes.

The server uses `holo.bytes` for search, comparison, and moves over checked borrowed
ranges, `holo.buffer` for pending input, and `holo.decimal` for checked decimal
parsing and formatting. RESP framing and protocol size limits stay in the
application.

### 2. Give the store its own bounded storage

**Implemented.** The table preinitializes 16 stable slots and validates key
and value extents before writing any slot bytes. A GET reply retains the
logical value by copying it into the reusable reply slot.

Start with a preallocated table of slots and checked logical lengths. Keep the
table alive across requests, with receive and reply storage scoped to their
actual uses. Validate a `SET` and check capacity before publishing its new value.
The store owns the accepted key and value bytes after the receive buffer is
reused. An overwrite reuses a slot, and a failed update preserves the previous
entry. A `GET` reply must retain valid access to its bytes until output finishes.

Verify insert, lookup, overwrite, full-table and oversized-value failures,
missing versus empty values, and repeated reuse at fixed capacity. Keep the
first implementation within the existing [M1 memory rules](spec.md#m1-memory-rules).
Any storage change follows the [Goose compatibility checklist](spec.md#goose-performance-compatibility-checklist).
The initial table leaves general maps, pools, stable handles, and collection
growth for later design.

### 3. Run the server with a real client

**Verified on macOS arm64.** The inherited stream and IPv4 loopback host
use the experimental checked send and session operations. The conformance
run covered one piped client session and separate client invocations,
including a rejected overwrite between successive reads.

Add a checked, bounded byte-write operation to the connected-socket path. Specify
the initialized extent, returned byte count, and behavior for short writes,
interruption, would-block, and disconnect. Keep a reply usable until it has been
sent or the connection ends. Document when malformed or over-limit requests
get a RESP error reply and when the server closes the connection. Test exact
reply bytes and ordering when requests arrive together. A transport failure
after a committed `SET` leaves the update committed.

Prove one real `redis-cli` session, then add listener and accept support while
keeping the store alive across successive connections. Holo owns parsing,
lookup, update, and reply formatting. The reviewed host boundary owns native
socket operations. Document its ownership and cleanup rules before selecting a
public socket API.

## Public preview gate

Run the preview check from a Git checkout or extracted candidate on macOS arm64. Install Rust 1.88 or
later, Python 3.10 or later, a matched LLVM toolchain, and `redis-cli`. For the
same LLVM and Redis release series as CI, use Homebrew.

```sh
brew install llvm@22 redis@8.2
export HOLO_LLVM_PREFIX="$(brew --prefix llvm@22)"
export PATH="$(brew --prefix redis@8.2)/bin:$PATH"
make check
```

The check copies a checkout's tracked and unignored new files into a temporary
source tree or builds directly from an extracted candidate. It uses a fresh
Cargo target directory and checks the exact Hello World
output, runs the Redis parser oracle, and exercises stream and IPv4 loopback
replies, including an overwrite, a rejected request, and a reconnect. The
loopback check requires permission to listen on an ephemeral port. When Cargo
dependencies are already cached, run `python3 -B tools/check.py --offline`.

The [preview workflow](../.github/workflows/preview.yml) runs the same command
from a clean checkout on macOS arm64 and runs the workspace tests.
`make archive` creates the source candidate and checksum. Qualify other
targets separately. [Release notes and archive instructions](releases/unreleased.md)

The [implementation plan](implementation-plan.md) retains compiler milestone
status and evidence. The [specification](spec.md) governs language decisions for
any new borrowing, storage, or effect rules this application requires.
