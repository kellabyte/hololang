# RESP2 Holo sources

`main.holo` implements the checked parser, session loop, table dispatch,
and reply formatting. The parser reports typed failures and publishes scalar
offsets only after complete validation. The live session handles each outcome
with `on` blocks. The replay verifier uses a numeric adapter to record outcomes.
Each parse binds a readonly view of the live pending bytes. Validated offsets
then narrow that same view to the key and value, without copying them.
The parser uses `holo.bytes.find` to locate the carriage return in a decimal
header. It searches at most twenty digit positions and the following byte.
Command names are compared against byte literals with ASCII case folding.
The entry refills a writable view of the pending buffer's free tail in place.
It uses `holo.bytes` to compare exact byte views and copy values,
`holo.buffer` to compact the unread tail, and `holo.decimal` to parse lengths.
The bulk reply builder lends a writable view of its output budget to
`holo.bytes.write`. The cursor advances after each complete write and its
final position gives the reply length. The session passes a readonly view of
the finished reply to `core.send`, using the current send offset on each attempt.
It uses `holo.net` results to advance by accepted bytes, wait for readiness,
retry interruptions, and end or advance the session after a disconnect.
Fixed replies use byte literals copied into the same output slot.

`store.holo` exports the inline slot schema, its constructor, and bounded
lookup and update helpers through `redis_server.store`. The entry owns one
fixed table and lends its published slots to those helpers. Lookup returns
`Some(index)` for a matching key and `None` for a miss. Insertion scans for a
match while remembering the first free slot. The update validates both source
extents before writing a new key or value and publishes the logical length
after the write. Each slot's constructor establishes its physical capacity.

The package manifest and reproduction commands live in the
[example guide](../README.md).
