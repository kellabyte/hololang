# Source function definitions

This component lowers the explicitly experimental source subset into the existing
checked construction IR. `../functions.rs` owns name resolution, closed-call-graph
validation, contract checks, and bounded scalar-template substitution. `schema.rs`
resolves ordered inline schemas; `scalar.rs` handles pure u64 expressions and
conditions; `producer.rs` handles destination construction, exact result forwarding
and rejection.
`errors.rs` resolves child-family edges and orders expanded error leaves.
`tests.rs` contains focused source acceptance/rejection and compiler-budget checks.

Inputs are spanned frontend AST declarations. Outputs are schemas, destination
producers, pure scalar templates, and typed signatures for ordinary borrowed
functions. Error declarations share the type namespace with records. The
resolver rejects ambiguous and cyclic unions, preserves leaf order and payloads,
and budgets the generated paths. Each case can carry one `u64` field.
Fallible scalar headers retain their family for the parent frontend,
which lowers entry and ordinary bodies. Typed destination constructors keep
their schema and family together in the producer definition. The checked IR
verifier remains required before LLVM lowering. These modules do not depend on
legacy code, allocate generated-program heap objects,
or choose default source arithmetic semantics.

Scalar-helper arguments require `u64 as readonly copy`. Record constructors also
accept `Bytes as readonly reference`; their `core.len(input)` and `input[index]`
expressions read only the explicit borrowed input. Scalar helpers have one
pure return. A numeric literal constructor has optional `core.reject_if`
prefixes and one fresh literal. Fields must appear in declaration order.
Explicit `core.bytes` expressions initialize payload bytes, narrowing values
to their low eight bits. `core.index()` exists only in the element expression.
Returning an existing local
aggregate, arbitrary reference inputs, writable byte parameters, mutation, and
general function bodies remain outside these producer categories. Ordinary borrowed
functions may mutate their explicitly passed fixed state. They cannot enter a
producer or a pure scalar template. Owned byte output always uses explicit `core.bytes`;
direct field forwarding does not silently copy existing payloads.

Typed append producers have a final `return ok Record:` with every field
initialized in order. A prefix guard has the form
`if condition { return error Family.Case(payload) }`; its declared payload
is evaluated only on rejection. A typed nested field uses `try constructor(...)`
with the exact field schema and either the enclosing error family or one of
its declared direct children. This routes rejection
through the same destination chain without an aggregate temporary. Byte fields
use the same `core.bytes` initializer and checked input range as numeric
producers. General local bindings in typed producer bodies remain outside this
slice.

A producer can instead have one `return named_constructor(...)` statement.
Its callee must return the exact nominal record and, for typed results, the
exact error family. This forwards the existing destination, typed operands,
and outcome. It allocates no intermediate record. A child-family lift still
requires the explicit nested `try` path. Prefix rejection guards belong to
the fresh-literal form. Package binding resolves same-namespace callee and
type identities before this check.

Nested literals become synthetic destination producers with all outer typed
parameters explicitly forwarded. Byte descriptor slots cannot become numeric
scalars; input byte-index children participate in depth and substitution budgets.
This simple adapter may forward unused inputs; it introduces no aggregate temporary.
Named and synthetic producers retain
existing checked suffix rollback. Constructors cannot claim strict `noalloc`
because current lowering advances dynamic region storage. `noblock` and scalar
`noalloc` are admitted only after all source calls resolve to the supported closed
pure graph; this makes no wait-freedom, page-fault, or execution-time guarantee.

Scalar-helper invocations inside byte-bearing producers use the checked IR's
eager scoped scalar application. Actuals are evaluated once in written order,
then the inlined body sees a fresh scalar parameter frame. An unused or repeated
parameter therefore cannot erase a trapping input read or duplicate its runtime
evaluation. Ordinary scalar-only templates keep their prior nontrapping
substitution path. Both application actuals and its entire fresh-frame body count
toward compiler depth/node limits; substitution never captures body parameters
from an enclosing frame. This is compiler IR/SSA machinery, not a runtime closure,
payload copy, allocation, or indirect dispatch.

Compilation rejects cycles, expansion beyond depth 64, and generated definition
work beyond a cumulative 100,000-node budget. Substitution preflights the expanded
tree before cloning, including repeated parameter occurrences. These are explicit
implementation limits, not settled full-language restrictions.

From the repository root, check with `cargo fmt --check`,
`cargo clippy --offline --all-targets -- -D warnings`, and `cargo test --offline`.
Frontend acceptance tests require installed LLVM tooling for native execution;
missing tools fail those checks. Parsing alone does not establish memory or
performance guarantees.
