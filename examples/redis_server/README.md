# Redis GET/SET server

This Holo example implements plain `GET key` and `SET key value` using the
Redis RESP2 wire protocol. The listener serves one client at a time on IPv4
loopback and keeps the in-memory table across connections. Use `redis-cli -2`
to try it. [RESP wire format](https://redis.io/docs/latest/develop/reference/protocol-spec/)

## Scope

- Requests are RESP2 arrays of bulk strings with canonical unsigned decimal
  lengths and case-insensitive ASCII command names. Keys and values may
  contain arbitrary bytes, including empty strings. The server handles
  requests split across reads and pipelined requests on one connection.
- The table holds up to 16 keys. Keys may be up to 32 bytes and values up to
  128 bytes. `SET` can overwrite an existing key when the table is full.
- The example supports only plain `GET` and `SET`. RESP3, other commands,
  `SET` options, expiration, persistence, authentication, and TLS are outside
  its scope. The table starts empty on each process launch.

## Run it

From the repository root, build with matched LLVM and start the listener.

```sh
mkdir -p build/examples
work_dir=$(mktemp -d build/examples/redis-server.XXXXXX)
cargo run --offline --locked -p holo -- build --locked --release \
  --package examples/redis_server/holo.toml -o "$work_dir/redis-server"
"$work_dir/redis-server" listen-loopback 6380
```

In another terminal, run the client.

```sh
redis-cli -2 -p 6380 SET language Holo
redis-cli -2 -p 6380 GET language
redis-cli -2 -p 6380 GET missing
```

These commands return `OK`, `Holo`, and a nil reply, respectively.

## Implementation

The Holo entry parses one command at a time from a 512-byte pending buffer.
It handles the parser's named outcomes directly and reads again when the
current command needs more data. A complete request records its consumed
extent and the offsets of its key and value.
The input buffer can be reused after `SET` copies into a fixed slot.
Overwrites reuse the selected slot. Each send attempt uses a readonly view of
the finished reply and starts at the number of bytes already sent. The output
slot stays live until the reply is complete or the session ends.

The entry requests 1,024 bytes for input and output buffers, 16 for the byte
writer cursor, 64 for the request record, and 4,096 for the table. These are
logical backing capacities.
`holo.buffer` compacts unread bytes before a refill when necessary. Each refill
writes directly into a writable view of the pending buffer's free tail and
advances its live length by the received count. A successful `SET`
copies into a table slot and `GET` copies the live value into the reply slot.
The entry reuses those allocations across requests and connections. The
native host supplies the socket operations. Holo implements parsing, table
storage, and reply formatting. Request lengths use `holo.decimal` and bulk
replies use the bounded cursor in `holo.bytes.write`. The session loop handles
send progress, readiness, and session changes through typed `holo.net` results.
[Milestone scope](../../docs/milestone-0.1.md)

`SET` replies `+OK`. A successful `GET` sends the logical value as a bulk
string, including a zero-length bulk for an empty stored value. The lookup
returns `None` for a missing key, which the handler sends as a nil bulk string.
Simple error replies have the form `-ERR n` with
the standard CRLF terminator.

| Code | Meaning | Session behavior |
|---:|---|---|
| 1 | EOF with an incomplete request | Send the error if the peer still reads, then close |
| 2 | Malformed framing, command, or arity | Send the error and close |
| 3 | Key, value, or pending input exceeds the profile | Send the error and close |
| 4 | Decimal length exceeds `u64` | Send the error and close |
| 5 | Full table for a new key | Keep the session open |

## Verify it

Build the CLI for the parser verifier. Stop the listener before reusing its
executable in the server verifier.

```sh
cargo build --offline --locked -p holo --target-dir "$work_dir/cargo"
python3 -B examples/redis_server/verify_parser.py --holo "$work_dir/cargo/debug/holo" \
  --output "$work_dir/redis-parser-check"
python3 -B examples/redis_server/verify_server.py "$work_dir/redis-server"
```

The parser verifier compares the server's Holo parser with an independent
Python implementation. The server verifier checks exact replies over a
nonblocking Unix socket passed through `stream-fd FD`.

To include the loopback listener in verification, append `--listener` to the
server verifier command. It uses noninteractive `redis-cli -2` and checks
a single session and separate client invocations. The previously verified
client version was `redis-cli 8.2.1`. [Client source](https://raw.githubusercontent.com/redis/redis/8.2.1/src/redis-cli.c)
