# Region reference model

The reference model evaluates the small region IR into ordinary safe Rust data.
It is an oracle for deterministic values and failure outcomes; it does not emit
machine code or prove the checker correct.

- `value.rs` owns independently represented records and payloads. It derives the
  fixture's byte accounting from mathematical sizes, without compiler layout code.
- `state.rs` owns lexical bindings, backing identities, batch generations, and
  typed producer inputs retaining captured source identities/extents.
- `expression.rs` evaluates ordered scalar observations and field projections.
- `construction.rs` builds owned record values and derives recoverable failures.
- `layout.rs` independently derives representation eligibility and length widths
  from logical schemas; it imports no compiler planner or offset helpers.
- `execution.rs` evaluates scopes, aliases, mutation, iteration, and publication.
- `ordinary.rs` evaluates synchronous borrowed calls with a fresh lexical frame,
  ordered logical prefix capture, and shared caller-owned state handles.
- `mod.rs` exposes bounded `run` (preserved wide control), `run_with_layout`, and
  model error classification. Only the explicit mode label is shared with Holo.

The initial native fixture format has an eight-byte record-size header, eight-byte
scalar fields, and an eight-byte length followed by each inline byte payload.
Nested records carry their own header. Model accounting uses `u128`, so native
`u64` size overflow can be detected without repeating native pointer arithmetic.

Explicit `Derived64` omits a record's size header when all fields are fixed or
the sole variable field is last; a nested last child may itself retain a header.
Multiple variable fields and a variable field before a following field retain an
eight-byte total. Empty headerless records occupy one initialized unit byte, so
sequential iteration always makes progress. `Compact32` additionally uses a checked
four-byte byte count and returns overflow status2 for counts above `u32::MAX`
before narrowing/reservation. The maximal allowed count may instead exhaust small
backing with status1. These rules are independently implemented in the oracle,
not obtained from a `LayoutPlan`. Model records retain their chosen prefix/length
width so completed-size and projection accounting remain independent of incremental
reservation. Shared schema DAGs are memoized during shape inspection to avoid
exponential oracle work; cycles/depth limits remain explicit errors.

Record fields are an ordered vector to make source order explicit. This is a
deliberately simple test representation; its allocations and lookup costs are not
Holo runtime performance evidence. Model-owned bytes are separate from native
backing storage, and no test model state is linked into generated executables.

Readonly byte inputs capture a batch generation, published record index, field
path and frozen length. Forwarding clones this small logical descriptor, never
the source payload. Input length/index expressions resolve those identities
against current initialized values; scalar arguments remain independent copied
values. Source and destination may use one batch because published records remain
available while a new record is built privately. A completed output owns its own
model byte vector, so later source mutation cannot change it. This host allocation
is an independent value representation, not evidence of native payload staging or
copy volume. Input bounds errors are model traps, while capacity/overflow/rejection
remain recoverable statuses with the existing append rollback.

Scalar `Apply` expressions evaluate every argument once, left to right, using the
caller's typed inputs and current byte index. Their body then runs in a fresh
scalar-only frame without caller byte/lexical storage observations or an inherited
byte index. This preserves trapping unused actuals and repeated parameter uses;
the oracle's temporary argument vector says nothing about native allocations.

Ordinary call inputs capture a record identity or logical byte prefix before
callee entry. The callee sees only its own bindings. Mutations through state
handles update the caller's existing model records and survive return. Boolean
calls and locals retain a Rust `bool` through copied parameters, returns, and conditions.
Logical operations evaluate the left operand first and skip the right when
the left determines the result.
Forwarding preserves the current prefix limit, and the input is unavailable
once its call frame ends.

A literal borrow retains shared immutable bytes and its narrowed range.
It needs no live batch and cannot become a write destination.

The model represents typed failures by family and case names and evaluates only
the selected result arm in a fresh binding scope. It uses no native ordinal or
representation when deciding whether a named or residual arm runs. A named
producer rejection carries only its optional copied `u64`. The enclosing append
restores the original marker before selecting a failure arm. A successful append
adds the initialized record before selecting the Unit arm. Typed byte fields
reuse the same independent size arithmetic, width checks, and element-by-element
model as numeric append. The numeric control
maps independently represented construction failures to its documented statuses.
`BindResult` invokes the helper once and saves its semantic outcome in the
current lexical scope. Later handlers select an arm from that saved outcome,
including named cases and scalar error payloads.
`TryLet` and `TrySaved` bind the semantic success or return a failure from the
ordinary model frame before executing subsequent operations. The saved form
reuses the previously evaluated outcome. Ordinary and nested
construction propagation preserve same-family cases and prepend the declared
child alias when lifting to an enclosing error union. Construction storage
failures retain their original identities and reach the append's rollback.
Whole-result forwarding requires exact nominal record and error identities
and evaluates the callee's actuals in order. The callee accounts for the
single destination extent, and its outcome returns directly to the caller.

`ConstructHandled` uses one model-only generation for a scoped record. A
successful arm can borrow its completed value by logical identity. Leaving the
arm invalidates the generation and restores the marker. A rejected attempt
restores the marker before a failure handler can retry. The model shares its
reference storage with batch elements for projection tests. The native
scoped path has no collection or indexed lookup.

The evaluator imposes a step/depth budget on its own test work and reports an
inconclusive model error when that budget is exceeded. It refuses clock readings
rather than inventing expected timing values. Hosted refill and its producer
actuals require a native or injected adapter witness. The memory oracle
never substitutes the published field's full extent for a fresh received count.
Small differential inputs assume
successful natural host allocation unless acquisition failure is explicitly
injected; actual allocator availability and physical address placement are not
predicted by this oracle.
Successful `ExitZero` stops remaining operations and iterations while enclosing
model scopes still restore markers and end owner lifetimes. Printed output before
completion is retained; this is entry-process completion, not a generic function
return or a prediction of host resource shutdown behavior.
