# LLVM backend

These Rust modules generate LLVM; the [native runtime support](../../../../runtime/README.md)
is maintained separately. The public Rust module remains `holo::llvm`.

This component lowers the checked region representation into LLVM IR. Its entry
point accepts checked programs; source-level lifetime and effect decisions belong
to the validator, not to backend guesses about pointer behavior.

The [finite M1 review](../../../../docs/implementation-plan.md#m1-acceptance-decision--november-10)
accepts this core under an explicit native profile. Failed generated checks call
the target trap before invalid access, with no Holo logging/cleanup before host
transfer. Darwin arm64 driver links use eager chained imports; reviewed native
copy/fill/zeroing helper assumptions belong to the acceptance profile. This is
not a general effect verifier or a claim that every runtime/target is certified.

| File | Owned responsibility |
|---|---|
| `mod.rs` | Checked-only module entry point and whole-module assembly, including static text constants for entry printing. |
| `byte_sum.rs` | Checked-loop selection and exact-width native wrapping reductions. |
| `context.rs` | Per-function lexical bindings, active cleanup descriptors, and typed inline projections. |
| `copy.rs` | Guarded subrange-copy policy, bounded inline transfers, and shared checked fallback. |
| `cursor.rs` | Linear traversal over a captured published prefix, independent of reference rebindings. |
| `cursor_inventory.rs` | Lexical writes, checked slot publication, exact extents, and bounded scalar facts for staging. |
| `cursor_proof.rs` | Inductive proof of nonwrapping source and receiving ranges. |
| `cursor_staging.rs` | Exact transfers at entry sites selected by the cursor proof and the opt-in wide-copy leaf. |
| `uniform_batch.rs` | Bounded opt-in proof of one initialized byte extent across every publication in a lexical batch. |
| `function.rs` | Unique LLVM values/blocks and fixed entry stack slots. |
| `input.rs` | Ordered typed argument capture and forwarding of transient readonly byte locations/extents. |
| `expression.rs` | Ordered unsigned arithmetic, comparisons, and checked observations. |
| `operations.rs` | Ordinary scalar/reference operations and dispatch to structured emitters. |
| `ordinary.rs` | Internal synchronous helpers, ordered typed captures and checked logical prefixes. |
| `refill.rs` | Hosted input ABI, direct destination capture, bounds and result checks, and explicit readiness wait. |
| `send.rs` | Hosted output ABI, checked source subrange and result, and explicit writable wait. |
| `session.rs` | Optional current-client resolution and checked session transition. |
| `tcp.rs` | Dedicated accepted TCP entry operands, static ownership cleanup, and checked readiness and terminal close statuses. |
| `scope.rs` | Region acquisition/free, batch marker cleanup, loops, and branch scopes. |
| `staging_copy.rs` | Opt-in range and disjointness proof for checked entry byte staging loops. |
| `staging_slots.rs` | Lexical backing proof and dominating byte descriptors for the stable-slot staging trial. |
| `static_byte_sum.rs` | Tiny and large pure sums selected at literal checked-prefix calls after ordinary argument capture. |
| `exit.rs` | Successful entry completion with reverse lexical cleanup before host exit. |
| `producer.rs` | Nested final-destination initialization and recoverable error propagation. |
| `publication.rs` | Destination call capture, append commit, scoped record dispatch, and marker rollback/reset. |
| `representation.rs` | Typed record extents and indexed lookup from the selected schema layout. |
| `storage.rs` | Metadata access, packed loads/stores, and checked reservation emission. |
| [runtime/llvm/prelude.ll](../../../../runtime/llvm/prelude.ll) | Embedded native declarations, fixed metadata types, and terminating assertions. |
| [runtime/measurement.c](../../../../runtime/measurement.c) | Conditional monotonic timing and an opaque memory barrier; no data-processing kernels. |
| [runtime/refill](../../../../runtime/refill/README.md) | Conditional native process entry and POSIX input boundary. |

Region metadata stores base/capacity/top; batch
metadata stores its region/start/published end/count. The preserved `emit` API
selects `Wide`: every record has an eight-byte total-size header and byte payloads
have eight-byte lengths. `emit_with_layout` accepts the explicit `Derived64` and
`Compact32` experiments through the same lowering path and returns planning errors
before emission. `Derived64` omits derivable record headers while retaining u64
lengths. `Compact32` uses checked four-byte payload lengths, widened before every
64-bit index/address calculation. No implicit widening, truncation, or relocation
fallback occurs. All record accesses use alignment 1.

The owning layout planner decides each schema's retained header, positive empty
unit, and fixed extent. Fixed schemas and a single final variable field can omit
their total header. More complex variable schemas retain one for constant-work
skips; a nested child's plan remains independent. Headerless empty records write
one unit byte so traversal always progresses. Fixed extents return constants;
other typed helpers read frozen length/child-size metadata only, never payloads.
No runtime layout callback, per-record descriptor, or second backend is introduced.

The emitter passes the final receiving address to each constructor. An append
publishes count and end only on success. A scoped record borrows the same
address in its success arm. Tail release restores a saved top without enumerating
nested values. The driver handles tool discovery, target selection, and native
linking.

The experimental TCP entry receives an admitted descriptor in SSA and may
borrow a host-owned readiness queue. Its single outer lexical owner adds one
native close on each reachable normal exit. A direct explicit close removes
that cleanup at compile time. Existing region and marker cleanup remains
separate, and no runtime flag or handle table tracks the owner. Wrapped
send and refill reuse the existing byte capture and range checks. Named
waits delegate to the selected host readiness adapter.

Scalar-only producer signatures retain their u64 operand ABI. Each readonly byte
formal adds ordinary pointer/u64-length operands, not an allocated descriptor or
stored record field. Entry capture decodes an initialized field's length and location
without reading payload bytes; nested calls forward those operands. Input byte
reads check the captured extent before forming/accessing the element pointer.
In emitted original IR, byte-field reservation precedes element evaluation, so a
failed reservation or zero-length output has no initializer payload loads. The
borrow has no runtime ownership tracking and adds no `noalias` attribute; same-root
prefix inputs and overlapping input borrows remain valid checked cases.

A checked hosted entry keeps the same storage backend. Refill captures the
existing field before evaluating the request, validates the positive limit
against the slot and native cap, and emits one adapter call. A validated native
count becomes the readonly operand extent. Ordinary calls and destination
constructors lend these original SSA operands to readonly formals. The capture adds
no metadata load or payload copy. A producer's `core.bytes` still initializes
owned output through the usual destination path. Explicit wait emits its own
adapter call with no payload operand. Successful entry return runs active storage cleanup and returns
to the native host, which owns descriptor close.

Send captures an initialized byte-field address and extent, then evaluates
offset and count once. Guards prove the subrange before forming its start
pointer. A single adapter call returns bounded positive progress or an
encoded status, which the emitter validates before binding it in SSA.
Writable waits and session transitions introduce no payload storage.
Only session-aware entry code resolves descriptors through the current client
host state. Output and session modules have separate optional C linkage.

The [short-copy investigation](../../../../evidence/short-copy-20261109/README.md)
confirmed that LLVM already propagates known lengths through these calls and
published-field projections in the tested cases. The input benchmark's explicit
source-opacity boundary hides that knowledge. A real source length assertion per
batch restores constant-length optimization while leaving payload opacity intact.
Neither observation changes readonly permissions or the measurement boundary.

A general experimental lowering recognized only exact same-input identity
initializers, used exact chunks for lengths up to 32, and emitted LLVM memcpy for
larger inputs after the original reservation checks. Two paired campaigns showed
clear short-copy gains alongside repeated wide-layout median regressions, larger
code, and an optimized spare-backing read. It was not adopted. Its source, proofs,
semantic checks, complete measurements and code inspection remain in the archive;
the ordinary initializer lowering is restored without an alternate compiler path.
The native boundary sweep remains active. Exact original accesses do not establish
exact optimized or physical memory traffic.

A [whole-input memcpy follow-up](../../../../evidence/copy-and-recovery-20261109/README.md)
kept the exact captured-extent identity proof but used one bulk intrinsic at all
lengths. It passed O0/O3/ASan semantic checks, yet two adjacent-pair campaigns showed
no broad gain and a repeated wide 4096-byte slowdown. It was rejected; this backend
and runtime are restored to the original inputs. The stronger audit accepts only
that reviewed canonical bulk form when supplied, with unchanged reservation
guards and CFG dominance. No bulk-copy compiler flag or runtime policy remains.
The final recovery feature changes source acceptance/termination checking only;
thirteen benchmark sources emit byte-identical LLVM to the old control.

The guarded subrange policy on macOS arm64 recognizes one direct borrowed-byte
read at the comprehension index plus a nonzero constant offset. A destination
length expression equal to the same captured input length uses the ordinary loop.
Zero-offset copies and compound initializer expressions also use that loop. After the original
width, overflow, and capacity checks, a nonempty transfer must fit the captured
source suffix. Only then can the backend form the shifted pointer and copy.
The checked producer call graph supplies disjointness between initialized
published source ranges and the private receiving suffix. The policy adds no
blanket alias promise.

Proved ranges of up to 64 bytes use fixed inline LLVM chunks, including 32-byte
head and tail chunks for sizes 33 through 64. Head and tail
chunks stay inside both ranges in the emitted IR and may write an identical
byte twice at overlapping offsets. Longer proved ranges use LLVM bulk copy.
Empty copies skip the input access. A failed extent proof calls the shared
original initializer loop so each byte still reaches its own bounds check in
order. This uncommon path is outlined. Producers that can reach the guarded
copy get LLVM's ordinary inlining hint so caller-known lengths and checks can
simplify the dispatch. LLVM retains the cost decision. `holo inspect-copies`
and the emitted IR comments report the same target and expression selection.

Scalar `Apply` expressions capture actuals into SSA operands once, left to right,
and emit the pure body using a fresh scalar parameter frame. They need no runtime
closure, heap object, or native function dispatch; unused trapping arguments still
execute. The enclosing byte-input/index frame is restored after body emission.
Ordinary helpers emit separate internal functions with direct scalar results.
State references pass a pointer and readonly byte arguments pass a pointer and
captured length. A prefix check runs after source descriptor capture and before
later arguments and the call. The bound follows forwarded inputs and controls
every checked load. State references and scalar locals use fixed frame slots at
unoptimized levels. Arguments and returns introduce no payload copy or dynamic
allocation, and the call ABI carries no uniqueness attribute.
The macOS arm64 byte-sum selector recognizes counted loops over one live frozen
field or captured borrowed byte extent. The loop may only read the current index
and add its unsigned byte to a private scalar modulo 2^64. Parsed argument
captures and the direct checked-IR form use the same proof. The exact match rules
leave alternate bounds, indices, operators, aliased field updates, and additional
work on the original checked loop. The recognized body cannot mutate the payload,
rebind the reference, or change the captured extent. The optimization uses no
alias promise and preserves writes through any alias before or after the loop.
The emitted helper reads alignment-1 blocks of sixteen bytes and guarded 8/4/2/1
byte tails. Empty extents read no payload. Other targets keep scalar lowering.
An isolated 64-byte helper improved bulk reader scans but regressed short
scanner and lifecycle workloads. [Outlined 64-byte gate](../../../../evidence/outlined-64-sum-gate-20261111/README.md)

An early `ExitZero` emits active batch marker restores and region frees in reverse
acquisition order before calling host `exit(0)`. Compiler cleanup descriptors remain
available while emitting sibling paths. Its unreachable continuation cannot run
normal scope cleanup a second time. This entry operation carries no general return
value and does not decide the source language's broader return semantics.

The bounded reservation checks are part of this initial configuration's costs.
LLVM may optimize them only when their conditions are established. Mutable and
read-only source access do not supply `noalias` promises. Native tests must cover
both optimized and unoptimized code and independently expected outputs.

`storage::reserve` emits those checks directly at each original reservation site:
checked u64 addition, overflow return 2, then capacity comparison, exhaustion
return 1, and only then address formation/top publication. This preserves order
and marker effects while avoiding a temporary `{pointer,status}` helper result
whose immediate status test produced redundant native branches. No check is
removed, fused, reordered across a field, or specialized for a benchmark.

A [construction-success branch-layout trial](../../../../evidence/construction-branch-20261110/README.md)
added fixed 2000:1 LLVM branch weights without changing guards, evaluation order,
status values or rollback. All nine lifecycle paired medians improved, but a
wide-layout 4096-byte copy slowdown reproduced in an exact-binary repeat. The
trial was not adopted: after it, the five emitter inputs were restored byte for
byte, with no broad hint policy or format-specific workaround retained.
The archive preserves the complete candidate, tests, code inspection, wins and
regressions; the successful placement change is not a general performance rule.

An [accepted narrow follow-up](../../../../evidence/construction-followup-20261110/README.md)
applies the same fixed 2000:1 layout metadata
only to producers whose checked signatures contain solely U64 inputs. Checked
producer byte arguments can only forward Bytes formals, while direct field
borrows require entry code; therefore this scope excludes transitive borrowed
payload reads. Unused Bytes inputs are conservatively excluded. Producer guard,
width, size and nested-status branches use that local scope; an entry append
chooses independently from its callee, so mixed fresh/copy appends cannot leak
hint state. Explicit rejection, initializer loops and user branches stay ordinary.
This selects an optimizer heuristic, never a safety or failure-frequency promise.
Scalar source seeding inside a copy module can still receive hints and perturb
whole-module inlining/register allocation, so unchanged copy performance requires
actual paired evidence rather than this classification alone. The mixed-signature
emission unit test covers direct/nested/unused byte inputs, eager trapping actuals
and alternating eligible/ineligible entry calls; existing O0/O3/ASan fixtures
remain the separate semantic checks. Default lifecycle paired medians improve
about 3% at all three measured sizes; full-copy intervals cross zero and ten copy
native pairs remain unchanged. The broader fixed-weight policy stays rejected.
Final 191-test/format/Clippy and rebuilt identity validation is recorded with this
finite selection; metadata alone is not a speed claim or a public language rule.

A remaining-capacity experiment proved a one-guard success path using
`bytes <= capacity - top` under the existing valid-marker invariant, with overflow
classification only on failure. It preserved behavior but regressed the small
default-layout workload and lacked a convincing larger-case default gain in paired
results. The original helper above is retained. Its candidate code, proof, native
artifacts and unfavorable results remain in the
[experiment archive](../../../../evidence/remaining-capacity-comparison-20261109/README.md);
the new native exact-fit/failure-order regressions remain active. No format-specific
second reservation implementation was introduced to salvage a narrower result.

Indexed borrowing of variable-sized records walks from the batch start and costs
O(index). `ForEach` instead captures the original published prefix and performs one
typed frozen-extent hop per record. Appended records are excluded from that pass,
and reassigning its source reference cannot redirect the physical cursor. The
captured end and count describe the same prefix, so native traversal needs only
the end boundary; an unused count observation can be optimized away.

The opt-in `emit_with_uniform_batch_extents` trial proves every append to a
lexical single-byte-field batch has the same literal or freshly captured literal
extent. It checks all structured branches and loop bodies. Same-name nested
owners have separate publication. Mixed, dynamic, or unproved extents retain
the ordinary path. On macOS arm64 Derived64, proved cursors use the existing
physical header width plus that extent for each hop. Direct cursor byte
operations and borrowed call descriptors reuse the exact bound. The checker
keeps reference rebinding on the same root, and later appends retain the same
shape. Original index checks, append outcomes, captured end, and reset behavior
remain in effect. The trial adds no native metadata or allocation.

Constructor evaluation/failure order remains field ordered. Retained size headers
or empty unit bytes reserve first, then ordinary scalar/nested/byte operations
reserve their own extents. Narrow byte lengths above u32::MAX return construction
status 2 at that field before length truncation, reservation, or initialization.
All size additions and capacities remain checked in u64. The owning append still
publishes only complete output and performs one suffix rollback on any failure.
Different physical formats naturally change capacity outcomes; these experiments
do not silently redefine the source `Bytes` default or remove its wider control.

Byte initializer values retain the full-u64 expression path and truncate only the
stored byte. A private low-byte projection experiment preserved all values and
exposed a shorter byte-SIMD loop, but its measured construction costs regressed;
it was not adopted. The original expression/producer lowering is restored without
an alternate path or experiment flag. Candidate code, proofs, raw results and
before/after binaries remain in the
[byte-initializer archive](../../../../evidence/byte-initializer-comparison-20261109/README.md).
Two additional native fixtures protect all five wrapping/bitwise operations,
high scalar bits, long physical indices, failure order and retained-reference reuse.
Lengths, indices, scalar fields, calls, guards, numeric rules and source eligibility
remain unchanged. Smaller SIMD instruction counts alone do not establish a speedup.

A separately tested exact-vector-prefix follow-up preserved the original u64
scalar expression source and used exact eight-byte prefix stores for closed pure
trees. Its private end-counter assignment exposed the proved prefix exit without
an LLVM assumption, but actual scalar machine bodies became five instructions
instead of four and retained an unreachable u64 vector fallback. Default/compact
lifecycle gains were less consistent than the selected narrow heuristic and eight
input/failure review flags remained adverse descriptive observations. This vector
candidate is not adopted; its module is absent from active source and no unmeasured
hint/vector combination is enabled. Complete source, tests and results remain in
the [follow-up archive](../../../../evidence/construction-followup-20261110/README.md).

Explicit clock and opaque-region operations call the native support boundary.
They are execution-only measurement facilities, not constructor expressions or
new public language syntax. Neither call has a purity/non-aliasing attribute;
instrumentation and its overhead must be disclosed in timing comparisons.

The opt-in `emit_with_staging_copy` experiment in `staging_copy.rs` operates on
checked entry loops on macOS arm64. It matches one forward byte-field
assignment, with the parser's pure captured scalar indices and destination
guard when present. Up to four bytes use the original loop directly. Larger
counts capture frozen descriptors, prove both complete unsigned subranges,
and check disjointness with the distance between their native addresses.
The proved path shares the guarded constructor copy's inline chunk geometry
through 64 bytes and bulk LLVM copy above that. Failed proofs use the original
loop, preserving forward-visible overlap, partial writes, and trap order.
The ordinary emitter remains unchanged.

The separate `emit_with_stable_staging_slots` trial can hoist one pair of
field descriptors into a positive-trip enclosing counted loop. The two
reference slots must have known distinct live region acquisition identities.
Borrowing from a batch supplies its acquisition identity. An alias inherits
only an established identity, and any possible rebind excludes the name.
The proof also excludes names introduced by a nested borrow, alias, or
cursor and checks the exact lowered reference slots and field paths at
each reuse. An empty enclosing loop skips descriptor capture.

Each fragment still checks its complete unsigned bounds against the cached
extents before forming addresses. Distinct allocations eliminate the
per-fragment separation test. One-to-four-byte fragments use exact inline
chunks, and larger copies keep the short and bulk policies. A failed range
proof runs the original loop with its original trap order and prefix writes.
The captured fields have frozen shape and live backing throughout the
checked loop. No LLVM alias attributes or native proof objects are needed.

The release-shaped `emit_with_proved_staging_slots` candidate enables only
this proof on the same target. A site without the captured slot pair follows
ordinary scalar lowering, including when another transfer in the same loop
qualifies. The ordinary entry point remains the baseline for qualification.

The ordinary macOS arm64 Derived64 emitter also uses checked constructor
and cursor facts. A direct successful append of a single byte field
establishes an exact extent for the stable borrow of that published
index. The source cursor must carry that extent through the existing
room assertion and exact update. The receiving cursor must stay within
its initialized boundary across the fragment minimum, updates, and
retry or reset transitions. Checked assertions remain in source order.
Disjoint live acquisition identities allow the proved copy without
rechecking each byte. The native one-byte arm precedes the two- and
three-byte arms, and wider transfers use the existing geometry.

The planner only runs for modules with a direct byte constructor. It
records lexical declarations and writes once, caps setup and copy-site
scans, and bounds capture resolution and scalar interval evaluation.
Reassigned captures, re-created extents that can shrink under a carried
cursor, separately reinitialized receiver bounds, and unrecognized
effects decline selection. The plan belongs to entry lowering and is
compiler-only. [Cursor proof and performance gate](../../../../evidence/staging-cursor-proof-20261111/README.md)

Ordinary macOS arm64 Derived64 lowering outlines the wide edge when the
proved maximum transfer is 65–256 bytes. The receiving extent provides the
maximum, and a bounded interval for the initial requested amount can tighten
it because the copy loop only narrows that amount. Sites outside the qualified
maximum use the existing bulk intrinsic. The compile-time choice adds no
runtime size branch. The shared `preserve_mostcc` leaf copies full 64-byte
blocks and finishes at the checked end. An overlapping tail repeats a write
from disjoint source storage. Tiny and short arms and typed producer copies
use their existing lowering. The API `emit_with_outlined_bulk_staging` and
example flag `--outlined-bulk` also allow larger proved sites for experiments.
[Packaged staging qualification](../../../../evidence/packaged-reader-gate-20261111/README.md)

The general macOS arm64 byte sum uses the outlined 16-byte loop and
exact-width tails. A literal checked prefix of at least 256 bytes at a
recognized pure call selects a separate outlined 64-byte reduction.
Ordinary argument capture and the prefix check still run in source order.
The large routine loads at alignment one, reduces each complete block
into a 16-bit subtotal and adds it to the wrapping 64-bit accumulator.
A nonempty remainder goes to the general helper. Modules without a
qualifying literal retain the existing helper and call sites.
[Large-prefix gate](../../../../evidence/proved-large-sum-20261111/README.md)

The ordinary macOS arm64 Derived64 emitter also selects the existing
wide helper for an exact 192-byte literal. The same pure-callee proof and
eager prefix check apply. Runtime extents and neighboring literals keep
their earlier call edges. The selection is compile-time only. Other
layouts can exercise the policy through `emit_with_exact_three_block_sums`.
[Release qualification](../../../../evidence/exact-192-default-20261112/README.md)

Scalar typed returns and immediate handlers are lowered by `results.rs`. On
the ordinary call path, the private `{ i64, i8 }` result uses the first
lane for the active success or error scalar. It sets that lane to zero for a
tag-only case and encodes error cases by their checked ordinals in the tag lane.
A handler evaluates its subject once, switches on the tag, and merges the
selected scalar with a phi. Only an arm with an active payload binding extracts
the scalar lane. This adds no runtime allocation or diagnostic buffers. The
private representation can vary with target and lowering.
An immediate handler of a small checked return chain over copied scalars can
emit the guard sequence in the caller and branch from each known result to
its selected arm. Actuals remain eager and ordered. A payload with several
incoming returns uses a phi. A per-site expression budget bounds code growth.
Forwarding, memory operations, calls inside the callee, and other ineligible
shapes retain the ordinary ABI and switch.
`results.rs` lowers `TryLet` through the same ordinary call. A tag test sends
failures to an immediate return. When a child error family occupies a later
range in the enclosing union, the failure block adds a constant to the tag
and keeps the payload lane intact. Identity and first-range lifts return the
original pair. The success block extracts and binds the scalar. Ordinary result
helpers have no region owners, and propagation uses the same private return pair.

Typed append producers use a private `{ i64, i64 }` return. The
first lane is `0` for success, `1` for `Full`, `2` for `SizeOverflow`, and
`4 + ordinal` for a checked producer error case, with zero-based ordinals.
The second lane holds the full `u64` for a payload case, the checked end for
a deferred variable-size success, or zero otherwise. Same-family child
producers return the original pair on
failure. A child family declared in the enclosing union uses a constant offset
for declared rejections and preserves the storage statuses and payload lane.
`producer_lifts.rs` plans fixed terminal calls whose child can encode that
offset directly. Each specialized producer changes its named rejection constants
and uses the same hidden operands and return lanes. The planner composes
contiguous offsets along eligible terminal edges and deduplicates them. It
emits at most four variants per producer. A call beyond that budget uses the
generic definition and adjusts the error on the caller's failure edge.
Nested producers write to their final address in the receiving region and use
no per-call rollback marker. An earlier forwarding trial retained
per-field marker writes and regressed representative workloads.
[Forwarding trials](../../../../evidence/error-reservation-20261111/README.md)

A whole-result forwarding producer emits its direct call before the usual
header or empty-unit reservation. The callee receives the original marker
and destination or the original SSA destination, cursor, and capacity.
Its original return status or pair goes back to the caller. If an enclosing
fixed child edge requested a lifted wrapper, the wrapper translates only
declared rejection statuses with the existing mapping. The callee owns
the sole record initialization. The destination boundary owns rollback.

Checked typed producers with a fixed, headerless layout use a private
destination, cursor, and capacity as SSA operands across calls. The append
derives the address and cursor from the same saved checkpoint. The producer
computes its field offsets from the planned layout and passes the appropriate
address and cursor to each fixed child. Following a successful child, the
next field uses the child's exact extent to recover the checked cursor.
Each reserve still checks overflow before capacity and updates the private
cursor before initializing the reserved bytes. The fixed subtree has no
region descriptor operand and leaves its marker untouched. Within the fixed
subtree, a final child with a compatible encoding returns the common
typed pair directly. The encoded pair can come from the original definition or
a specialized union variant. The macOS arm64 outlined control emits a tail
branch to the specialized child. [Native union gate](../../../../evidence/union-rejection-lift-20261111/README.md)
These rules add no LLVM aliasing promises or native storage slots.

Derived64 and Compact32 typed byte-bearing producers receive the same
destination, cursor and capacity operands. `DeferredReservations` keeps the
absolute cursor and matching tail pointer in SSA. A continuing parent uses a
fixed child's exact extent or a variable child's checked end. Retained size
headers are filled from the end minus the start. A headerless terminal
variable child in the same error family can supply the entire return pair.
Each reservation keeps its original width, overflow, capacity, and input
evaluation order. Numeric producers and the Wide comparison keep the
marker-based convention.
[Byte-bearing reservation evidence](../../../../evidence/byte-reservation-comparison-20261111/README.md)

On macOS arm64 in Derived64, `producer_inline.rs` selects both definitions
of an eligible terminal typed parent and guarded-copy leaf. Both use the same
error family and headerless deferred cursor ABI. The parent is reached by
an entry destination call. The leaf has one incoming constructor edge and no
standalone destination call. The pair budget charges checked operations,
expression nodes, and the copy dispatcher. The module budget weights each pair
by the number of syntactic entry call sites. Those are compiler cost units. All checks, actual
evaluation, and publication remain in the checked bodies.
[Bounded inlining gate](../../../../evidence/bounded-reader-20261111/README.md)

Numeric and typed append use the same publication helper in `publication.rs`.
It publishes the count and end on success. A successful cursor-based append
also stores the region marker once. If an append fails, it restores the saved marker
only for a producer that could have advanced it. Typed dispatch runs after this
join. A named error arm extracts the
payload only when it binds that value, and the selected arm's scalar flows into
an SSA phi. The payload lane is independent of the discarded suffix. This path
adds no runtime calls, preallocated payload storage, or unwind machinery. The
numeric producer ABI remains `i64`.

Scoped construction takes one region snapshot and derives the final address
from it before capturing actual arguments. It calls the same typed producer
and dispatches on the same status encoding. The success arm commits a fixed
or deferred cursor when needed and binds the final pointer in SSA. An explicit
rebindable alias may store that pointer in its own slot. Normal arm exit restores
one marker. Entry return uses the same active cleanup in reverse scope order.
Each Marked failure edge restores the saved marker before executing its arm.
Cursor-based failures have already left the marker unchanged. The local needs
no batch metadata, indexed lookup, temporary record, or extra backing.
[Scoped record source rules](../../../../docs/spec.md#fresh-aggregate-locals)

From the repository root, `cargo test --offline --test native` exercises actual
LLVM verification/code generation, both optimization modes, valid ASan execution,
and a known-bad sanitizer control. `cargo test --offline --test entry_exit` checks
nested early completion, model agreement, and instrumented reverse-order region
release at O0/O3 and under ASan. `cargo clippy --offline --lib -- -D warnings`
checks host implementation quality. A successful LLVM verifier result establishes
IR validity; it does not independently prove the source checker sound.
