# Entry and ordinary function lowering

`../lower.rs` owns lexical source facts, distinct IR names, operation spans,
expansion limits, and the entry adapter. `declarations.rs` lowers explicit
region acquisition, tail ownership, destination appends, and references.
`expressions.rs` resolves typed field paths, captures call arguments once in
written order, and instantiates closed scalar helpers. `statements.rs` handles
branches, iteration, hosted refill, fixed-shape writes, and entry completion.
`ordinary.rs` binds each helper's typed formals in its own source scope and lowers
its checked call expressions. Arguments stay inside the call node so a prefix
bounds check precedes all later actuals. Pure helper calls nested in an actual
use an eager scalar expression frame. The existing entry and producer paths keep
their direct captures.

`hosted.rs` lowers send, readiness, next session, and explicit close to
entry operations with immutable scalar statuses. Send holds offset and
count inside the operation so the byte projection is captured before
their evaluation. A lexical TCP binding resolves to a distinct checked
owner identity. Explicit refill, send, and readiness wrap the existing
operation with that identity and keep the wrapper and underlying attempt
at one source position. Refill outcomes retain their separate scope
positions. The checker controls where the host owner can bind and whether
each attempt or consuming close uses a live binding.

`construction.rs` lowers entry's `core.construct(region, producer(...))`
into one checked operation with ordered outcome blocks. Each block has its
own lexical scope. Only `on ok(record)` introduces the readonly record
reference. Named rejections use the existing nominal case and scalar
payload resolver. The fallback covers storage and domain failures without
a bound reason. Actuals stay in the operation for destination-first
evaluation. The verifier enforces the live region tail and reference lifetime.

`with core.region` selects one constructor by its source span before explicit
owner declarations wrap IR tails. The owner lowers to an ordinary region
with separate brace scopes. The one-operand constructor supplies the private
region name and shares argument and handler lowering with the named form.
The final preorder span map gives checking both exact operation identities.

`results.rs` checks nominal scalar returns, exact forwarding, and `on` arm
coverage and payload scope. A payload-bearing return supplies one `u64` to its
declared case. `on ok` discards the success value, `on ok(value)` binds it
immutably, and `on ok(_)` explicitly discards it. Named error arms can bind
their payload or discard it with `_`. Actual arguments and arm expressions
stay inside the checked result node, so selecting an arm cannot execute
another arm's prelude. Boolean literal arms can initialize a condition with
`let`.
An immutable `let value = try call(...)` in a scalar-result function lowers to
one ordered call with a success binding and an ordinary failure return. The
actuals resolve before the binding is visible. Entry and plain scalar helpers
have no propagation exit. Typed record constructors support a separate
`try` in nested destination fields, including declared direct child families.

A typed destination append uses the existing named batch and ordered actuals.
The `on ok` arm has no payload. `on Rejected Case(value)` selects a constructor
failure, while `on Full` and `on SizeOverflow` cover storage failures.
`on error` can cover the remaining failures. The verifier checks the partition.
Each arm runs after publication or rollback with a private payload scope.
Entry can bind a `u64` or boolean-literal decision with `let`. Direct entry
`return core.append(...)` permits literal zero in every arm and uses the same
cleanup as `return 0`.

Destination-producer calls use typed scalar and readonly-byte arguments. Byte
actuals can name complete payload projections through live record references.
A refill data branch can also lend its received name directly to a readonly
`Bytes` parameter in a synchronous ordinary call, append, or scoped
construction. The received
binding cannot escape as a local value. Source-order scalar captures precede
the emitted append. Published field descriptor metadata loads can occur inside
append because allowed argument effects cannot rebind caller roots/projections
or resize payloads, and validated descriptor resolution cannot fail. This
preserves source observations without adding a runtime descriptor-binding layer.
The checker independently validates the originating live root and field kind
or verifies the fresh refill capability in its lexical scope.

The output is ordinary unchecked construction IR; `SourceProgram::check` invokes
the existing access/lifetime checker. No second memory model or runtime ownership
tracking is introduced. `let`/`var` binding mutability is checked here separately
from reference path access. Readonly aliases may observe another alias's writes.

Library package checking uses the same declaration and ordinary body lowering.
It supplies no entry statements. This allows the checked core to validate every
ordinary definition and its call graph even when the library has no caller.

Owners consume the remainder of their source block and use existing lexical
marker cleanup. Acquisition failure has explicit recovery without an installed
owner. A handler may terminate entry, or finish its owner's entire lexical block
and rejoin an enclosing continuation. It cannot enter the success-only tail.
The failed owner is bound only after handler lowering; handler/success locals
remain in their separate scopes. Explicit source retries acquire their own
owners and costs. Main's region-aware check rejects normal fallthrough. An
unconditional entry `loop` has no fallthrough. It can still leave entry through
`return 0`. The reverse fold avoids recursion in flat declaration sequences. Early
`return 0` uses the checked core's cleanup-aware entry exit. Entry and each
ordinary body have their own preorder source map, including conditional branches.
Entry also maps region success before recovery and refill data before nondata.
Refill keeps its limit as an ordered expression so the destination is captured
first. Its two outcomes each get a fresh lexical binding. Explicit readiness
wait is a `let` initializer with a copied status. Generated temporary bindings
are compiler-only.

This experimental subset accepts pure scalar helper inlining and direct borrowed calls.
Implicit backing for aggregate locals and aggregate transfers remain outside the
subset. Explicit wrapping operations avoid choosing unresolved numeric
defaults. Entry expression/template work has a 100,000-unit budget and lowering
nesting is capped at 64. Captures, spans, and statement nodes are not all charged;
this is not a total compiler-memory cap or program memory guarantee. Destination
byte bounds are checked before RHS evaluation, then checked again by the existing
store path; no generated-code elimination of the second check is promised.

Run `cargo test --offline --test frontend --test frontend_native` from the root.
All public source entry points remain explicitly experimental.
