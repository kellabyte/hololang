# Publish only what’s ready

A frame appears in the batch when its last byte is ready. If the input buffer
holds half a frame, the caller learns how many bytes are needed and the batch
keeps exactly the frames it held before the call.

```holo
return core.append(frames, decode_frame(input))
    on ok                    then Accepted
    on Rejected Truncated(n) then ReadMore(n)
    on Rejected BadMagic     then RejectMalformed
    on Full                  then Backpressure
    on SizeOverflow          then RejectOversized
```

The decoder builds in the new frame’s final slot. The append publishes a complete
frame or discards its private work before an `on` arm runs. A reference to the
last accepted frame stays usable. When the rest of the input arrives, the caller
can try the append again.

**Status.** The experimental checked slice is complete. It covers scalar
results, named error unions, typed append, destination forwarding, and
scoped construction of fixed and byte-bearing records. A `try` can
propagate a declared child error with its copied `u64` payload. The opening
example uses the proposed outcome enum. Runnable append examples handle
the same outcomes with scalars or booleans.

For an ordinary lookup that can miss, `Option<u64>` represents presence with
`Some(value)` and absence with `None`. Redis uses both cases when choosing a
GET reply. [Optional scalar returns](spec.md#optional-scalar-returns)

## Compose a result from several services

A dashboard loads the active account, queries a customer count, and saves it.
Each service owns its own error family. The dashboard names the ones it can
return.

```holo
namespace app.dashboard

using app.session
using app.sql
using app.cache

error DashboardError {
    from session.SessionError as Session
    from sql.SqlError as Sql
    from cache.CacheError as Cache
}

func load_customer_count
    arg scenario u64 as readonly copy
    returns Result<u64, DashboardError>
{
    let account = try session.account_id(scenario)
    let query = sql.count_customers(account)
    let count = try query
    let saved = try cache.save_count(count)
    return ok saved
}
```

If the query returns `SqlError.Timeout(500)`, the second `try` returns
`DashboardError.Sql.Timeout(500)` and the cache call is skipped. The caller
can save the outcome and handle it later. Here, `on error` handles any
remaining failure, including a lost database connection.

```holo
namespace app.dashboard

using app.ui

public func show_customer_count
    arg scenario u64 as readonly copy
    returns u64
{
    let outcome = load_customer_count(scenario)
    return outcome
        on ok(count) then ui.show_count(count)
        on Session.PermissionDenied then ui.sign_in()
        on Sql.Timeout(delay) then ui.try_again(delay)
        on Cache.Unavailable then ui.cache_unavailable()
        on error then ui.query_failed()
}
```

```sh
cargo run --offline -p holo -- run --locked --release --package tests/fixtures/error_unions/holo.toml
```

The runnable demo prints `42`, `1`, `500`, `3`, and `4` on separate
lines. The scenario argument drives the deterministic services through success,
sign-in, retry, cache recovery, and fallback. [Dashboard package](../tests/fixtures/error_unions/README.md)

A saved `Result<u64, E>` is immutable. The call runs at the `let`
declaration, and the local preserves its original success value or named
error and payload. A caller can do other work before handling the result or
handle it more than once. `let count = try query` binds the saved success
or propagates its failure using the same family rules as `try call(...)`.
Both expression and statement handlers use the
same exhaustiveness and payload scope rules as immediate handlers.
When the enclosing helper returns the same `Result<u64, E>` type,
`return outcome` forwards the saved value and preserves its original
case and payload.

### Error union rules

- A family can mix local cases with `from ChildFamily as Alias` entries. Each
  alias must be distinct from the family's other aliases and local cases. A
  nominal family may appear only once as a direct child. Cycles are rejected.
- Each child retains its case names, payloads, and nested aliases. If the
  parent includes `from RequestError as Request` and that family includes
  `from SqlError as Sql`, the outer handler can select `on Request.Sql.Timeout(delay)`.
  Coverage is checked for every leaf. A handler uses the expected family's
  full path when an explicit package type prefix shares the same spelling.
- `try` lifts a failure into the caller's family when the callee uses that
  same family or a declared direct child. Each intermediate layer can propagate
  its own declared child, preserving the complete path. Both `return call(...)`
  and `return outcome` require the exact result type.
- `return error DashboardError.Sql.Timeout(500)` constructs the selected leaf
  explicitly. Each concrete case has at most one named `u64` payload.
  There can be at most 255 leaves in an expanded family in this implementation.
- `from qualifier.ChildFamily as Alias` resolves the child with the declaring
  file's `using` directives. A `public error` may include only public children.

## Handle typed scalar results

Consider a telemetry header validator that takes three copied fields from a
complete header. It reports the first validation failure or returns the admitted
length. Its public signature uses `Result<u64, FrameError>`, and its caller
uses the same handling syntax as the opening example.

```holo
let code = frame.validate_header(magic, kind, length)
    on ok(admitted) then admitted
    on Oversized then 3
    on BadMagic then 1
    on UnknownKind then 2

let accepted = frame.validate_header(magic, kind, length)
    on ok then true
    on error then false
```

These are two independent calls. The first maps each error to a scalar and
reads the validated length. The second observes only whether admission
succeeded. `on ok` ignores the scalar payload. A handled
boolean can be bound with `let` and used as a condition. The first slice
permits literal boolean arms for that form.

A validator can also check the received extent before subtracting the header
size. Here `require_prefix` returns `Truncated(required)` when the buffer is
short. `payload_available` propagates that error through `try` and computes
the payload length only when the extent is valid.

```holo
public func payload_available
    with noalloc, noblock
    arg available u64 as readonly copy
    arg header_length u64 as readonly copy
    returns Result<u64, ExtentError>
{
    let validated = try require_prefix(available, header_length)
    return ok core.sub_wrapping(validated, header_length)
}

let required = extent.payload_available(3, 4)
    on ok then 0
    on Truncated(total) then total
```

Here the caller gets `4` from the error arm. If it supplies six bytes instead,
the success payload is `2`. The [scalar result tests](../tests/compiler/frontend/frontend_evaluation.rs)
check arm selection and payload propagation through `try`.

For an append, the success arm has no payload. The complete record is already
in the batch when the arm runs. Use `on error` to cover every failure left by
earlier named arms.

```holo
let accepted = core.append(rows, make_parent(7, 3))
    on ok    then true
    on error then false
```

```sh
cargo run --offline -p holo -- run --release tests/fixtures/typed_append.holo
```

The fixture prints `18446744073709551615`, `8`, and `1` on separate lines.
It reads the rejected value after rollback, publishes sequence `8` on retry,
and recognizes `Full` when the destination has no room for a second record.
[Complete fixed-record example](../tests/fixtures/typed_append.holo)

## Decode a frame into its final slot

The example frame starts with three bytes containing magic `0x48`, a sequence
number, and the payload length. `Truncated` carries the total number of input
bytes needed, so the stream reader can refill without interpreting an error
string.

```holo
error FrameError {
    Truncated(required u64)
    BadMagic
}

struct Frame {
    sequence u64
    payload Bytes
}

func decode_frame
    with noblock
    arg input Bytes as readonly reference
    returns Result<Frame, FrameError>
{
    // A short header needs a refill before any field is read.
    if core.len(input) < 3 {
        return error FrameError.Truncated(3)
    }
    // A complete header with the wrong magic is malformed.
    if input[0] != 0x48 {
        return error FrameError.BadMagic
    }
    // The header length and one byte always fit in u64.
    if core.len(input) < core.add_wrapping(3, input[2]) {
        return error FrameError.Truncated(core.add_wrapping(3, input[2]))
    }
    return ok Frame:
        sequence = input[1]
        payload  = core.bytes(input[2], input[core.add_wrapping(3, core.index())])
}
```

The guards establish that the header and complete payload exist before field
initialization. `core.bytes` reads the payload range beginning at byte `3`
and initializes owned bytes in the final slot. The compiler recognizes this
checked range initializer and uses the existing byte-copy lowering. An
eligible append avoids an intermediate frame and a second frame copy. The
proposed `copy input[3..end]` form and ordinary scalar `try` in a producer
prefix remain separate source proposals.

For a batch ending at sequence 7 with room for another frame, input
`48 08 03 AA` returns `ReadMore(6)`. The batch still holds sequence 7 at the
same address. With `BB CC` appended to the input, a fresh attempt publishes
sequence 8 with payload `AA BB CC`. The earlier reference remains valid through
both attempts. This walkthrough assumes the reservations preceding validation
succeed.

```sh
cargo run --offline -p holo -- run --release tests/fixtures/typed_frame.holo
```

The runnable caller builds a `Received` record with a channel number followed
by `frame = try decode_frame(input)`. The channel is already private when the
child returns `Truncated(6)`. The caller reads `6` after rollback, retries with
the complete input, and prints `8` and `204` for the published sequence and
last payload byte. It also mutates the source buffer after success to verify
that the published frame owns its bytes. [Complete caller and checks](../tests/fixtures/typed_frame.holo)

## Decode a received prefix

The hosted `refill` operation lends exactly the positive read's initialized
prefix. The same decoder can consume that view as a readonly producer input.

```holo
refill slot.wire, 6 as input {
    let needed = core.append(frames, receive(1, input))
        on ok                          then 0
        on Rejected Truncated(required) then required
        on error                       then 1
    core.print(needed)
} else status {
    core.print(status)
}
```

The input name is usable only in the data continuation. The producer receives
the native read's exact pointer and count, with the original checked slot root.
Nested producers forward that readonly extent. A short read keeps its actual
length even if the rest of the slot contains bytes from an earlier attempt.
Once the continuation ends, a later refill can reuse the slot. A successful
frame owns its copied payload and can outlive the view. A returned error can
carry the required extent as a copied scalar. EOF and other native statuses
stay in the `else` continuation, where no received input is bound.

The [hosted frame example](../tests/fixtures/typed_refill_frame.holo) reads a complete
frame, then a four-byte prefix that needs six bytes, then EOF. It verifies
rollback and the earlier frame's owned payload after the slot is reused.
The [incremental reader](../tests/fixtures/typed_stream_reader.holo) uses
[`holo.buffer`](../lib/holo/buffer.holo) to assemble a frame across reads in a
bounded initialized slot. It passes `core.prefix(pending.wire, filled)` to
the producer, so the nested decoder can use `core.len(input)` as its only
input bound. It uses `Truncated(required)` to choose the next read limit,
retries the append, and publishes the frame once
its entire extent is present. EOF with an incomplete frame reports the last
required extent. The native acceptance test covers files and fragmented
nonblocking sockets across all layouts at O0, O3, and under ASan.

## Construct a record or return an existing value

`return ok Frame:` introduces a fresh `Frame` initializer. The colon opens
the field list. Each `field = expression` begins on its own line at a common
indent deeper than the opening line. Dedenting ends the initializer. This
slice initializes fields in declaration order and requires each field exactly
once. The checker resolves `Frame` to the declared success type and plans
construction at the receiving destination when the call is eligible.

The owned-local design uses the same initializer for a local value.
`return ok frame` uses that existing value as the success payload.

```holo
let frame = Frame:
    sequence = input[1]
    payload  = copy input[3..end]

return ok frame
```

`ok value` follows the value’s ordinary return rules. A copied scalar such as
`ok length` is in the first slice. The move and lifetime rule for an owned
local remains open. Returning an existing owned frame must establish that
its storage survives the return or that an explicitly permitted transfer
applies. The first checked append slice admits fresh construction at its
final destination and rejects prebuilt owned results.

### Borrow a fresh result within its success arm

In entry code, `core.construct` gives a typed producer the tail of a bounded
region. The region must be free of a live batch or another scoped record.
Each `on` clause runs a block. Only `on ok(ticket)` introduces a readonly
record reference. Use `on ok` when the block needs no record binding.

```holo
core.construct(memory, open_ticket(id))
    on ok(ticket) then { core.print(ticket.id) }
    on Rejected InvalidId(value) then { core.print(value) }
    on Full then { core.print(0) }
    on SizeOverflow then { core.print(0) }
```

A final `on error` may cover any remaining failures. Rejection restores the
marker before its block runs, so that block can retry in the same region.
Success releases the slot when its block ends, including on an early return.
Copy a scalar out of the block when it needs to survive. The reference and
its byte views share the block lifetime.
[Native scoped-result tests](../tests/compiler/frontend/frontend_native.rs) · [Storage and lifetime rules](spec.md#fresh-aggregate-locals)

Inside a hosted `refill` data block, a scoped constructor can borrow the
freshly received bytes through a readonly `Bytes` parameter. The input
extent is the count returned by that read. A handler for truncation can
retry with the same live prefix once the region tail has been restored.
Any bytes the producer copied into the result belong to the scoped
record and survive writes to the input slot while that record is live.
[Hosted scoped-result conformance](../tests/runtime/hosted_refill.rs)

### Handle backing acquisition and construction separately

The experimental `with core.region(bytes) { … } else { … }` form keeps one
backing live for the success block. Its `else` handles a failed acquisition
before any constructor arguments run. Inside the block,
`core.construct(open_ticket(id))` selects that backing. Its `on` clauses
handle construction after acquisition, including `Full` when a record
cannot fit and `Rejected InvalidId(value)` when the producer rejects the id.
A final `on error` covers unlisted construction failures. A loop inside the
block can revisit the same constructor and reuse the released tail.
[Source rules](spec.md#supplying-a-stack-budget-in-source) · [Native placement tests](../tests/compiler/frontend/frontend_native.rs)

### Forward a whole destination result

A constructor can forward another constructor's exact result through the
destination it received. The wrapper consists of one direct return call.
The record and error family must both match by declaration identity.

```holo
func decode_network_frame
    with noblock
    arg input Bytes as readonly reference
    returns Result<Frame, FrameError>
{
    return decode_frame(input)
}
```

`core.append(frames, decode_network_frame(input))` still owns publication
and rollback. The wrapper evaluates its call arguments once in order and
passes on the destination and checked storage operands. The inner call
initializes the record, including its header where the layout requires
one, and returns its original success or error. Forwarding introduces no
second record, payload copy, or marker. Plain record constructors can
forward the same way when both signatures return the exact record.

## Ordinary typed failure

`Result<T, E>` has one active alternative, a success value of type `T` or a
case of `E`. An error family has declaration identity even when another family
has the same case names. `return error E.Case` produces the named failure.
`try` evaluates the result once, yields its success value, or returns its
error through the enclosing function’s normal exit. The scalar slice accepts
`let value = try direct_call(...)` inside an ordinary `Result<u64, E>` function
when the callee returns the same error family or one of its declared direct
children. Use explicit handling and mapping for an unrelated family.
`return decode_frame(input)` forwards the entire result when its type agrees
with the signature. `return ok frame` supplies the success payload of a result.

```holo
return checked_payload_length(input)
    on ok(length)    then ExpectFrameBytes(3 + length)
    on Truncated(n)  then ReadMore(n)
    on BadMagic      then RejectMalformed
```

Ordinary failure runs the lexical cleanup of scopes it exits. Writes already
made to caller-visible state remain visible. The
[scalar result rules](packages.md#typed-error-families) define `on` arms and
exact forwarding for `Result<u64, E>`. Cases may carry one named `u64` field.
`return error Family.Case(value)` initializes that field. `on Case(binding)`
binds it in the selected arm, and `on Case(_)` discards it. Stored results,
general aggregate moves, and arbitrary payload patterns require their own rules.

## Handle the result with `on`

- An `on` list on a `return` operand or `let` initializer forms an expression.
  Its arms begin on following lines at the same indentation, deeper than the
  subject line. Dedenting ends the list. Function and executable body braces
  follow the [existing rule](spec.md#2-decisions-carried-forward).
- Evaluate the subject once and exactly one `then` expression. Its value is
  the value of the whole expression. All arms must produce the expected type,
  or agree on a type when the surrounding context supplies none. The expected
  return type resolves unqualified result values such as `Accepted`.
- `on ok` selects success without binding a value. It handles unit success
  and can ignore a copied scalar payload. `on ok(value)` binds the active
  scalar in that arm, and `on ok(_)` also ignores it.
  An error arm names a case of the known family.
  `on Rejected Truncated(n)` first selects `Rejected`, then selects the
  `Truncated` case of its `FrameError` payload. `n` is in scope only in that
  arm. Qualification is available as `on Rejected FrameError.Truncated(n)`.
- Every list requires a success arm. Name each error case explicitly or cover
  the remaining cases with an error fallback. The checker rejects missing,
  duplicate, and wrong-family patterns and points to the relevant arm or case.
  The same rules apply to ordinary calls and to a checked append.

### Run a block for an ordinary result

Use a direct call as a statement and give every arm a block when each outcome
needs its own actions. The call runs once, and only the selected block runs.
Payload bindings are immutable and local to that arm. The block can update
outer variables, run scoped operations, or return from the enclosing function.
If the arm falls through, execution continues after the handler. A block
handler requires the same outcome coverage as an expression handler.

```holo
parse_request(request, core.prefix(wire.data, filled), cursor)
    on ok(consumed) then {
        cursor = core.add_wrapping(cursor, consumed)
        read_needed = 0
    }
    on NeedMore(required) then {
        core.assert(required > filled)
    }
    on Malformed then {
        read_needed = 0
        reply_error = 2
    }
    on OverLimit then {
        read_needed = 0
        reply_error = 3
    }
    on Overflow then {
        read_needed = 0
        reply_error = 4
    }
```

The [RESP2 session loop](../examples/redis_server/src/main.holo) uses this form
to distinguish a partial request from a malformed one at the point of recovery.

### Check success with an error fallback

```holo
let accepted = core.append(frames, decode_frame(input))
    on ok    then true
    on error then false
```

`on error` handles every returned failure left uncovered by the named arms and
discards its reason. The design also allows `on error(reason)` to bind the
original error. The experimental slice exposes payloads through named cases.
An `on` list can have one fallback, as its last arm. Named cases handle their
own outcomes and the fallback covers the rest. For this append, success
produces `true` after publication and a returned failure produces `false`
after rollback. Fatal checks still trap.

## Publication semantics

The typed form of `core.append` takes a named live batch and a direct producer
call. It returns `Result<Unit, AppendError<E>>`, where `E` is the producer’s
declared error family. The opening receiver maps that result to `Receive`.

```holo
error AppendError<E> {
    Rejected(reason E)
    Full
    SizeOverflow
}

enum Receive {
    Accepted
    ReadMore(required u64)
    RejectMalformed
    Backpressure
    RejectOversized
}
```

`AppendError<E>` describes the compiler operation’s result family. The producer’s
success type must match the batch’s record identity and admit construction at
its planned destination. The checker rejects standalone result values and
prebuilt records as producer operands in this slice.

1. **Prepare.** Resolve the named batch before evaluating the producer’s
   arguments. Capture each argument once in written order. The owner keeps
   a checkpoint for its private append and supplies the final destination to
   the producer. Argument capture and checks preserve existing append order.
2. **Construct privately.** Nested producer calls receive the appropriate
   destination or field location. Each readable field and byte must be
   initialized on every successful path. Only the producer graph may touch
   the private suffix, and references to it cannot escape construction.
3. **Publish on success.** Advance the published end and count once, after
   initialization. Return unit success to the caller.
4. **Restore on recoverable failure.** Leave the published count and end
   unchanged and restore the owner’s marker. The failed suffix becomes
   inaccessible. Return the typed failure after restoration. If the caller
   uses `on`, the selected arm runs after the append has returned its result.

| Recovery case | Meaning |
|---|---|
| `Rejected Case` or `Rejected Case(payload)` | The producer or a nested call reported a declared domain error. The selected arm can use the active case’s payload. |
| `Full` | A reservation exceeds the available space in the batch’s bounded backing. |
| `SizeOverflow` | Checked size arithmetic or the selected layout’s length representation cannot represent the requested construction. |

A nested producer failure propagates to the owning append. Recovery can start
a fresh append. It cannot resume at the failed field.

**Failure order is execution order.** Preserve the existing checked append’s
header or unit reservation before producer rejection prefixes. A scalar
initializer runs before its field reservation. A byte field evaluates its
length, checks size and width, and reserves space before evaluating byte
initializers in index order. Failed reservation and zero-length fields skip
byte initializers. Explicit range copying uses the same final-slot reservation
and reads only the validated range. An earlier reservation can report `Full`
before a later validation would report `BadMagic`. Fatal bounds checks and
assertions still trap under the M1 policy. [Current order](spec.md#borrow-sequencing-and-failure-rules) · [Fatal check policy](spec.md#fatal-checks-and-the-m1-native-boundary)

## What the checker must establish

- **An eligible producer graph.** The first slice uses direct, statically
  resolved, acyclic calls. Producers can read initialized inputs and write
  their own unpublished output. Reject writes through aliases to published
  storage, reentrant append, external effects, and unknown callees throughout
  the graph, including in unused definitions. An ordinary function may have
  effects that disqualify it as a producer.
- **A valid input view.** The checked root, projection, and extent stay live
  through the call. An input can come from the same batch’s initialized
  prefix when the checker establishes that the private suffix cannot overwrite
  it. `readonly` limits an access path. LLVM alias promises require a separate
  proof. [M1 access rules](spec.md#m1-memory-rules)
- **A usable error after rollback.** A named case may carry a copied `u64`,
  such as the required wire length. Its value survives the checkpoint restore
  independently of the discarded suffix. A future borrowed payload design
  must prove that the referent survives every intervening exit and reset.
- **Effects on every reachable path.** Appending obtains fresh region storage
  and counts as allocation under strict `noalloc`, even when backing is already
  reserved. Typed error transport needs no backing allocation for the proposed
  tag and scalar cases. `noblock` still covers producers, generated helpers,
  lexical cleanup, and recovery arms. Formatting, logging, or retry policy
  belongs to explicit caller code. [Effect contracts](spec.md#noalloc)

## Compiler implementation

### Scalar results in the checked compiler

The parser preserves spans and clause indentation. The package binder resolves
each error family to its declaration identity. Source lowering expands child
leaves into contiguous ranges in declaration order and retains the direct
child edges. The source checker enforces arm coverage, branch result type,
payload scope, and exact forwarding. The checked IR independently verifies
the graph, child ranges, payload shapes, calls, and dispatch before LLVM
emission. Named case identities stay in the IR until lowering. The scalar
slice keeps the entire `u64` range for success and error payloads.

The general fallible call path returns a private LLVM `{ i64, i8 }` pair. Tag zero
selects success. Nonzero tags are declaration ordinals. The same `i64` lane
carries the active success or error payload. A tag-only failure initializes the
inactive lane to zero. `try` tests the tag and binds the success value in SSA.
On failure, same-family propagation returns the original pair. A direct child
lift adds the child's constant tag offset on the failure edge and preserves
the payload lane. This keeps the same return layout and introduces no
allocation or unwinding. `on` switches on the tag and extracts the scalar only on an edge
that binds the corresponding payload. The selected arm yields an SSA merge
value. The `on` expression itself needs no materialized result slot or
diagnostic allocation.
Target calling conventions and optimization determine register assignment and
spills. [LLVM lowering](../compiler/src/backend/llvm/results.rs)

```text
result = call validate_header(magic, kind, length)
switch result.tag
    0          -> read result.value if the success arm uses it
    named case -> read result.value if the case binds a scalar, then evaluate its arm
    remaining  -> evaluate the error fallback
merge the selected value
```

When an immediate `on` handles a small checked return chain over copied
scalars, the compiler can instead emit that chain at the call site. It
captures all actuals once in written order, evaluates each guard in its
original order, and routes each known return to its matching arm. Only the
chosen arm runs. The emitter merges payloads when multiple return sites
reach the same arm. The bounded shape excludes memory access, local mutation,
and calls inside the inlined chain. `try` and general handled calls keep the
ordinary result ABI. [Native scalar dispatch tests](../tests/compiler/frontend/frontend_evaluation.rs)

### Typed append in the checked compiler

The experimental source accepts `Result<Record, E>` producers for checked
destination record schemas. Standalone source and manifest packages support
fixed and byte-bearing records. Package constructors use namespace-local
record and error declarations. Public signatures require both declarations
to be public. Public records expose only public inline child types.
[Package examples and tests](packages.md#byte-bearing-typed-append-across-packages)

`return ok Record:` names a
fresh destination and lists fields in declaration order. Prefix guards can
return `error E.Case(value)`. A nested field uses `try` to call a typed producer
with the same family or a declared direct child. The checker verifies the nominal
family, complete initialization, argument effects and roots, arm coverage, and
arm-local payload scope. The checked IR verifies these properties independently.
The source slice supports scalar and literal boolean arm results. It binds a
handled append with `let`. In the native entry, an immediate handled `return`
follows the existing restriction to exit code zero. Readonly byte inputs use
published, initialized projections or the direct view name from a live refill
data branch. Moving existing record values into an `ok` result remains
outside this slice.

The [LLVM publication path](../compiler/src/backend/llvm/publication.rs)
snapshots the region top, passes the receiving address, and branches on the
producer’s status. Child lifting adjusts only named rejections. Success,
`Full`, and `SizeOverflow` keep their reserved statuses. A selected
`on Rejected Child.Case(value)` arm receives the original scalar after
rollback. Numeric append uses the same publication path. A producer
that advances the shared marker follows this sequence.

```text
checkpoint = region.top
destination = region.tail
outcome = call_producer(region, destination, captured_arguments)

success:
    publish region.top as the batch end and increment count
    produce ok Unit

recoverable failure:
    restore region.top to checkpoint
    produce the typed failure
```

Typed producers return a private LLVM `{ i64, i64 }` pair. Status `0` is success,
`1` is `Full`, `2` is `SizeOverflow`, and declared rejections start at `4` in
declaration order. A cursor-based variable producer returns its checked end
in the second lane on success. On rejection the lane carries the full `u64`
payload, or zero for a payload-free outcome. Nested propagation preserves the
payload while a union lift can translate its tag. Only the owning append
restores its checkpoint when a producer could change the marker.
The handler reads a copied payload only on the matching arm after publication
or rollback. The pair stays outside the private suffix and requires no
preallocated success or error buffer. LLVM
and the target calling convention choose registers or spills. The encoding
is private to this lowering.

For headerless fixed records, the producer receives the checkpoint cursor,
capacity, and final write address as private operands. A nested producer gets
the cursor and address at its own static field offset. The address carries
no permission to write until that field's checked addition and capacity
comparison pass. Each successful reserve advances the cursor in SSA. A
successful child accounts for its exact layout extent, including the one
initialized byte of an empty record. Only the first caller outside the fixed
subtree commits that advance, once on success. A failed fixed append leaves
the original marker, count, and published end untouched. The child's guard
still runs at its original point, after the preceding field reserves and before
the child's own first reserve. When the last child completes the record, the
parent can return the child's pair directly if the child emits the parent's
tag range. A fixed child can use a bounded compiler specialization that
changes its named rejection tags at their point of construction. Success,
`Full`, and `SizeOverflow` use their existing encodings. A continuing call
or a call without a selected specialization lifts the result on its failure
edge.
[Native reservation and union lift tests](../tests/compiler/frontend/frontend_native.rs)

For byte-bearing typed records in Derived64 and Compact32, the producer
advances a private cursor through the variable-size suffix and returns its
end. Append commits that end on success. A rejection leaves the region
marker at the saved checkpoint, even after earlier fields were written.
A caller resuming after a variable child uses its checked end before
reserving the next field. A retained record header is written only after
the last field succeeds.
[Native byte-bearing reservation tests](../tests/compiler/storage/reservation.rs)

Rollback requires fixed marker bookkeeping for the selected owner. The
compiler can eliminate redundant operations when it preserves the same state.
Total failure cost includes work already performed, propagation through nested
calls, lexical cleanup, and the handler. Reset makes the suffix reusable within
its backing. Backing release, decommit, and resident memory are separate costs.
The lowering uses ordinary calls and branches, with no required error
allocation, unwinding, logging, or formatting.

#### Typed append acceptance fixture

Use a fixed `Parent` with an inline `Child`. Seed one published parent and
retain a reference to it. The second append initializes the private sequence
word before the child checks its input. On rejection, the caller must still
see the original marker, count, address, and contents. A successful retry
then uses the restored suffix.

```holo
error BuildError {
    BadChild(value u64)
}

struct Child { value u64 }
struct Parent { sequence u64, child Child }

func make_child
    arg value u64 as readonly copy
    returns Result<Child, BuildError>
{
    // Fail before the child initializes its own field.
    if value >= 100 {
        return error BuildError.BadChild(value)
    }
    return ok Child:
        value = value
}

func make_parent
    arg sequence u64 as readonly copy
    arg value u64 as readonly copy
    returns Result<Parent, BuildError>
{
    // The first field precedes the call constructing the nested destination.
    return ok Parent:
        sequence = sequence
        child    = try make_child(value)
}

let reason = core.append(rows, make_parent(8, 18446744073709551615))
    on ok                      then 0
    on Rejected BadChild(value) then value
    on Full                    then 1
    on SizeOverflow            then 2
```

The differential test checks the full-width payload after rollback across all
three layouts. A region sized for the seed alone returns `Full` before the child
runs. A Derived64 region with one extra word reaches the child. Invalid input
returns `BadChild` before the child's field reservation, while valid input
reaches that reservation and returns `Full`. The same marker, count, and seed
checks hold after both attempts. [Native and model test](../tests/compiler/frontend/frontend_native.rs)

An earlier matched benchmark timed successful construction and nested
rejection in separate native batches. It used the same numeric producer
shape, prepared inputs, and prewarmed backing. The failed interval covered
complete attempts through dispatch and digest.
An isolated marker-store cost would require a separate measurement.

The initial M3 Max comparison measured 3.44% more paired elapsed time for
typed success than numeric append and 3.53% more in the repeat. Its
generated code showed extra status and payload selection after inlining.
The later fixed-reservation lowering closed the fixed-record gate against
C and Rust in the inline and outlined configurations on that target.

The staged frame comparison measured the byte-bearing decoder against C,
Rust, and Goose with matched fragments and packed owned output. The
generated code contained a per-byte checked staging loop. The comparison
covered the whole reader, including copying and published payload consumption.

Direct phase clocks on the 192-byte reader showed separate costs in admission
and owned-payload consumption. A second diagnostic fed pre-staged prefixes
to the same decode and publication path. The admission gap against C and
Goose persisted there. The owned scan also cost more than both controls.

## Implementation stages and acceptance

The supported slice covers nominal error families with copied scalar
payloads, scalar `Result` calls, explicit `try` and exhaustive `on`, and
checked fixed-record and byte-bearing append. Whole-result forwarding
passes the final destination through a direct call with the same nominal
result type. `core.construct` gives one bounded region slot to the
success arm and restores the marker on exit. The incremental frame reader
and locked hosted scoped result are the end-to-end acceptance examples.
Independently stored aggregate results, foreign type names in package
signatures, and borrowed error payloads have separate ownership and
interface requirements.

The locked staged reader is the regression gate for the supported slice.
Its native test checks truncation, capacity rollback, retry, EOF, and
previously published payloads across actual wide and two-byte refills.
An earlier packaged performance replay measured split admission and the
complete reader against C, Rust, and Goose. The fixed-record append gate
passed against C and Rust across the three measured call configurations
on macOS arm64. The locked hosted scoped test verifies that the producer
sees the exact received prefix, restores the tail before the selected
handler, and can retry at the same address. Native Linux throughput and
PMU collection remain reader performance work. [Hosted package tests](../tests/compiler/packages/packages.rs)

```sh
cargo test --offline -p holo --test packages package_outlined_staging_preserves_hosted_rollback_and_reuse
cargo test --offline -p holo --test packages package_hosted_scoped_construct_retries_the_fresh_prefix
```

| Stage | Acceptance evidence |
|---|---|
| **1. Ordinary typed results. Implemented experimentally.** | Source and checked IR cover package visibility, exact forwarding, case identity, exhaustiveness, and result types. Native tests cover full-width success and error payloads, named cases before fallback, and exactly-once evaluation. [Scalar result tests](../tests/compiler/frontend/frontend_evaluation.rs) |
| **Named error unions. Implemented experimentally.** | Package and native tests cover named child handlers, short-circuiting, nested lifts with full-width payloads, and rollback across fixed and byte-bearing layouts. The checked IR verifies every child tag range and rejects cycles and ambiguous lifts. [Runnable dashboard](../tests/fixtures/error_unions/README.md) · [Native cases](../tests/compiler/frontend/frontend_native.rs) |
| **2. Fixed-record typed append. Implemented experimentally.** | Native O0/O3/ASan and independent model cover copied payloads, nested rejection, failure ordering, rollback, retry, and a retained reference. The checked IR rejects incompatible producers, payload shape and scope errors, and incomplete handlers. [Runnable fixture](../tests/fixtures/typed_append.holo) |
| **3. Byte-bearing typed append. Implemented experimentally.** | Model and native tests cover nested truncation and malformed input, empty and complete ranges, `Full`, typed size overflow, and an independent bounds trap. The runnable fixture checks a retained reference and ownership of the copied payload. [Runnable frame](../tests/fixtures/typed_frame.holo) · [Native cases](../tests/compiler/storage/byte_input_native.rs) |
| **4. Package typed append. Implemented experimentally.** | Locked package tests check nominal types and visibility, byte-identical LLVM against flat source, and native O0, O3, and ASan across layouts. The nested byte-bearing decoder verifies rollback after a parent write, exact-slot retry, and published payload ownership after input mutation. [Package rules and example](packages.md#byte-bearing-typed-append-across-packages) · [Conformance tests](../tests/compiler/packages/packages.rs) |
| **5. Forwarding and scoped results. Implemented experimentally.** | Exact result forwarding preserves the receiving slot through nested calls. Scoped fixed and byte-bearing construction verifies early exit, exact fit, exhaustion, nested rollback, and lexical borrow restrictions. The locked hosted case runs against files and nonblocking sockets at O0, O3, and under ASan in all three layouts. [Native forwarding and scope tests](../tests/compiler/frontend/frontend_native.rs) · [Hosted package tests](../tests/compiler/packages/packages.rs) |

The hosted package witness adds real file and nonblocking stream reads through
the same checked decoder. A short read leaves stale bytes in the reused
slot and returns `Truncated` from the exact received extent. A complete
frame that exceeds capacity rolls back before a smaller frame retries in
the same destination. The earlier published payload remains owned and
readable after both paths. [Hosted package test](../tests/compiler/packages/packages.rs)

The staged package also exercises actual wide and two-byte reads with the
ordinary compiler. The qualified macOS arm64 staging leaf changes the
checked transfer into the pending slot. Typed truncation, capacity rollback,
exact-slot retry, and previously published payloads pass the same package
gate at O0, O3, and with AddressSanitizer.
[Staging copy tests](../tests/compiler/backend/staging_copy.rs)

The typed and numeric frame fixtures agree across layouts at O0 and O3.
They use the same guards, wire inputs, and destination fields. The matched
byte-bearing campaign measured success and nested rejection separately across
32-, 64-, 65-, and 192-byte payload profiles.

Inspect branches, spills, reservation checks, marker stores, bytes copied, and
allocation and helper calls on both paths. Check record sizes and offsets across
Wide, Derived64, and Compact32 where eligible. Compare always-success, rare, and
mixed failures with matched optimized controls before attributing a speed change
to typed errors. Report combined attempt and retry timing separately from any
isolated bookkeeping measurement.

The earlier runtime-input scalar comparison covered all-success, rare, and
mixed validation. Its AArch64 typed loop inlined the validator, selected the
success payload and error tag, then masked and compared that tag again to
choose the final code. Numeric Holo combined validation with selection of the
final digest input. The loop path through the back edge contained 25 typed
instructions versus 18 numeric when
the length check passes, and 21 versus 11 for an oversized header. These are
static path counts for the trial binaries. Both inlined the call. The
adopted direct scalar lowering reduced the paths to 18 and 11 instructions.
It improved paired elapsed time across success, rare, and mixed profiles in
the exact-binary repeat. The append publication-edge trial produced identical
optimized binaries and left the existing append lowering active.

The outlined producer diagnostic confirmed that the pair crossed the tested
arm64 boundary in two registers. With parent and child outlined, the typed
parent normalized the child pair before returning. The numeric parent could
tail-call the child. The complete-attempt timings included that difference
along with dispatch and storage bookkeeping.

Direct forwarding recovered the native tail branch and improved the fully
outlined rejection profile. The broad trial regressed 64-byte success, and
the fixed-only trial regressed the fully outlined rare-failure profile in
three campaigns. The four-language comparison placed inline Holo within
3% of the faster C or Rust control on success and rare failures in the
repeat. The main inline success run and the outlined profiles left the gate
open at that stage. The subsequent checked SSA reservation lowering passed
the fixed-record gate in both runs.

The earlier late LLVM trial preserved inline and parent-only code and
recovered a tail branch with the child still outlined. Fully outlined
success and rare failures missed that trial's gate. A private-metadata
probe removed three parent-only reloads and reduced paired elapsed time
by 18% on success and 29% on rare failures in the repeat. That probe
motivated the checked SSA reservation lowering.

The [Goose compatibility checklist](spec.md#goose-performance-compatibility-checklist)
guides that review. This slice keeps the checked bounded data stack, bump
reservation, retained references, inline payloads, and marker reset. It implements
checked error tags and direct static dispatch for these fixed records. General
sequential error layouts, relative links, broader alignment choices, specialized generic callbacks,
and new check-elimination rules remain separate work. Queue transfer, relocatable
images, pools, and stable handles also retain their own obligations. Report
payload, reserved, committed, and resident memory separately when measurable.
