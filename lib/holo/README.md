# Holo core standard library

The core standard library is written in Holo and ships as the local
[`holo` package](holo.toml).

| Namespace | Purpose |
| --- | --- |
| [`holo.bytes`](bytes.holo) | Search and compare borrowed byte views, move bytes between views that may overlap, and join two low octets in little-endian order. |
| [`holo.bytes.read`](bytes/read/integer.holo) | Read checked little- and big-endian integers at the start of a byte view or at a cursor's position. |
| [`holo.bytes.write`](bytes/write/cursor.holo) | Append octets, bytes, and decimal digits to a writable view at a cursor's position. |
| [`holo.decimal`](decimal.holo) | Parse canonical ASCII `u64` digits from borrowed bytes and write them into a bounded byte view. |
| [`holo.buffer`](buffer.holo) | Maintain a live prefix in caller-owned initialized byte storage. |
| [`holo.net`](net.holo) | Typed results for socket receive, send, readiness, and session transitions. |
| [`holo.net.tcp`](net/tcp/README.md) | Typed results for closing a TCP stream. |

Declare `holo` as a local dependency in your package manifest, then import the
namespace you need with a directive such as `using holo.bytes`.
[Package guide](../../docs/packages.md)

`bytes.find(input, octet)` returns `Some(offset)` for the first matching
byte, with an offset relative to the supplied view. It returns `None` for
an empty view, a missing byte, or a needle above 255.
`bytes.equal(left, right)` returns a `bool` that can be used directly in
conditions. It compares the supplied extents, including empty views.
`bytes.equal_ignore_ascii_case(left, right)` folds ASCII letters for
comparison and compares all other octets exactly.

Import the packaged `holo.bytes.read` subnamespace with `using holo.bytes.read`.
Its `read.u16_le`, `read.u16_be`, `read.u32_le`, `read.u32_be`, `read.u64_le`,
and `read.u64_be` functions read the first two, four, or eight bytes of a
borrowed view and return `Result<u64, ReadError>`. A short view reports
`Short(required)` before any payload read. Trailing bytes remain unread.

For successive fields, initialize a [`read.Cursor`](bytes/read/cursor.holo)
with `read.start()` in caller-owned storage. Pass a writable reference to the
cursor and a readonly byte view to `read.take_u8` or the corresponding
`read.take_u16_le`, `read.take_u16_be`, `read.take_u32_le`, `read.take_u32_be`,
`read.take_u64_le`, or `read.take_u64_be` function. A complete field advances
`position` by its width. `Short(required)` reports the field width and leaves
`position` in place. The caller can retry with a longer view starting at the
same byte, or reset `position` for a new input view. The cursor stores no byte
reference and can live across refills.

Initialize a `write.Cursor` with `write.start()` and pass it with a writable
byte view to `write.put_u8`, `write.put_bytes`, or `write.put_decimal`. Each
call returns the byte count and advances `position` after the complete write.
The destination may overlap the source of `put_bytes`. `Short(required)`
reports the width of the attempted operation, and `InvalidOctet(value)`
reports a value above 255. Both preserve the destination and position. Retry
a short write with a longer view starting at the same byte. At the end of a
sequence of successful writes, `core.prefix(destination, scan.position)`
borrows the initialized output. [Writer source](bytes/write/cursor.holo)

For a stream buffer, the caller owns an initialized byte slot and keeps the
length `filled` and, if needed, a read cursor `consumed`. `buffer.space` checks
the live length and reports the room left. `buffer.append` copies a new chunk
and returns the updated length. `buffer.compact` moves the unread tail to offset
zero and returns its length, after which the caller resets its read cursor.
For hosted input, `refill` can fill a named writable view of the unused tail
directly. Add `core.len(chunk)` to `filled` in the data branch. The caller can
lend exactly the live bytes with `core.prefix(slot.wire, filled)`.
The helpers retain no references and never resize the slot. Invalid cursors or
insufficient destination space take the ordinary bounds trap before any write.

Check all namespaces from the repository root.

```sh
cargo run --offline -p holo -- check --strict-names --package lib/holo/holo.toml
```
