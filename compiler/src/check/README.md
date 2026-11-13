# Structured IR checking

This component validates the first compiler API before LLVM lowering. The public
boundary is `check()` in [../check.rs](../check.rs); only it constructs a
`CheckedProgram` borrowing immutable input.

- [definitions.rs](definitions.rs) checks unique schemas, producers and ordinary
  helpers, acyclic inline data and calls, and ordered constructor initialization.
- [expressions.rs](expressions.rs) checks scalar observations and field projections,
  separating pure constructor expressions from execution-time memory observations
  and ordinary call arguments.
- [scopes.rs](scopes.rs) checks lexical bindings, region/batch roots, readonly paths,
  stable append, reset restrictions, and the closed frame of ordinary helper bodies.
  Entry text printing rejects embedded NUL so the host prints the entire literal.

Programmatic [placement](../placement.rs) invokes the same verifier with a
private binding rule. The generated owner and constructor are identified only
for that borrowed validation pass. Ordinary entry operands and lexical binders
cannot capture the private name. The generated constructor still proves its
tail is free and checks the actuals and arms under ordinary name lookup.

Constructors have a destination-only instruction set, so writes to prior records,
global effects, and indirect calls are unrepresentable here. That is a deliberate
subset boundary, not proof of a future general function-effect checker.

Producer signatures and actual arguments distinguish copied u64 values from
synchronous readonly byte borrows. Entry byte arguments require a complete Bytes
projection through a live published record reference or the live received
binding of a refill data branch. Nested calls forward an existing Bytes formal.
Scalar expressions cannot reinterpret a byte formal as a number, and entry
code cannot fabricate an input parameter. Input length and byte observations
require a typed formal in a producer or ordinary helper. Byte-read bounds
remain runtime checks. Because a producer cannot store or return a borrow or
mutate published data, the live call-site root suffices for the synchronous graph.
Same-batch prefix inputs and compatible writable aliases remain valid. No blanket
non-aliasing or frozen-source promise follows from a readonly argument.
Internal scalar `Apply` expressions validate every eager actual in the enclosing
context, then validate their body in a fresh scalar-only parameter frame. This
prevents helper substitution from dropping unused trapping byte reads or duplicating
argument evaluation, without granting the body an outer byte/index capability.

Outermost `Op::Function` declarations establish the ordinary table before entry
checking. Their scalar-only record parameters borrow existing writable state.
Typed byte actuals capture a complete published field or forward an existing
formal, with an optional checked logical prefix. A hosted data branch can lend
its lexical received extent directly. Helper bodies use a fresh frame,
and the checker verifies scalar returns on every path and rejects unknown calls,
cycles, owner operations, address exports and external effects. The admitted
closed graph establishes the initial source effect summary without adding a
runtime ownership object. Multiple arguments can borrow the same compatible
state. The checker uses root identities to prevent lifetime escape and assigns no
alias attributes to those arguments.

Outermost `Op::ErrorFamily` declarations establish closed nominal families of
1–255 leaves. Each leaf has zero or one `u64` payload. A family can include
other nominal families under unique aliases. Child edges must be acyclic.
Each included family's leaves occupy one exact block in declaration order.
Within the block they preserve the child's tag order and payload shapes. The
checker can derive a constant tag offset with no runtime type registry.

A scalar-result helper returns a `u64` success, a declared error with its declared
payload shape, or an exact-family forwarded result on every path. Immediate
handlers evaluate one checked call and cover success plus every named or
residual error. The iterative expression walk establishes arm-local payload
scope, prevents error tags from entering scalar operations, and registers every
call edge, including calls in arms and forwarding returns, for cycle checking.
`TryLet` checks the same arguments and call edge. It accepts the same family or
a declared direct child. Its success continuation introduces an immutable
scalar and still requires a structurally valid return. Failure exits the
owner-free ordinary frame with the caller's declared family.

A typed destination producer uses the existing checked record schema and
ordered `u64`, Bytes, and nested field operations. Its named rejection
prefixes precede its first field initializer and
must provide the payload declared by its family. Typed nested producers accept
the same family or a declared direct child. A child failure reaches the owning
append with the outer family tag and original payload. The receiving scope
continues to own rollback of unfinished fields. `AppendHandled` verifies the
receiving schema and actual arguments through the same boundary as numeric
append. It requires an explicit Unit success arm and covers `Full`,
`SizeOverflow`, and each declared domain case, with an optional final failure
fallback. The selected case's payload is immutable and lexical to its arm. A
named fallback is opaque, and the immutable handler result is introduced only
after checking the arms.

`ConstructHandled` verifies the same outcome partition for a typed direct
producer. Its success body holds a region tail claim independent of source
names and exposes at most one immutable readonly record reference. The same
root-depth rules cover its aliases, byte projections, and attempted escape.
All failure bodies run without that claim, so they can retry in the same
region. The direct success name has no mutable reference slot.

A `Build::Forward` body must contain exactly that one operation. Checking
resolves the named producer and requires the same nominal schema and error
family. Its actuals use the ordinary scalar and readonly byte rules. The
forwarding edge participates in the same acyclic producer graph as a nested
field call, while its callee owns the complete destination initialization.

Reference bindings are lexical. The first checker supports `AssignRef` only when
the root remains the same; cross-root rebinding is explicitly unsupported after
the lifetime check. This avoids dropping possible origins across branches/loops
without pretending that general reference-flow analysis exists. Reset rejects
every currently named reference to the batch, including outer-scope aliases.

Hosted I/O operations are entry-only. Refill requires a writable published
Bytes field. Its data arm receives only a lexical readonly argument capability.
The checker rejects another refill of the same root and field path through any
alias until that arm ends, including when the received binding is shadowed.
The checker tracks batch roots and field paths, so different record indices
within one batch do not prove disjointness. Compatible byte writes through an
existing writable reference retain their ordinary alias semantics. The nondata
arm receives an immutable scalar status with no view. Explicit wait only binds
a scalar. A checked entry loop establishes a fresh lexical scope on each turn.

Send accepts a live initialized Bytes field through either access mode. Its
offset and count are scalar expressions checked in source order. The native
range guard checks their values before the byte pointer enters the adapter.
Writable wait and next-session status are immutable scalars. A session
transition is rejected while any refill data binding is still live.

The lexical TCP scope is the sole executable root when selected. Its opaque
owner cannot be used as a scalar or reference. Named synchronous attempts
require the same live owner and delegate byte and view checks to the
existing hosted operations. Explicit close consumes the owner directly
inside the TCP scope. Conditional close and loop-carried owner state
are future work. Nested successful returns still release the stream.
Implicit host I/O and session advancement are rejected in this profile.

Lookup tables and validation work lists allocate only in the host compiler. No
runtime reference counts, root scans, or `noalias` promises result from checking.
Entry-operation errors expose a stable zero-based preorder `operation_index()` so
a frontend can associate its own spans without adding source metadata to IR.
Parents precede children. Region success precedes failure, then precedes else,
and refill data precedes nondata.
The innermost rejecting operation retains its index during error propagation.
Definition errors have no operation index. `ExitZero` is entry-only successful
completion; unreachable operations are still structurally checked.
Clock observations and opaque-region barriers are execution-only measurement IR;
they do not select source-language APIs or satisfy constructor purity.
Run `cargo test --test check --test check_locations` from the repository root for positive and negative
coverage, and use the root validation commands for the whole implementation.
