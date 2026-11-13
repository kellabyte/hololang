# Next milestone: a checked region core and four-language evidence

**November 8, 2026 — execution plan for [the fresh specification](spec.md).**

**Repository organization updated November 9, 2026.** The root workspace separates
[compiler](../compiler/README.md), [CLI](../cli/README.md),
[native runtime support](../runtime/README.md), and the future
[Holo library home](../lib/README.md). Application demos are in
[examples](../examples/README.md), with [Holo regression programs](../tests/fixtures/README.md)
under the test suite and Rust API examples under the compiler.
Architecture remains in [compiler.md](compiler.md). The standalone prototype
was retired on November 12. Historical manifests and archived documentation
retain their original paths. The
[layout path map](../evidence/project-layout-20261109/path-map.json) relates former
fresh-source locations to the current tree. Relocation changes no language rule
or performance conclusion.

The milestone is a working language subset that demonstrates the memory
guarantees through checked construction, stable growth, and scoped release.

**Current acceptance, November 10:** the finite M1 checked core is accepted under
its named Darwin arm64 compiler/runtime profile. The [final decision](#m1-acceptance-decision--november-10)
records passed checks, trusted assumptions, unresolved measurements and deferred
features. This completes the agreed foundation before another comparison campaign;
it does not complete the language or establish a speed ranking.

**Naming diagnostics, November 12:** the first [D21](spec.md#identifier-naming-conventions)
increment checks supported declarations, with advisories by default and
`--strict-names` for enforcement. Its acceptance cases cover original
declaration spans, standalone and package scope, unchanged formatting, semantic
failure precedence, and rejection before native build work. The language's
identifier validity rules and native lowering stay under the existing checks.
The workspace package suite runs the strict policy on the shipped standard library.
[Validation and release identity](../evidence/naming-diagnostics-20261112/README.md) ·
[Standard-library gate](../evidence/stdlib-naming-gate-20261112/README.md)

**Current comparison, November 10:** the authorized control rebuild/verification
and fresh matched scorecard are complete. The [scorecard record](#fresh-matched-scorecard--november-10)
retains 1,680 measured observations and 108 preflights with real C, Rust and freshly
verified local Goose controls. That scorecard changes no optimizer or accepted
language semantics; the subsequently authorized optimization trial is below.

**Current optimization outcome, November 10:** the
[narrow construction follow-up](#narrow-construction-and-vector-follow-up--november-10)
accepts a private scalar-input branch-layout heuristic after separate full
lifecycle/copy/failure campaigns and final rebuilt validation. This completes
M2's first finite compiler improvement. The earlier
[broad branch-layout trial](#construction-success-branch-layout-trial--november-10)
remains rejected; the independently tested vector-prefix candidate is not adopted.

**Current workload follow-on, November 10:** the
[ingestion campaign](#in-memory-ingestion-follow-on--november-10) is complete:
360 measurements and 30 preflights compare actual parsed Holo, tuned C/Rust
arenas and the verified local modified Goose compiler. All four are close on
larger payloads; Goose has a clear short-message advantage in this fixture.
The accepted compiler remains unchanged. Shared-host distributions and scope
limits accompany every result; this does not establish a language ranking.

**Historical acceptance review, November 9:** M1 remained incomplete. The
[generated cleanup audit](#generated-cleanup-path-acceptance--november-9) now covers
explicit reset, append rollback and multi-owner/early-exit paths. The next
concrete concern was the generated helper contract boundary. The [November 10 audit](#generated-helper-contract-audit--november-10)
now supplies finite native evidence and a reviewable unresolved runtime/fatal-check
decision, followed by final gate disposition. The [current acceptance review](#m1-acceptance-review--november-9)
separates that work from demonstrated behavior, unresolved measurements and
deferred language features. It does not change the approved M1 rules or close
the remaining gate.

**User-directed sequencing, November 10:** finish the current native-contract
work and the M1 acceptance review before starting another language-comparison
campaign. First settle the concrete failure/runtime boundary below and implement
its approved checks; then disposition the composite gate and its disclosed limits.
Native correctness runs, isolated labeled policy experiments and evidence replay
support that work. The next competitive scorecard, performance optimization and
realistic follow-on workload remain after this gate. This sequencing instruction
did not itself approve failure semantics or a runtime trust policy. The subsequent
instruction to proceed with the next priorities approved the concrete M1 policy
package below; its implementation and verification are now the remaining work.

## 1. What the audit found

The initial audit assessed the retired prototype against the proposed
Goose-style memory foundation.

| Area | Retired prototype | New milestone needs |
|---|---|---|
| Headers | `func name(arguments) Result` | Preferred `with` / `arg` / `returns` syntax. |
| Storage | Heap-backed buffers, boxes, and variants | A specified compiler-managed scoped region. |
| Growth | Reallocation and owned-value relocation | Supported nonmoving extension. |
| Scope cleanup | Recursive aggregate cleanup and frees | Plain-data marker reset without per-object traversal. |
| References | Conservative whole-root exclusivity | A sound retained-element-reference / extension rule. |
| Results | Scalar/vector/void results | Eligible aggregate results built at their destination. |
| Compiler structure | Checking largely occurs during textual LLVM lowering | A small explicit checked representation of storage, lifetimes, and destinations. |
| Benchmarks | Primarily Holo versus C/CRoaring | Shared language-mechanism cases in C, Rust, Goose, and Holo. |

The first local capability probes used a release build of that prototype.

- A basic buffer program using the existing syntax was accepted.
- The preferred header was rejected with `expected (`.
- Growing a buffer while an element borrow remained live was rejected because
  its owner was inaccessible during the borrow.
- Returning a struct was rejected because aggregate results are unsupported.

These observations describe the prototype at the time of the audit.

## 2. Names that prevent misleading comparisons

Use these labels in source manifests and reports:

- **C:** a real C implementation, not a C++ result renamed as C.
- **Rust:** record the exact toolchain and representation tier.
- **Goose:** record the exact compiler, runtime, standard library, native backend,
  and dirty source state when applicable.
- **Holo-current:** the retired executable prototype in historical comparisons.
- **Holo-new:** the checked region subset, once implemented.

Label the original executable path **Holo-new checked IR core**. Its record
operations really execute generated LLVM; the Rust input builder is not the measured
algorithm. Label the source frontend **Holo-new experimental source**. It may
claim only its supported subset. Earlier IR timing artifacts remain IR artifacts;
they are not retroactively source measurements. An unsupported case is **unsupported**, not
a zero, a guessed timing, or a host-language stand-in.
Historical Roaring and Goose experiment results remain historical results for
their actual programs and representations.

## 3. First freeze a deliberately small memory model

These are the concrete rules to specify before lowering the new subset. They are
milestone proposals; the full language's storage and reference system remains open.
The user approved [M1-R1 through M1-R3](spec.md#m1-memory-rules) on November 9:
alias-permitting access and sequencing, private publication/suffix rollback, and
initial checked bounded backing. The full-language defaults outside this slice
remain open. Initial native evidence used programmatic construction IR; real
experimental source coverage now shares the same checked core.
[The source guide](../compiler/src/README.md) records implementation limits.

1. Unsigned integer fields, byte payloads, sequential records, and ordinary calls.
2. One well-defined growable construction destination per relevant region top.
   Define which already-live values can coexist with it.
3. Supported extension preserves existing addresses. Define exhaustion and target
   backing requirements; virtual reservation does not mean infinite capacity.
4. Existing-element references may survive a permitted extension. Prove why the
   extension cannot invalidate them. `readwrite` must not silently become an
   unchecked whole-container non-aliasing assertion.
5. Eligible producers construct their results in the final destination through
   nested calls. Define branch cases and distinguish explicit existing-value copies.
6. Eligible nested records have an inline representation with inspectable metadata.
7. Scope release of plain data resets a fixed set of markers, with no per-record
   traversal, frees, reference-count work, or runtime garbage-collection pass.
8. Reject escaping references, reset while a usable reference, captured callback,
   or in-flight operation can still access the region's existing contents, and
   unsupported interior resizing. Specify which old views/values become invalid
   after reset; the region owner must remain usable for reuse. These require
   actual checker rules.

A manually written arena demo can establish a representation's costs, but it
does not establish a safe language implementation. The implementation must make
the relevant invalid programs unrepresentable or reject them.

Use the preferred header for this subset. Leave compressed layouts, elaborate
schedules, general shared mutation, dynamic callbacks, accelerator support, and
new ownership-transfer syntax for later milestones. Do not require floating-point
semantics merely to demonstrate region construction and reclamation.

## 4. The first new-language program

The first executable slice should:

1. Construct records with unsigned fields and variable-sized byte payloads.
2. Have a producer write a record directly into its eventual batch destination.
3. Scan the batch to an independently calculated checksum.
4. Exit the scope and make its backing storage reusable by marker reset.
5. Repeat with the same backing storage to demonstrate reuse and bounded memory.

Add the retained-element-reference / growth case as an acceptance gate before
calling the subset Goose-compatible. Include nested calls and early failure so
the placement and cleanup rules do not work only for a straight-line demonstration.

The physical record format, input generator, and checksum must be specified in
the fixture. C and Rust should include tuned flat/arena controls using that format,
as well as straightforward representations where they answer a different question.

## 5. Measurement cases, in order

| Case | Purpose | Holo-current status at audit |
|---|---|---|
| Flat scalar-array transform/scan | Validate the harness and compare ordinary native-loop generation. | Supported in principle with existing buffers and timing built-ins. |
| Retained element reference during growth | Test stable growth plus reference usability. | Whole-root borrow restrictions conflict with this pattern. |
| Build → scan → release nested records | Test placement, flatness, copies, and reclamation together. | Existing heap-backed aggregates are a different representation; aggregate-return placement is absent. |
| Repeated scoped construction/reset | Test release cost and backing reuse as object count grows. | Current cleanup walks owned structures; it is a control, not the proposed reset implementation. |
| Relative-link tree/index | Evaluate compact links, working sets, and address arithmetic. | A follow-on case after the first region subset. |

The scalar control should run first. Similar or different times there cannot
establish success or failure of the new memory model: the control intentionally
does not exercise its distinctive mechanisms.

## 6. Fairness and correctness rules

### Matched work

- Fix the logical algorithm, inputs, distributions, numeric behavior, checksum,
  work units, and phase boundaries.
- Record representation differences and borrowing/ownership requirements.
- Use both straightforward and tuned flat/arena tiers when relevant. A tuned C
  arena is a necessary strong baseline for a region-language claim.
- Keep memory-safe checked language behavior enabled. If C uses unchecked accesses,
  label that fact rather than silently treating safety obligations as identical.
- Match C and Goose native compiler/target settings where supported, and record
  Rust's backend version. Different backend versions remain a comparison limit.

### Native measurement

- Time native execution, with compilation, input generation, printing, and oracle
  checks outside the timed kernel unless a lifecycle case explicitly includes them.
- Prevent dead-code elimination and unintended repetition hoisting without adding
  a different barrier to every element in one language.
- Warm up, run measured processes sequentially, vary language order, preserve raw
  samples, and report a distribution rather than the best observation.
- Calibrate sufficient work. Tiny pointer resets need generated-code inspection
  and repeated region reuse; one isolated timer interval is not convincing.
- Preserve total lifecycle timing as well as phase timings. Do not force phase
  boundaries to materialize copies the intended design would otherwise avoid.

### Memory and toolchain identity

- Track allocations, copies, representation size, reserved address space, committed
  backing, and resident memory where each can be measured correctly.
- Separate logical reset from OS decommit and external-resource cleanup.
- Record source/binary hashes, compiler versions, flags, commands, runtime/stdlib
  identities, host architecture, CPU information, and workload parameters.
- A dirty compiler is allowed as an experiment only when its actual inputs are
  identified. A commit ID alone does not identify modified source bytes.

### Acceptance checks

- Independent expected outputs; checksums must agree before interpreting timings.
- A small replayable model-based region harness, with generated defined programs
  and operation sequences, internal invariants, and independent expected results
  checked against unoptimized and optimized native execution.
- Preserved seeds, source/traces, compiler/toolchain identities, flags, and expected/
  actual outcomes; reduce discovered failures into regressions. Audit generators
  for excluded cases and do not reuse checker/lowering logic as the sole oracle.
- Compile-fail cases for reference escape, reset while referenced, and disallowed
  growth. Also require valid stable-growth examples to compile.
- Allocation/capacity failure and partial construction with correct cleanup.
- Layout inspection and generated-code evidence for destination construction,
  retained addresses, and absence of per-object reclamation.
- Multiple working-set sizes and object counts; instrument causal claims separately
  from headline timing runs.

Follow the [staged testing architecture](compiler.md#7-testing-architecture-and-staged-adoption).
Expected source rejection and specified runtime traps must be distinguished from
compiler crashes or miscompilation. M1 does not require a full distributed
simulator or Antithesis setup. Introduce event simulation with future concurrent/
I/O components, and evaluate external testing platforms after an executable slice
exists. Testing hooks must preserve normal safety enforcement and keep their
release costs explicit; simulated timing cannot establish hardware performance.

## 7. Deliverables and stopping points

### M0 — establish trustworthy controls

- [x] Audit current implementation and available benchmark infrastructure.
- [x] Define labels and unavailable/unsupported reporting rules.
- [x] Run a checked C/Rust/Goose/Holo-current scalar control with archived inputs,
  native timings, metadata, and independently verified results.
- [x] Confirm the harness does not confound process startup with kernel time or
  run measured languages concurrently.

The historical scalar runner used explicit generic CPU targeting and native
optimization flags. Its first non-smoke control validated 36 native
executions, with nine samples in each language,
1,048,576 values and 30 passes. The medians cluster near 9.6 ms, with substantial
sample variation. This verifies the control workflow and does not rank languages
or establish new-memory-model performance.

The 32 KiB payload control added 36 validated executions using 4,096 values
and 1,024 passes. The 72 samples were local controls without affinity or
frequency isolation or memory accounting. The retired runner was removed
with the prototype. Use the [maintained benchmark guide](../benchmarks/README.md)
for current comparisons.

### M1 — implement the checked region subset

- [x] Review and approve the [region, reference, destination, and failure
  rules](spec.md#m1-memory-rules), including valid and invalid examples.
- [x] Introduce a small experimental checked IR-to-LLVM path.
- [x] Implement the small replayable region IR harness with an independent model,
  valid/invalid IR, failure injection, root/initialization checks, and native O0/O3 execution.
- [x] Execute the first nested-record program with the preferred header syntax
  through the explicitly experimental source adapter, at O0/O3 and under ASan.
- [x] Pass retained-reference/growth and lifetime-rejection cases in checked IR.
- [x] Inspect construction/reset paths and run the first matched four-language
  lifecycle fixtures with wide/compact C/Rust arena controls and process CPU/RSS.
- [x] Validate populated one-marker lexical release/reuse and inspect O0/O2
  native release paths across count, payload, and inline-depth shapes; retain
  unresolved physical timing separately from the fixed-work evidence below.
- [x] Observe backing mapping/residency across logical reset and final allocator
  release for the selected unchanged O2 controls, with calibrated own-task/page
  queries and explicit unavailable commitment/process-death metrics.
- [x] Account for fresh-construction payload writes across success/partial failure,
  retry and reuse using destination provenance, native semantic witnesses and
  calibrated post-optimization LLVM access traces; retain their harness limits.
- [x] Account for complete readonly byte-field input copies across nested calls,
  aliased inputs, same-batch prefix input, partial failure, retry and reset/reuse;
  independently explain source, final destination and all nonpayload effects.
- [x] Pass root-specific reset/reuse and multi-owner cleanup continuation cases,
  including actual parsed source, independent expected/model results, allocator
  failure, and native O0/O3/ASan under all three layouts.
- [x] Accept the implemented bounded-backing policy across many-cycle refill,
  finite exhaustion/rollback and first/middle/last acquisition failures; observe
  live backing during reset/reuse and a separately bounded competing allocation.
  This does not select VM backing or establish genuine OS-pressure behavior.
- [x] Let source acquisition recovery finish its owner block and resume outside
  it, preserving failed-owner isolation, explicit entry returns, retry exhaustion
  and exact cleanup at O0/O3/ASan through the existing checked Region operation.
- [x] Preserve and audit original/optimized/native cleanup for explicit reset,
  append rollback, lexical release, multi-owner completion and acquisition
  recovery, with count/payload/inline-depth and separate owner-count anchors.
  Validate ordinary/observed execution and independent model outputs; distinguish
  native checkpoint/spill work, eliminated stores and backing frees from reset.
- [x] Complete remaining causal memory/reclamation evidence: physical reset
  timing where meaningful, broader copy/failure cost accounting and
  backing/commitment policies where measurable. The finite owner correctness
  matrix below does not measure cleanup cost or establish every Goose mechanism
  or M1. The [acceptance review](#m1-acceptance-review--november-9) identifies the
  completed finite cleanup audit and its then-remaining helper concern. The
  [November 10 final disposition](#m1-acceptance-decision--november-10) accepts the
  reviewed native profile and records physical reset latency as unresolved and
  generic commitment as unavailable, not measured numerical passes.

The checked-core implementation is in [compiler](../compiler/README.md), with a runnable
[region example](../compiler/examples/region.rs), [checker cases](../tests/compiler/check/check.rs),
[native/ASan checks](../tests/runtime/native.rs), and an [independent model](../tests/support/model/README.md).
The [lifecycle microbenchmark](../compiler/examples/measure_region.rs) measures real native
construction, linear cursor scan, and marker reuse; it is not yet a matched
C/Rust/Goose campaign or an isolated release-cost test. Backing setup/free and
validation/output are outside its clock interval; one opaque barrier per cycle
is included. Retain all raw samples and report its checking/layout costs.

The **finite M1 gate is accepted under the reviewed native profile**. The [source example](../tests/fixtures/region.holo),
[source rejections](../tests/compiler/frontend/frontend.rs), and [source/native checks](../tests/compiler/frontend/frontend_native.rs)
now exercise preferred headers, scalar helpers, nested destination returns,
retained aliases, byte mutation, captured-prefix scans, reset/reuse, failed
capacity/overflow/rejection rollback, and cleanup-aware early entry completion.
Native results are compared with independent expected values and the existing
model at O0/O3/ASan. CLI checks cover LLVM-independent validation, diagnostics,
native build/run, emitted IR, strict flags, and refusal to overwrite outputs.

The source adapter has experimental core API names/statuses and a
deliberately limited [supported boundary](spec.md#experimental-source-subset).
It supports scalar and complete readonly byte-field constructor arguments,
explicit backing assignments, packed wide/derived/compact metadata, and
conservative same-root reference rebinding. Derived64 is now the source default;
compact lengths remain explicit. Source contracts are checked only on closed pure
scalar/producer forms; there is no general source effect checker, and generated
helper effects remain an [acceptance concern](#generated-helper-and-contract-boundary).
The original frontend increment included optional observed phase timings, not a
measured compiler-speed or runtime-performance improvement; subsequent layout and
input campaigns below retain their separate evidence.

The [preserved source acceptance run](../evidence/source-frontend-20261109/README.md)
contains the maintained source example, emitted LLVM, native O0/O3 executables,
exact outputs/commands, tool identities, compiler-source snapshots, and hashes.
Both binaries print the independently calculated `98`, `0`, `0`; the experimental
specification example also passes check-only. These are functional observations,
not a timed comparison. Source evaluation tests separately instrument the clock
boundary to check argument order and bounds-failure precedence deterministically.

The first local lifecycle artifacts are:

- [512 records × 1,000 reuse cycles](../evidence/region-core-small-20261109/report.md):
  nine validated samples, with a declining timing trend. See the
  [post-run review](../evidence/region-core-small-20261109/review.md); its median is not
  a stabilized throughput estimate.
- [65,536 records × 16 reuse cycles](../evidence/region-core-large-20261109/report.md):
  nine validated samples, with [optimized assembly and review](../evidence/region-core-large-20261109/review.md).

Both retain inputs, compiler sources, tool identities/commands, binaries, hashes,
and raw observations. First-touch/page-fault effects can remain inside construction
despite backing acquisition being outside the timer. These local mechanism runs
do not establish source-language performance, isolated free cost, compactness,
resident-memory efficiency, or a win against another language.

The first matched source campaign is
[lifecycle-comparison-20261109](../evidence/lifecycle-comparison-20261109/report.md),
with [all three case tables](../evidence/lifecycle-comparison-20261109/results-reviewed.png),
[largest-case graphic](../evidence/lifecycle-comparison-20261109/results-largest-reviewed.png),
and [raw samples](../evidence/lifecycle-comparison-20261109/raw.jsonl).
The maintained [suite](../benchmarks/region-lifecycle/README.md) defines the exact
work, timing phases, representation tiers, safety/failure differences, resources,
sources/tool identities, and reproduction commands. Eighteen binaries passed
preflight; all 270 measured observations passed independent checksum, status, and
representation checks. Each process performs a full untimed warmup using its own
backing; 15 measured processes per implementation/case run sequentially with varied
order. Native intervals cover 33,554,432 records per process, across 512, 65,536,
and 1,048,576 records per reusable batch.

Largest-case median aggregate costs were Holo **8.55 ns/record**, matched wide C
**8.27**, matched wide Rust **8.45**, Goose **7.88**, compact C **8.34**, and compact
Rust **8.23**. These are complete build/scan/reset costs divided by record count,
not individual-operation latency. Holo's matched-Rust difference is small relative
to observed variation. Goose leads medians in these cases; this is not a language
ranking or proof of causal attribution to one mechanism. Common native mode is
O2/generic, not Holo CLI release O3. Rust uses LLVM 21.1.3; the other native
backends use LLVM 22.1.2, a disclosed comparison limit.

The [stability review](../evidence/lifecycle-comparison-20261109/generated-review.md)
records increasing early-to-late medians: Holo's last-five versus first-five
samples rose 7.36% for 512 records, 4.45% for 65,536, and 0.38% for the largest
case. Several other controls also drift. The varied order and complete ranges
help disclose this; the campaign is not a stabilized peak-performance estimate.
No samples are removed, including descriptive outliers.

At 1,048,576 records, observed inline record images were **47.5 MiB** for
Holo/wide controls and **27.5 MiB** for Goose/compact controls; payload alone is
15.5 MiB. Holo peak process RSS max was **48.88 MiB**, Goose **28.94 MiB**. Fixed
descriptors/backing slack are outside image bytes but included where resident in
process RSS. Goose's 2 GiB stack reservation setting is not RSS/commit; its source
marker telemetry is unavailable and recorded as -1. OS commit/reservation totals,
post-reset RSS, isolated free cost, and object allocation/copy counts remain
unmeasured. Generated code shows marker restoration without timed per-object
free/copy calls, while Holo's component capacity/overflow guards remain visible.

Failed SDK/link and sub-clock-resolution probes remain under generated
`evidence/lifecycle-smoke-20261109`, `evidence/region-controls-probe-*`, and the Goose
copy-placement experiment in `evidence/goose-lifecycle-probe`. They are exploratory
validation artifacts, not samples selected out of the reported distributions.

#### Header-derived and narrow-length implementation

The user authorized the staged representation change on November 9: first derive
eligible record sizes while keeping u64 byte lengths, then evaluate explicit
checked u32 lengths, and validate/rerun comparisons before selecting defaults.
[Physical layout rules](spec.md#experimental-physical-layouts) describe the
conservative schema policy, retained complex headers, empty-unit progress,
full-width alternative, overflow, and per-format capacity outcomes.

The [planner](../compiler/src/layout.rs) and shared backend now implement all three modes.
`inspect-layout` reports reasons/offsets without LLVM. The 130-test full all-target
suite and one additional focused native overflow regression pass, bringing current
coverage to 131 compiler tests, including [plan checks](../tests/compiler/storage/layout_plan.rs),
[source/native layout conformance](../tests/compiler/storage/layout_native.rs), all-layout generated
model comparisons, narrow width/capacity/overflow rejection, retained aliases,
mixed/non-tail variable fields, empty records, captured-prefix append, reset,
rollback, bounds failures, and cleanup-aware early exit. O0/O3/ASan are used where
applicable. The independent model derives physical sizes separately from the
compiler planner using mathematical integers and its own schema facts.

The first [layout campaign](../evidence/layout-comparison-20261109/report.md) preserves
495 validated measured observations and 33 preflights, including the hash-verified
actual prior Holo binary, current wide/derived/compact Holo, matched 32/16/12-byte
C/Rust controls, and the intended local Goose checkout. Largest record images
were **47.5 / 31.5 / 27.5 MiB**, confirming 16/20 MiB savings for the two new modes.
However, the first derived/compact lowering was **about 3–6% slower** than wide
in paired local results. [Generated-code/stability review](../evidence/layout-comparison-20261109/generated-review.md)
records the unfavorable result and redundant status/control-flow sequences.
At that point both new modes remained opt-in: the result did not justify adopting
a slower format as the default. All inputs, binaries, distributions, and samples
remain preserved; the corrected lowering was compared separately below.

The subsequent [ordered-reservation comparison](../evidence/layout-reservation-comparison-20261109/report.md)
contains **585 validated measured observations and 39 preflights**, with actual
preserved wide/derived/compact binaries as same-campaign before controls. The
general lowering change emits each existing overflow and capacity edge directly
at the original reservation site, avoiding a `{pointer,status}` result followed
by a redundant success test. It removes no safety check, fuses no reservations,
and preserves field order, failure status priority, marker publication, and
append rollback. A focused boundary case verifies that top-plus-extent overflow
returns status 2 before capacity failure even when the extent alone fits u64.
The [focused graphic](../evidence/layout-reservation-comparison-20261109/results-focus.png)
shows the largest-case before/after and matched-format controls;
[all13rows/threecases](../evidence/layout-reservation-comparison-20261109/results.png)
retain the complete comparison. No unfavorable sample or prior layout campaign
is replaced. Post-adoption default emission is byte-identical LLVM to the measured
explicit-derived program; a separate O3 run validates default execution without
mixing that observation into the O2 comparison.

[Generated-code and paired review](../evidence/layout-reservation-comparison-20261109/generated-review.md)
confirms the observed redundant status branches disappear while native checks
remain. Derived64 paired aggregate cost is about **4.4–4.8% lower than concurrent
wide** in the three tested sizes; compact also improves. Largest point medians
are wide **8.75**, derived **8.46**, compact **8.41 ns/record**. C/Rust controls
using matched formats remain close/overlapping; local Goose leads at **8.07**.
Differences between derived and compact are small/noisy, and these are not
whole-language rankings or peak O3 release measurements.

Observed largest images are **47.5 / 31.5 / 27.5 MiB**, with peak process RSS
maxima **48.88 / 32.88 / 28.88 MiB**. Derived saves 16 MiB/about33.7% while preserving
u64 lengths; compact saves20 MiB/about42.1% and explicitly bounds each payload
length to u32. Following the approved staged checks and comparison, **Derived64
is now the experimental source CLI default**. Compact stays explicit; library
`llvm::emit` and CLI `--layout wide` preserve the wide control. This is an
experimental compiler policy, not a universal ABI or every-schema speed guarantee.
This campaign alone did not complete M1's memory/reclamation accounting. Remaining
growth guards are disclosed costs under approved M1-R3, rather than a requirement
to adopt check-free growth before M1. Later diagnostics and the
[current acceptance review](#m1-acceptance-review--november-9) identify the present
evidence gaps; wider architecture performance remains unestablished.

### Construction/scan phase diagnostics — November 9

The [phase suite](../benchmarks/region-lifecycle/README.md#diagnostic-phase-campaign)
now adds separately instrumented binaries while preserving the original combined
controls. Both variants share real Holo/C/Rust/Goose construction and scan source;
no compiler rule, layout default, or runtime algorithm changed. For the largest
case, all three combined Holo LLVM files are byte-identical to their previous
reservation-campaign controls. There is no Git metadata here, so no branch/main
baseline is claimed.

The [completed campaign](../evidence/lifecycle-phases-20261109/report.md) retains
**900 validated measured observations plus60 preflights**: ten combined and ten
instrumented rows, three batch sizes, fifteen samples each. The strict output and
summary harness now has26 passing tests. Auxiliary probes cover C/Holo O0/O2/ASan,
Rust O0/O2, and actual Goose O2. Six tiny phase probes correctly reject zero-duration
calibration rather than entering the timing distribution; above-floor probes and
all main-campaign preflights/samples pass. Raw streams, order, inputs, compiler/
runtime identities, binaries, generated LLVM/C/assembly, and commands are preserved.

Largest-case point medians, in ns/record:

| Implementation | Original combined | Instrumented build + opaque | Instrumented scan |
|---|---:|---:|---:|
| Holo default, full-width derived16B |8.688|3.151|5.479|
| Holo explicit compact12B |8.666|3.169|5.484|
| Local Goose12B |8.312|2.801|5.477|

These observations point to construction as the next place to investigate the
remaining gap. They do not identify reservation checks as its cause: generated
code, differing backing/failure rules, and a separately controlled optimization
experiment are needed for that claim. Scan distributions are close; phase clocks
also change optimizer freedom, registers, and CPU/cache state. Phase medians must
not be added to predict the original combined interval. Full matched32/16/12B
C/Rust controls, IQR/ranges, resources, and instrumented/combined paired ratios
remain in the report. The [focused graphic](../evidence/lifecycle-phases-20261109/results-focus.png)
shows the largest eligible-layout comparison; the
[complete graphic](../evidence/lifecycle-phases-20261109/results.png) retains all cases.
An [independent audit](../evidence/lifecycle-phases-20261109/independent-phase-analysis.md)
rechecks every raw stream, oracle witness, interval sum, block, and published
distribution with zero failures. Derived/compact construction is slower than
Goose in all15 paired blocks in each case; scan pairs reverse signs and overlap.
The audit retains observed drift, all outliers, and ratios above/below1 between
instrumented and combined variants rather than calling their difference pure
clock overhead.

**The isolated reset-scaling gate remains open.** Gross scope-exit readings overlap
empty-clock gaps and include zero observations. [Holo/Goose code review](../evidence/lifecycle-phases-20261109/generated-review.md)
and [C/Rust review](../evidence/lifecycle-phases-20261109/controls-codegen-review.md)
show private Holo/C/Goose marker work moved/folded into next-cycle setup, whereas
Rust retains a fixed paired marker store between scope clocks. No record cleanup
walk or successful per-record allocator/free/payload-copy call appears in those
reviewed native loops. This is generated-code evidence, not dynamic allocation/
copied-byte telemetry or a measured isolated release cost. No clock median is
subtracted and no reset-speed ranking is inferred.

M1 remains incomplete: isolated release scaling, dynamic allocation/copy accounting,
OS reserved/committed memory and post-reset resident backing, broader shapes and
architectures remain outstanding. The original unfavorable layout campaign and
all later controls remain intact.

### Remaining-capacity guard experiment — November 9

The construction diagnosis led to a controlled change in the shared
`storage::reserve` helper. Under the existing invariant `0 <= top <= capacity`,
`bytes <= capacity - top` proves both capacity and nonoverflow. A failed fit can
classify the original top-plus-bytes overflow as status2 or ordinary shortage as
status1. The experiment kept every reservation at its original field boundary,
preserved length/width guards, rejection precedence, private initialization,
publication, rollback and reset, and added no LLVM alias/arithmetic promises.

The candidate passed135 compiler/example tests, strict all-target Clippy, formatting,
and29 harness tests. This included18 focused O0/O3/ASan executions across all layouts,
existing source/model/native cases, and separately labeled host algebra checks over
8,421,376 valid u8 states/requests plus ten u64 boundaries. The two meaningful native
fixtures remain in [reservation regressions](../tests/compiler/storage/reservation.rs). Candidate-only
arithmetic tests are archived with that experiment rather than left as unused
production checks after rejecting its algorithm.

The [comparison](../evidence/remaining-capacity-comparison-20261109/report.md) contains
**1,170 validated observations and78 preflights**, with original combined and
instrumented binaries for all three Holo formats, matched C/Rust/Goose controls,
and18 hash-verified actual prior Holo executables. No branch/main baseline is
invented. [Independent audit](../evidence/remaining-capacity-comparison-20261109/independent-reservation-analysis.md)
rechecks all1,248 output streams, independent oracles, image/status/marker witnesses,
interval conservation, distributions,78 binary hashes and preserved provenance.
No sample or outlier was removed.

Paired candidate/prior **combined cost changes for the default Derived64 layout**:

| Records/batch | Median cost change | Paired middle50% | Faster / slower blocks |
|---:|---:|---:|---:|
|512|+1.76%|+0.66% to +2.59%|2 /13|
|65,536|−1.06%|−1.98% to +1.23%|9 /6|
|1,048,576|−0.15%|−3.95% to +1.51%|8 /7|

Positive means slower. Larger default cases are inconclusive despite an apparently
better ratio of independent point medians in the largest case. Compact's medium/
large paired medians improve about1.5–1.6%, but that narrower result does not justify
a second format-specific reservation implementation. Host drift is substantial;
all ranges and descriptive paired signs remain visible. The
[decision graphic](../evidence/remaining-capacity-comparison-20261109/results-decision.png)
shows every layout and size. [Whole-run quantile comparisons](../evidence/remaining-capacity-comparison-20261109/whole-run-quantiles.md)
retain means, throughput and p50/p75/p90/p95/p99/p99.9/min/max deltas. Those interpolate
fifteen whole-run samples and do not establish per-operation or rare-tail latency.

[Native review](../evidence/remaining-capacity-comparison-20261109/generated-review.md)
confirms successful constructor guard decisions fall5→4 in Wide and3→2 in
Derived/Compact. Code size, register allocation and some scan setup also change;
fewer branches alone do not establish a speedup. No new allocator/copy/drop/free
path or source semantic change was introduced. Scope/reset timing remains
unresolved against clocks and optimized marker motion.

**Decision: not adopted.** The original single reservation helper is restored
byte for byte; no experimental switch or alternate layout-specific path remains.
The active compiler/example suite has133 tests after retaining two new native
regression fixtures and archiving the two candidate-only algebra tests. Its
formatting, strict all-target Clippy, full default/example tests and29 harness
tests pass after restoration. The release executable is rebuilt from restored
source. Candidate source, proof, binaries, all samples and adverse results remain
in the generated archive. This closes the experiment, not M1's outstanding memory/
reclamation gates. Byte-initializer SIMD width is a separate possible experiment,
requiring its own correctness and timing evidence before any adoption.

### Byte-initializer width experiment — November 9

A second, separately controlled trial evaluated a private low-byte expression
context. For the checked producer's closed Const/Param/Index and wrapping
Add/Sub/Mul/Xor/And trees, projection modulo256 commutes with every operation.
Plain i8 expression nodes therefore preserve the final stored byte of the original
u64 expression. Only byte values were changed; scalar fields, argument ABI,
lengths, physical counters/addresses, guards, failure order and ownership remained
full-width and unchanged. There were no new LLVM promises or source-language rules.

The candidate passed135 compiler/example tests, formatting, strict all-target
Clippy, and41 harness checks before measurement. Two new
[byte-initialization fixtures](../tests/compiler/storage/byte_initialization.rs) compare the
independent full-u64 model and hand outputs with actual IR/source O0/O3/ASan
execution across all layouts. They cover all five operations, nested u64 wrapping,
high scalar/argument bits, lengths0..511 including physical indices beyond255,
failed reservation, retained aliases, rollback and reset reuse. These semantic
regressions remain active after rejecting the optimization.

Fresh C/Rust/Goose controls were deliberately strengthened with equivalent explicit
u8 byte arithmetic. All seven O2 controls pass independent output probes, retain
their original guards/placement, and emit the same short byte-SIMD loop. This
prevents crediting Holo with a unique hardware capability that other languages can
express. Those fresh control sources changed; their differences from archived host
results are not language/compiler improvements. The primary causal comparison is
unchanged Holo source, flags and work against actual preserved Holo binaries.

The [campaign](../evidence/byte-initializer-comparison-20261109/report.md) retains
**1,248 measured observations and78 preflights**. A new adjacent-pair schedule runs
candidate/prior Holo consecutively, with exactly8 candidate-first and8 prior-first
blocks per case/protocol. All39 semantic groups' coverage, adjacency and order
balance are audited. [Independent analysis](../evidence/byte-initializer-comparison-20261109/independent-byte-analysis.md)
checks all1,326 raw streams, oracle/image/status/marker witnesses, distributions,
78 binary hashes and18 actual prior control identities, with zero discrepancies
and no exclusions. All layouts/sizes, order strata, ranges, drift and outliers
remain visible. [Whole-run quantiles](../evidence/byte-initializer-comparison-20261109/whole-run-quantiles.md)
include means, throughput and requested percentiles; sixteen whole-run samples
still do not establish operation-tail latency.

Default Derived64 paired candidate/prior cost changes:

| Records/batch | Combined median [middle50%] | Instrumented build+opaque median [middle50%] | Slower combined / build blocks |
|---:|---:|---:|---:|
|512|+0.61% [−1.63%, +1.62%]|+4.55% [+1.48%, +7.83%]|10 /14 of16|
|65,536|+1.67% [+0.43%, +2.71%]|+0.95% [−0.21%, +3.61%]|13 /12 of16|
|1,048,576|+0.81% [−0.07%, +2.78%]|+4.62% [+1.97%, +6.71%]|12 /14 of16|

Positive means slower. The medium combined regression and small/large construction
regressions persist in both execution orders. All nine build medians are adverse;
no combined comparison has an IQR wholly on the improvement side. The
[decision graphic](../evidence/byte-initializer-comparison-20261109/results-decision-final.png)
shows all nine paired layout/size comparisons, without replacing combined timing
with the separately instrumented phases.

[Native review](../evidence/byte-initializer-comparison-20261109/generated-review.md)
confirms vector initializer bodies shrink13→5 instructions while still storing
eight bytes per vector iteration. Guards, scan-body instructions and call counts
remain unchanged; this is not a wider-store or check-removal result. The scalar
remainder instead grows4→5 instructions per byte and gains a vector-exit transfer.
With lengths0..31,28/32 records need a remainder. Those observed changes nominate
short-loop/tail/transition paths and dependencies for profiling; they do not prove
the regression's cause. Entry-frame shrinkage occurs outside native timing.

**Decision: not adopted.** The original expression and producer files and rebuilt
release executable are restored byte for byte. No alternate byte path or experiment
flag remains. The restored135 compiler/example tests, strict all-target Clippy,
formatting, and42 harness tests pass. The extra harness check prevents fresh Holo
metadata from falsely claiming projection: it now records compiler-selected,
while archived candidate metadata remains unchanged. Strong byte-tuned host
controls, adjacent balanced scheduling, two native fixtures and all failed-trial
evidence are retained. M1's dynamic memory/reclamation gates remain open.

### Retained-binary CPU attribution — November 9

The [profiling investigation](../evidence/construction-profile-20261109/README.md)
uses the actual retained production and rejected byte-width combined executables,
without rebuilding or changing their work. The medium case has 65,536 records and
512 cycles. Thirty-six sequential Xcode Time Profiler launches provide six captures
per role/layout, with adjacent pairs, three pairs in each execution order, and
rotating layout positions. This is attribution evidence; the previous unprofiled
campaign still determines the candidate's rejection.

The [independent audit](../evidence/construction-profile-20261109/independent-profile-analysis.md)
passes all 36 native exits/output oracles, six binary SHA/UUID identities, 36 map
digests, linked instruction bytes, raw-PC/time joins and weight conservation.
There are 10,723 accepted observations: 10,654 target-image PCs, 46 external-image
PCs, 14 missing raw PCs and nine known PCs without image identity. Seventy-two
blocked stackshots remain explicit exclusions. All observed core labels are
P Core. Nominal tool weights are preserved, not interpreted as hardware cycles.

Within mapped construction, scalar-body/test plus vector-exit/scalar-setup shares
are:

| Layout | Production pooled share | Rejected candidate pooled share | Paired median change | Candidate higher pairs |
|---|---:|---:|---:|---:|
| Wide |21.61%|37.68%|+17.08 percentage points|6 of 6|
| Derived64, default |24.32%|38.36%|+12.88 percentage points|6 of 6|
| Compact32, explicit |27.96%|40.47%|+11.62 percentage points|6 of 6|

The vector-body share falls in all 18 pairs. These paired changes describe this
capture set; the [graphic](../evidence/construction-profile-20261109/results-profile-final.png)
also preserves individual captures, empirical spread and all-running denominators.
Candidate code merges vector-completion testing into its scalar/test block, while
production places some equivalent work in the separate transition block. Their
union is therefore the useful primary comparison. A zero sample count at separate
transition PCs does not establish zero execution or cost. A higher fraction or
observed count does not establish isolated scalar latency or explain the slowdown.

The [complete-CFG review](../evidence/construction-profile-20261109/cfg-and-model-review.md)
also finds fewer selected successful-path instructions overall in the candidate,
despite more branches and different scalar recurrences. Static path counts,
sampled locations and scheduling-model assumptions remain distinct evidence.
Periodic sampling, nearby-PC attribution, setup/warmup and profiler perturbation
limit causal conclusions. Only this medium case was profiled; no profiled native
elapsed comparison or population confidence interval is claimed.

Two legitimate missing-symbolication formats stopped earlier decoders. Their
failed reports, exports and source snapshots remain immutable. Decoder-v3 copies
the original traces, exports every table before normalization, and conservatively
retains absent PC/image data as unknown. Recovery reruns no native workload.
The profiling harness passes 88 host-only tests. Production's 40 source files,
release executable and 351 preserved legacy files still match their recorded
identities. No compiler optimization, numeric rule or storage guarantee changed.

This evidence nominates short scalar/test/transition and control paths for further
measurement. Check hardware-event sampling availability on the existing default
binaries before choosing another rewrite; ordinary PC samples cannot distinguish
branch/frontend effects from address/value dependencies. Any future byte-SIMD
candidate still needs its own semantic and unprofiled timing acceptance; no new
lowering was chosen or adopted here.
Dynamic allocation/copy accounting, isolated reset scaling and post-reset backing/
resident-memory evidence remain M1 gates.

### Hardware-event capability — November 9

The [capability investigation](../evidence/hardware-counters-20261109/README.md)
prepares the exact retained/candidate Derived64 medium executables and runs one
bounded production CPU Profiler probe. Its target path, native exit, checksum,
failure/marker witnesses and logical image validate. The confirmed host is Apple
M3 Max; installed Xcode is 16.4 (16F6), while xctrace identifies itself as 16.0
(16F6). Raw tool/host metadata, native output and exports are preserved separately
from the sealed earlier profile. The candidate was prepared but not executed;
this is not a before/after campaign.

The actual TOC requests `CORE_ACTIVE_CYCLE` PMI sampling at a 1,000,000-cycle
threshold for the selected target. Its populated `cpu-profile` export explicitly
uses `cycle-weight`, rather than the timer profile's nanosecond weights. An
independent audit joins all 1,075 weighted rows to their raw PMI counterparts,
retaining 1,072 target-image, two external and one unknown-image observations.
These are statistical whole-process observations, including startup/warmup. Raw
`pmi-count` readings are not assumed to be additive interval weights, and no
IPC, exact total-cycle comparison or isolated instruction cost is inferred.

Requested branch, misprediction, stall and retired-instruction events were not
collected. The stock CPU Counters template has no PMC events selected, and the
installed CLI exposes no documented event-selection flag. The documented route
is the Instruments event picker followed by saving a new custom template.
Instruments app control timed out; the subsequent Finder control request returned
“Computer Use was not approved to use Finder.” No custom template, private event
selectors, serialized-setting edits or security changes were used to bypass that
limit. The result is **configuration-blocked**, not proof of unavailable hardware
events or zero branch/stall counts. Processor Trace was not attempted because
Apple's recording requirements specify M4 or later; primary references and exact
installed documentation are archived in the investigation.

Compiler source/release identities and the previous sealed evidence remain
unchanged. No lowering or language rule changed. Another compiler rewrite remains
unjustified by this capability probe. Continue the independent M1 allocation/copy,
retained-backing and reset-scaling gates; revisit event comparison only with an
explicit configured template whose event units, scope and coverage can be checked.

### Allocation and transfer API diagnostics — November 9

The [API diagnostic campaign](../evidence/memory-telemetry-20261109/README.md) adds
**60 validated observations** of 30 unchanged native controls: production Holo,
C/Rust arena controls at 32/16/12-byte prefixes, and compact Goose, across all
three retained sizes, twice each. Only the separate observer and calibration are
compiled. Rejected byte-width Holo binaries are excluded, and source/native/tool
identities remain pinned. The host controls retain their explicit byte-tuned
source and original Rust backend; no archived Python or foreign workload kernel
implements a language's work.

A finite Darwin interposer observes 14 allocator, public VM and transfer hook
slots. A fixed shared snapshot separates outer requests from nested API
delegation and survives abrupt exit without a destructor. Requested bytes are
argument extents, including failed requests, rather than net/live allocation or
unique copied bytes. Runtime alias metadata and transfer-family totals preserve
Darwin's aliased memcpy/memmove implementation without inventing source spelling.
Overflow, missing/inconsistent boundaries, concurrent admission and observer
reentry reject evidence rather than silently producing zero counts.

All 60 workload windows contain **zero covered API entries in both layers**.
These windows lie between the two existing native clocks; pre includes
constructor-ready setup and warmup, while post includes validation/output and
explicit cleanup. Independent audits reconcile all 5,040 cells / 40,320 counter
values, all native oracles and binary identities, and all 120 raw clock-return
PCs against actual linked calls to `clock_gettime`. Both observations for every
control have identical cells. This is API-boundary evidence, not a timing result.

Largest-case pre-interval outer requests, with default Holo and matching 16-byte
C/Rust controls (Goose uses a 12-byte prefix):

| Control | Heap API calls / requested bytes | Public mmap calls / requested bytes |
|---|---:|---:|
| Holo Derived64 |1 /33,554,432|0 /0|
| C derived arena |1 /33,554,432|0 /0|
| Rust derived arena |5 /33,555,044|2 /163,840|
| Goose compact |4 /131,256|1 /2,148,532,224|

These are observed requests, not resident/committed memory or total allocator
backing: private allocator VM paths are outside the hooks. Goose's mapping is
2 GiB plus a 1 MiB guard; it is not evidence of that much RAM use. Runtime and
printing activity outside the workload cannot be scored as per-record cost.
Post free calls include NULL inputs (largest Holo 6/5 NULL, C 2/1 NULL, Rust 3/0
NULL), with no freed/live-byte ledger. The selected Goose main returns normally
without explicit `gs_rt_stop`; absence of post free/munmap does not measure its
logical scoped reset or establish a leak/free advantage.

Nine native calibration cases validate known events, real transfer calls,
overlap/contents, allocation failure, shared persistence through `_Exit`, and
rejection of missing/extra boundaries, absent observer and overflow. A separate
nine-case errno supplement compares representative native witnesses with the same
uninstrumented fixture using the unchanged observer; it does not rewrite or
re-execute the primary campaign. All 36 nested-component and 88 parent host tests
pass. Earlier fortify, nested-delegation and boundary-validator failures remain
preserved alongside their corrected, separately identified attempts.

The [copy-path review](../evidence/memory-telemetry-20261109/static-copy-path-review.md)
and an explicit successful inline-copy calibration show the limit: zero observed
transfer calls does not establish zero inline/scalar copies, initialization,
spills or physical traffic. Bump construction is still allocation under strict
`noalloc`. Dynamic public API accounting now has scoped evidence; optimized
copy-volume, isolated reset scaling, retained/committed/resident backing and
broader shapes remain M1 gates. Compiler source and release bytes are unchanged.

### Populated scoped-release scaling — November 9

The [scoped-release matrix](../evidence/reset-scaling-20261109/README.md) extends
the earlier unresolved source-clock observation with real nonempty release/reuse
and code inspection. It adds ten orthogonal shapes: count 1..1,048,576 with empty
payloads, payload 0/32/4096 bytes at 4,096 records, and 0/1/4/16 extra inline wrappers
at 4,096 records with 32-byte payloads. Shared anchors appear once. All shapes have
one batch/associated marker; more inline data does not mean more independent owners.

Seventy actual native controls run as Holo/C/Rust O0/O2 and Goose O2. The intended
checkout requires native Goose `build -O2`; Goose O0 is unavailable. Holo uses
the unchanged checked experimental source path and full-width derived 16-byte
prefixes, matched by tuned flat C/Rust arenas. Goose retains its natural 12-byte
prefix and actual data-stack backing. Each cycle constructs/scans/validates a
fresh populated batch before three adjacent clocks; lexical owner exit lies
between the last two. A real id 777/three-byte probe then validates reuse. Holo/C/
Rust verify empty markers and reuse at the original integer address, after old
references end. Goose validates real image/checksum/reuse values while private
marker/address slots remain explicit unavailable sentinels.

**All 700 processes / 5,600 populated releases pass**, including 70 preflights.
Measured data is 630 process clusters with 8 rounds each (5,040 rounds), with varied
sequential order. No compiler/test competitor ran alongside native timing. The
independent audit imports no suite/compiler/generator model, recomputes the math,
checks raw stdout and native identities, and preserves all outliers. Original
Holo O0/O2 checked modules match byte for byte for each shape.

For every shape, original Holo IR restores exactly one region marker in the
scope interval. O0 native retains that restoration among five gap instructions
(three stores total, including two unrelated spills). O2 has one timestamp move
and **no marker store in this gap**. The per-profile linked sequences are identical
across all ten shapes. C O0 invokes its explicit release helper; Rust O0 invokes
Drop glue. C and Goose O2 similarly omit their private reset store from the gap;
Rust O2 retains one marker store. These are structural observations for actual
outputs, not a whole-language instruction or latency ranking.

Physical reset latency remains **unresolved**. All 70 process-median distributions
have zero median control/scope gaps. Of 5,040 measured rounds, 4,942 adjacent controls
and 4,922 scope gaps are zero; nonzero observations are preserved. A separate native
probe confirms that the campaign's CLOCK_MONOTONIC reports 1,000 ns resolution;
96,832 of 100,000 adjacent pairs were zero. The alternate RAW clock reports 42 ns
resolution, but was not substituted into the campaign. Finer clocks cannot make
an optimized-away private marker store into physical reset work. No subtraction,
forced volatile metadata, reset-empty replay, extracted reset kernel, foreign
workload, safety change or optimizer promise is introduced.

Eighteen component host tests, 88 parent tests and 36 API-diagnostic tests pass.
All ten C controls pass separately compiled ASan+UBSan runs (80 additional rounds)
and `-Wall -Wextra -Werror` checks. All ten Holo source checks pass. The compiler
was built offline for provenance, but its source/release bytes remain unchanged;
Rust compiler fmt/clippy/test checks were not rerun for benchmark-only changes.

The one-marker plain-data shape gate now has direct functional and generated-code
evidence. General physical reset timing, explicit `core.reset`/early-error paths,
additional independently owned stacks, optimized inline-copy accounting and
retained/committed/resident backing remain distinct limits or open M1 evidence.
No milestone, Goose mechanism beyond this scope, or universal constant-time
hardware latency is declared complete.

### Retained backing and resident pages — November 9

The [backing-residency diagnostic](../evidence/memory-residency-20261109/README.md)
collects sixteen unchanged native O2 controls from the sealed reset study, three
fresh processes each. Four cases cover 65,536 and 1,048,576 empty-payload records,
4,096 records with 4,096-byte payloads, and sixteen extra inline wrappers around
4,096 records with 32-byte payloads. Holo/C/Rust retain their full-width derived
16-byte prefix; Goose retains its natural 12-byte prefix and actual runtime.
Workload or compiler binaries are not rebuilt.

A separate Darwin observer selects backing by exact executable, real allocation
return PC, extent and unique success. Native branch/import-stub validation covers
Holo/C direct malloc calls, Rust's retained-return-address allocator tail chain
and Goose's mmap. Each process records all 24 existing helper clocks (full,
full-control and post-logical-reset for eight rounds), alongside allocation,
guard, actual free/unmap and final-before-teardown states where available.
Native eleven-field output still validates every id/byte and actual reuse.

**All 48 observations / 1,152 clock checkpoints / 384 reset pairs pass.** Every
full/reset pair retains identical selected backing residency. Relevant resident
data, separate from virtual mapping extents:

| Shape | Holo/C/Rust full → reset MiB | Goose full → reset MiB |
|---|---:|---:|
| 65,536 records, empty payload |1 →1|0.75 →0.75|
| 1,048,576 records, empty payload |16 →16|12 →12|
| 4,096 records, 4,096-byte payload |16.0625 →16.0625|16.046875 →16.046875|
| Depth 16, 4,096 records, 32-byte payload |0.1875 →0.1875|0.171875 →0.171875|

Holo/C/Rust capacities are exact malloc requests, with actual `malloc_size`
usable extents recorded; they are not total heap reservations. Goose's actual
2,049 MiB mapping comprises 2 GiB usable space plus a protected trailing 1 MiB
guard. Only the touched data pages above are resident in the selected range.
Whole-process virtual/resident/footprint/internal/external/compressed/reusable
accounting is recorded separately and includes observer costs. None is renamed
generic committed bytes, which remain unavailable on this diagnostic.

Independent mapping traversal establishes clipped mapped coverage; `mincore`
counts resident/attribute pages without reading payloads. It can succeed across
holes on Darwin, so mapping and residency are distinct gates. User/kernel page
granularity must match; buffers and traversal are bounded. Observer output/scratch
is prefaulted, but kernel page queries may allocate and cost work proportional to
pages/regions. Instrumented elapsed gaps are never timing or reclamation-cost
results and no `noalloc` claim is introduced.

All 36 host final `free` observations retain the same mapped/resident range
immediately afterward in these captures. This is not a leak diagnosis or an OS
promise: allocator bookkeeping, later reuse/pressure and OS decommit differ.
An independent supplement runs twelve fresh C calibration processes without the
observer. It confirms that real zone/all-zone in-use blocks and bytes decrease
on free while the mapped/resident range remains, without claiming a particular
cache mechanism or universal allocator policy.
The Goose control returns normally without explicit runtime shutdown, so no
backing-release event occurs before its final pre-process-death snapshot. Existing
Goose explicit trim facilities are separate and not exercised. No decommit,
pressure, allocator setting or baseline storage policy was changed.

Twelve instrumented native calibration cases and eight uninstrumented baselines
cover real touches, logical marker reset, malloc free, actual unmap, protected
guard, `_Exit`, native errno forwarding, missing/extra clocks, wrong caller/extent,
absent observer, invalid limits and a real live mapping hole. Earlier observer
revision/calibration and archive-binding smoke remain separate preserved evidence.
Twenty-three component tests plus 88 parent, 36 API and 18 reset tests pass.
Only observer/fixture compilation is new; Rust compiler source checks are not
rerun because production source and release bytes remain unchanged.

The first collection's frozen launcher did not persist allocator-specific
environment knobs. Later launches now record selected inherited values without
changing them; this is a documented metadata follow-up, not a rewrite of the
original evidence or a default-allocator claim. General allocator policy,
pressure-dependent retention, explicit trim/recommit tradeoffs and alternate
physical backing remain separate work. These captures close the selected
ordinary-reset retention observation, not all M1 memory/copy/reclamation gates.

### Bounded-backing acceptance — November 9

The [acceptance record](../evidence/backing-acceptance-20261109/README.md) adds
36 actual parsed-source native controls: four programs, three layouts and
O0/O3/ASan. All 117 sequential diagnostic processes and twelve native observer
calibrations pass. This is acceptance of **approved M1-R3's initial finite malloc
policy**, not automatic compiler-assigned VM stacks or completion of M1.

| Case | Established eligible behavior |
|---|---|
| Boundaries | Logical zero still acquires one physical byte, unusable by construction; one-byte-short/exact limits, capacity/overflow ordering, private-suffix rollback and prefix preservation pass. |
| Reuse | One acquired owner supports 1,024 small/full refill cycles, 2,048 capacity failures and 1,024 overflow failures, with stable addresses and exact final empty markers. No further backing requests occur during those cycles. |
| Owners | Sixteen total acquisitions normally; first/middle/last injected NULL takes the real failure branch and releases only successful owners in reverse order. Live outer data remains readable before cleanup. |
| Soak | Sixteen 4KiB/8MiB low/high-water cycles use one owner, exposing every fill through diagnostic boundaries and validating published ids, lengths and every payload byte. |

All nine normal reuse controls complete 9,216 cycles. Each layout/profile also
runs its soak normally and with a separate 64MiB C-owned allocation, touched at
the full checkpoint and freed before the reset checkpoint. Native wrappers
delegate to real malloc/free and implement no Holo record operation. The original
checked module is retained; a separate diagnostic copy changes only allocation
call targets and, for ASan, function attributes. Exact event-by-event request,
owner, usable-extent and counter conservation rejects missing/extra work.

For every optimized layout, the selected page-rounded backing remains
**8.015625MiB resident** at full, reset and refilled checkpoints, both normally
and with the competing allocation. The observed task resident total is about
9.4MiB normally and 73.4MiB with the stimulus; it stays near the latter value at
reset even after the competing allocation is freed. These single-process rows
illustrate allocator retention, not a leak diagnosis, permanent-retention
guarantee or statistical comparison. Request/usable extent, selected page
coverage, task VM accounting and footprint remain different quantities.

The stimulus is bounded demand, **not demonstrated system-wide memory pressure**.
Generic committed bytes remain unavailable. The selected pointer is cleared
before free; after-owner snapshots report task totals rather than selected
post-free pages. Holo trim/recommit is unsupported, and no host trim substitute
or new backing default is introduced. Injected NULL tests the existing failure
branch, not an OS OOM kill. At collection, the source adapter required acquisition
recovery to terminate entry. The [subsequent continuation implementation](#acquisition-recovery-continuation--november-9)
removes that adapter limit without changing this campaign's selected programs.
An explicit reconstructed rejected source reproducer remains historical evidence.

Independent replay derives source outputs and ownership ledgers without project
imports, checks raw streams and source/native/compiler identities, and reverses
the diagnostic-only LLVM transformation. Clock values and observer/sanitizer
costs do not establish speed, physical traffic or isolated reset latency.
Compiler/runtime/build inputs and the release executable are unchanged. Future
VM assignment, pressure and trim/recommit policy remain separate work; accepting
this initial finite policy does not require silently selecting any of them.

### Fresh construction and copy accounting — November 9

The [copy-accounting diagnostic](../evidence/copy-accounting-20261109/README.md)
adds fourteen actual parsed Holo cases across wide/derived/compact layouts and
explicit O0/O2 pipelines. Unary cases span payload lengths 0/1/31/257/4096 and
0/4/16 inline wrappers. Split records contain two variable byte fields followed by
a scalar trailer, retaining the required outer header. Target attempts succeed,
reject before either child or the trailer, fail real later capacity reservations,
or overflow a later byte extent. Mid-byte failure and existing aggregate input
copies remain unsupported, rather than simulated.

Four generations per program publish a small prefix, attempt the selected target,
retry, then reset/reuse. Each attempt has two existing source opaque boundaries.
Native status, used bytes, visible count, full published checksum, first-element
address, address reuse and capacity independently validate rollback/publication.
All 42 source/layout checks pass; 84 uninstrumented native controls and 168 traced
observations validate 672 generations at both profiles. The 67,620 recorded
payload-byte writes exactly equal semantic initialization and unique byte
coverage; recorded payload reads and within-generation rewrites are zero.

Those counts require a separate provenance argument: a scratch-to-final copy can
write its destination once while reading outside it. The original constructor
checker permits only final receiving-tail writes, fixed region metadata loads
and i64 loop counters initialized to zero/incremented by one. It rejects aggregate
allocas, data staging slots, payload reads, unknown callees and shifted receiving
destinations. Producer calls and append destinations are inspected through the
actual emitted SSA, independently of the language checker's authorization.

The recorded optimization pipeline runs before instrumentation. Each admitted
load/store/transfer receives a site identity and after-operation logger callback;
unsupported memory/effect forms reject. Incompatible inferred function/capture/
memory attributes are removed from the diagnostic copy. Original controls remain
unchanged. `llc -O2` lowers both variants without another middle-end pass. Original,
optimized and traced LLVM, site maps, verifier/codegen/link commands, assembly and
linked native bytes are retained. Callback spills, backend constant-pool loads,
retired accesses and physical cache/RAM traffic are outside the LLVM counters.

Ten positive calibration pairs cover direct initialization, retained inline and
intrinsic scratch copies, repeated destination writes and an obsolete copy erased
at O2. All ordinary/diagnostic final sixteen-byte checksums equal 136. Retained
copies have sixteen outside-source read bytes even with sixteen unique receiving
writes. O2 can scalarize scratch to i8 slots, so scalar-slot reads are not presumed
harmless. The repeated-write case retains 32 payload writes over sixteen unique
bytes. The erased-copy O2 case has sixteen final writes and no staging reads;
O0 retains its obsolete copy/overwrite. Three negative bracket/site/readiness cases
reject without changing native final contents.

**Failure counts are an observable-harness result.** The source barriers expose
backing and may retain private failed-attempt writes that ordinary unconstrained
code can remove. Reset/retry generations have independent address coverage, so
legitimate reuse is not labeled redundant copying. No baseline contract, runtime
callback dependency, unsafe bypass, optimizer promise or numeric rule is added.

The [four-language static review](../evidence/copy-accounting-20261109/comparative/comparative-copy-review.md)
binds eight actual O2 Holo/C/Rust/Goose controls and 387 sealed replay files. It
distinguishes named-transfer absence from unknown dynamic copy volume. Historical
Goose growable-to-frozen conversion has scratch construction and a payload copy;
no unmatched nearby native binary is substituted for that source. Selected direct
Goose construction returns the frozen destination form. General Goose owned-error
exits can move replacement payloads over abandoned prefixes and are not equivalent
to Holo's current scalar-status append failure. Host dynamic copy counts remain
unavailable, with no language ranking inferred.

Twenty-eight component host tests pass alongside the existing parent/diagnostic
suites. The production compiler was built offline for provenance and its source/
release bytes remain unchanged; compiler Rust source checks are not rerun for
benchmark-only work. Fresh scalar-input construction now has scoped write/copy
evidence. The follow-up below covers a finite set of lifetime/owner/exit cases;
input/owned-result copy semantics, backing policies and physical performance
still limit M1 and later claims.

### Independent owner reset and cleanup coverage — November 9

The audit found no new compiler defect. Seven [checker tests](../tests/compiler/check/owner_check.rs)
now distinguish each batch's root through nested loops/branches, captured cursors,
and lexical name shadowing. Unrelated outer aliases allow inner reset; a live
alias to the selected batch rejects it. Explicit reset retains batch tail
ownership, while ending its lexical scope permits a new batch on that region.
Parsed-source cases check acceptance and exact rejection spans.

Four [source/native fixtures](../tests/runtime/owner_lifecycle.rs), with separate
[source and hand expectations](../tests/support/owner_fixtures.rs), compare all
three layouts against the independent model. Thirty-six native children cover
O0/O3/ASan. The continuation case retains an outer writer/readonly alias/captured
cursor across inner rejection, capacity rollback, explicit reset, address reuse,
lexical batch reuse and later outer growth. Two early-return cases close one
sibling owner before exiting through three active owners following rejection or
capacity failure. A fourth injects NULL at the real allocation boundary, then
executes the parsed source failure branch with two active enclosing owners.

Test-only wrappers verify exact successful acquisition/free counts and LIFO
backing release. Volatile fault configuration and a noinline wrapper preserve
the allocator result branch under optimization. The model mirrors only that
external acquisition fault in a clone of the parsed IR; no new source fault
syntax or record-operation implementation is introduced. Continuation observes
marker zero and reuse directly. Backing-free guards cannot observe a marker
restore immediately before its backing is freed; emitted cleanup inspection
remains distinct evidence for that operation. Native children have deadlines
and preserve source/IR/toolchain/commands/output plus compiler/test/Cargo snapshots
on failure.

Formatting, strict offline Clippy, and the full offline Cargo suite pass. Compiler
implementation and language semantics are unchanged. This closes the listed finite
owner correctness gate, not arbitrary lifetime soundness, physical cleanup timing,
owned-error transfer, mid-byte failure, or a new backing policy. M1 remains incomplete.

### Readonly byte-input construction — next M1 extension

**Implemented experimentally November 9.** The user authorized definition, then
implementation of this minimal path after the finite owner/reset gate. The
[source proposal](spec.md#readonly-byte-input-construction-proposal) and
[component map](compiler.md#typed-byte-input-path) make that work concrete.
M1-R2 already permits reading initialized inputs; this proposal gives that approved
capability a restricted experimental byte-parameter form. It does not promote a
general slice/reference API or external I/O boundary to an approved language rule.

Implement only synchronous readonly borrows of complete published byte fields,
forwarded through destination producers. Reuse explicit per-byte construction
and existing trap/status/rollback order. Do not add general aggregate movement,
scalar-helper byte parameters, raw pointers, asynchronous retention, or file/
network/stdin acquisition to make the demonstration appear more complete.

Implementation acceptance checks:

- [x] Introduce typed producer formals and actuals in the one checked IR, update
  every existing caller, and reject malformed programmatic input before LLVM.
- [x] Parse/check/lower the complete proposed source. Hand-derived output is
  `0, 1, 1, 2663, 73, 0, 2`; independently confirm nested sizes 96/80/76 and the
  selected capacities under Wide/Derived64/Compact32. Include nested named calls
  and synthetic literals so forwarding never treats byte descriptors as u64.
- [x] Test empty input, two aliasing inputs, same-batch published-prefix input,
  caller-held writable aliases, mutation before/after a call, and retained source/
  destination references across valid growth. Owned output must not change when
  the caller later changes source bytes.
- [x] Check input type/mode errors, readonly writes, escape/out-of-scope roots,
  unfinished-output access, implicit aggregate forwarding, unknown effects, and
  `noalloc` rejection. Require valid same-root examples to compile too.
- [x] Check capacity/rejection/overflow rollback, retry/reuse, partial nested
  construction and early exit. Separately check trapping source bounds failures
  before loads, failed reservation before a would-be-invalid element read, zero
  element reads for an empty output, and evaluation of unused actual arguments.
  Exercise huge derived destination lengths from small real inputs; never invent
  a supposedly validated huge input descriptor over tiny initialized storage.
- [x] Compare hand expectations and an independent semantic model with native
  O0/O3/ASan under all layouts; preserve source/IR/toolchain/commands and failures.
- [x] Inspect destination forwarding, descriptor passing, bounds checks, and
  absence of mandatory aggregate staging in the emitted example and backend.
- [x] Extend calibrated input-copy accounting with bounded source/destination
  ranges, exact receiving coverage, staging calibrations and native witnesses;
  retain the distinction between diagnostic IR effects and physical traffic.

Relevant Goose compatibility mechanisms remain explicit:

| Mechanism | Design/implementation status and acceptance boundary |
|---|---|
| Destination construction and static calls | Typed byte inputs and nested final-slot calls are implemented in one checked IR/backend. The source/native and code-inspection gates above pass. General generics/callbacks stay deferred. |
| Stable growth and data-stack bookkeeping | Reuse approved bounded backing and published-prefix stability; no new growth policy, region per payload, geometric buffer, or automatic stack assignment. Same-root native/alias cases pass for this path. |
| Flat/compact representation | Owned output retains existing inline schema/layout rules; a transient borrow descriptor is not a new stored field. All three modes pass size/value checks; relative links/alignment alternatives stay deferred. |
| Failure and bulk release | Scalar statuses, single append rollback and fixed-marker cleanup are preserved. Byte-input failure/retry/reset and exit with active owners execute; exact backing-free counts remain separate evidence from the owner suite. Owned-error movement and arbitrary byte-stream failure stay outside the slice. |
| Memory traffic | Reading input into owned output requires real payload copying; metadata capture is distinct. The later readonly-input diagnostic accounts for this finite path, while physical traffic and general speed guarantees remain unestablished. Scalar-seeded evidence alone cannot stand in for existing-input copying. |

The [maintained example](../tests/fixtures/byte_input.holo) now runs. Nine
[malformed-IR/positive checks](../tests/compiler/check/byte_input_check.rs), five new frontend
definition regressions, two model-frame regressions, and eight
[source scenarios](../tests/compiler/storage/byte_input_native.rs) cover the new path. The source
matrix has 24 independent model outcomes and 72 native children across three
layouts at O0/O3/ASan. Every case checks full expected outputs or an exact bounds
trap. Trap instrumentation redirects only the intended byte-index assertion;
setup assertions retain their normal termination, and a 4096 index exceeds the
actual 256-byte backing in one case. Native failures have deadlines and preserve
complete source/compiler/test/toolchain/IR/command/output reproduction inputs.

Review found a correctness gap while enabling reads: previous scalar-only pure
substitution could drop an unused helper actual containing a bounds trap. Internal
`Expr::Apply` now captures scalar actuals once, then evaluates the body in a fresh
scalar-only frame. It emits SSA operands without a runtime closure/buffer/indirect
call. The checker, model, recursive budget/depth inspection, and substitution all
respect that frame. A native regression confirms an ignored input read still traps.
Old scalar-only templates retain their valid pure substitution.

Formatting, strict offline Clippy, and the full offline Cargo suite pass. The
[validation/code inspection record](../evidence/byte-input-20261109/README.md) retains
the full test log, original/verified LLVM, all-layout inspection and O0/O3 assembly,
commands, tool versions and source identities. The first trap-harness trial looked
for llvm.trap although this runtime uses abort; that test-only assumption was
corrected. The final observer selects the intended SSA guard and cannot accept an
unrelated assertion or signal as its expected trap.

This feature increment supplies functional implementation and correctness evidence.
The later input-accounting/comparison section adds its own finite cost evidence;
physical traffic and general performance guarantees remain unestablished. No speed
improvement follows from the feature alone, and prior scalar-seeded measurements
do not cover this mechanism.
Tagged alternatives, queues/images, pools and other deferred mechanisms are
unchanged. External ingress remains unavailable. M1 remains incomplete.

### Repository layout migration — November 9

The user approved direct root components: `compiler/`, `cli/`, `lib/`, `runtime/`,
`examples/`, `tests/` and `docs/`, without a `crates/` wrapper. Cargo now coordinates
two members: `holo-compiler` retains the public Rust library name `holo`; the `holo`
CLI depends on that one compiler. LLVM Rust generation is under
`compiler/src/backend/llvm/`; the emitted prelude and optional native measurement
helper are under `runtime/`. Executable Holo examples are directly in `examples/`,
while Rust API examples belong to `compiler/examples/`. `lib/` documents the future
Holo library boundary; no core/std implementation or empty subhierarchy is invented.

All 21 shared integration targets are explicitly registered once in the CLI
manifest, preserving executable discovery and root-level tests. Replay snapshots
and benchmark identity now include compiler, CLI, native runtime, root/member
manifests and lockfile. At migration, the legacy project and preserved source
snapshots had independent manifests outside the fresh workspace.
Language rules, checked lowering and native algorithms are unchanged.

The [migration record](../evidence/project-layout-20261109/README.md) retains path
mapping, source identities and validation. The full workspace's 160 Rust tests
pass, as do 113 relevant benchmark host checks. Formatting and strict workspace
Clippy pass. All three byte-input layouts emit LLVM byte-identical to the selected
pre-move records. The compiler API example validates O0/O3; the measurement example
exercises moved runtime support and complete provenance. A tiny zero-interval
smoke was correctly rejected and retained before a larger smoke passed; neither
is a performance comparison. At migration, the old prototype and prior build
contents retained all 168,201 recorded regular-file hashes. Archive paths stay historical;
the relocation map identifies their current maintained equivalents. M1 remains
incomplete and its remaining input-copy/performance gates are unchanged.

### Generated output organization — November 9

Disposable output now has one [build directory](../README.md#build-output). Cargo's cache is
`build/cargo/`, compiler API runs use `build/examples/`, benchmark work uses
`build/benchmarks/`, and failed checks retain replay inputs under `build/failures/`.
The repository Cargo configuration is included in new compiler provenance;
benchmark tooling discovers Cargo's actual target directory, including explicit
environment overrides, rather than assuming a root `target/` binary path.

Existing records moved together from `build/` to [evidence](../evidence/README.md),
at the same directory depth. Their original bytes, sibling relationships and
source-relative links are preserved. The categorized index distinguishes
comparisons, diagnostics, correctness records and exploratory probes without
rewriting sealed inputs or disguising failed experiments as accepted results.
Recorded commands and absolute paths remain descriptions of their original runs;
maintained documentation and archive defaults point to the current home.

This is output organization, not a language change or performance experiment.
Cleaning disposable builds must not remove preserved evidence. M1's remaining
correctness and performance gates are unchanged.

The [cleanup record](../evidence/generated-layout-20261109/README.md) verifies
63,370 original files unchanged, with no newly broken historical relative links.
All 160 Rust tests and 116 benchmark host checks pass, along with formatting and
strict Clippy. A stale moved-cache CLI-path failure was retained, then resolved by
clearing only Cargo output and rebuilding. API and source examples execute from
the new paths; the measurement smoke captures all 49 build/workload inputs,
including Cargo configuration. These checks establish organization and replay
behavior, not improved native performance.

### Readonly-input accounting and comparison — November 9

The [input diagnostic](../evidence/input-copy-20261109-v2/README.md) now supplies
finite copy evidence for the implemented complete-field readonly borrow path.
Fourteen actual parsed-source cases cover empty through 4,096-byte inputs,
4/16 nested wrappers, two aliased formals, same-batch published-prefix inputs,
later rejection/capacity/overflow, retry and reset/reuse. All three layouts run
at O0/O2: **84 ordinary controls, 168 traced children and 672 traced append
generations**. Native checks validate status, publication, complete byte values,
source preservation and actual source/output identities independently of counters.

Across those traced generations, **58,848 source payload bytes are read and
58,848 final payload bytes are written**, with exact expected receiving coverage.
No source writes, receiving rewrites, missing/private misplaced writes or
unexplained staging effects pass. A separate original-emitter audit checks
guarded inputs, canonical record-derived descriptors and final-tail construction;
post-optimization auditing explains scalarized slots, precise metadata coordinates
and every nonpayload effect, including spare-backing rejection. The checker still
supplies schema/lifetime validity; this is not a general arbitrary-LLVM proof.

**22 known calibration pairs and six negative cases** distinguish retained and
erased staging, intrinsic copying, repeated writes, same-backing source/destination
and invalid registration. The first full attempt stopped on LLVM's canonical
`-1` spelling for a u64-max scalar argument; that audit limitation was corrected
with a constrained scalar-use proof and regression coverage. The failed attempt
and preparation probes remain in the accepted record's exploration directory.
No compiler or language behavior changed. Diagnostic opaque boundaries can retain
private failed writes, and IR counters exclude backend spills and physical traffic.
That diagnostic does not establish ordinary failure timing; the ordinary-sequence
follow-up below adds finite evidence. General aggregate/owned-error movement
remains open.

The [matched input campaign](../evidence/input-comparison-20261109-v6/README.md)
separately measures existing-input copy/build, a common output boundary, complete
scan and scope reset. It retains strong raw-span C/Rust controls alongside packed
record-backed input views, all three Holo/C/Rust layouts and natural compact Goose.
Input sizes are 31/257/4,096 bytes, with one hot reused source, 256 output records
per scope and approximately 64 MiB logical copying per process. **48 preflights
and 432 measured children** validate independent checksums and representations;
untimed warmup checks count, every id, every length and every payload byte.

The complete report retains average, empirical p50/p75/p90/p95/p99/p99.9, min/max,
raw samples, process resources, source/runtime/compiler identities, code review
and all physical policies. These are nine whole-run samples per row, not operation
latency tails or an isolated memcpy/reset test. Source-owner/backend facts,
fused versus component guards, Rust's trusted raw-record boundary, Goose's fatal
exhaustion/stack policy and unavailable final marker remain explicit differences.
Earlier validation-branch, source-knowledge and overlapping-background-work trials
are retained as exploration rather than silently omitted favorable/unfavorable
samples. The final campaign uses the strengthened coherent oracle and a separate
measurement window. No whole-language ranking or compiler improvement is claimed.

In the explicit compact view (the same 12-byte prefix as Goose), the 31-byte
whole-lifecycle medians are Holo **13.69 ns/record**, C record view **13.43**,
Rust record view **13.60** and Goose **12.72**. Strong known-span C/Rust controls
are **8.85/9.59 ns/record**. Thus source-boundary/length knowledge materially
changes this local comparison; a storage-only interpretation is unsupported.
At 257/4,096 bytes the distributions overlap and smaller median differences are
inconclusive. The report preserves all widths, spans, views and full ranges; no
default layout, optimizer promise or numeric rule is changed to obtain a result.

### Known lengths and dynamic short-copy experiment — November 9

The [complete record](../evidence/short-copy-20261109/README.md) investigates
the short-copy gap without changing language semantics or weakening the input
benchmark. Twenty baseline and twenty candidate untimed native probes pass exact
warmup/checksum/status/reset/representation validation. The baseline already
forwards constant 31/257/4,096-byte extents through ordinary nested calls when
source contents are not deliberately hidden. A real length assertion once per
batch also permits constant-length optimization with every opaque boundary kept.
For 31 bytes, baseline LLVM emits the same two overlapping 16-byte copies as the
strong known-span C control. Thus the earlier result does not establish a lost
length fact in ordinary code. These probes are code/semantic evidence, not timings.

A reversible candidate recognized only the exact same-formal initializer
`bytes(len(input), input[index])`. After unchanged width, overflow and reservation
checks, it emitted exact 16/16/8/4/2/1-byte chunks for lengths at most 32 and LLVM
memcpy for larger inputs. Checked published-prefix/private-suffix separation
supplied its disjointness proof; no blanket alias or invariant-load promise was
added. Every lookalike initializer retained ordinary evaluation and bounds traps.
Two private matcher checks and all 81 O0/O3/ASan byte-input children passed. The
expanded model/native sweep validates complete bytes for 40 lengths (0–34 and
63/64/65/127/4,096), separate and same-prefix outputs, mutation and stable aliases.

The initial quiet campaign runs **65 preflights and 585 measured children**:
13 exact-source Holo before/after pairs plus the 39 original C/Rust/Goose controls.
The independent repeat runs **26 preflights and 390 measured children**, using
the exact same binaries for every Holo pair with 15 samples per row. Both retain
all layouts, full distributions, average/p50/p75/p90/p95/p99/p99.9/min/max deltas,
throughput, process resources, code sizes, complete raw samples and tool identities.
No Git main/branch comparison exists in this workspace; the baseline compiler and
its source are captured explicitly. Repeat foreign controls are retained from the
first record rather than presented as newly measured. Mixed sizes cover 0–65 in
one actual copy function; checked-31 keeps all opacity and adds a real condition.

| Case | First median elapsed change | Repeat median elapsed change |
|---|---:|---:|
| Unknown 31, compact | -29.07% | -29.21% |
| Unknown 31, derived | -29.10% | -28.30% |
| Unknown 31, wide | -14.81% | -10.08% |
| 257, wide | +12.44% | +13.21% |
| Mixed 0–65, wide | +9.35% | +8.50% |
| Mixed 0–65, derived | -5.36% | -6.80% |
| Mixed 0–65, compact | -3.79% | -4.08% |
| Checked 31, compact | +1.34% | +6.50% |

Negative elapsed change is faster. Short compact/derived gains have disjoint
sample ranges in both runs; many other ranges overlap. Repeated slower wide
medians are a default-adoption concern, not a statistical significance claim.
4,096-byte repeat differences are at most 0.33% and inconclusive. These are hot-
input whole copy/build/barrier/scan/reset intervals, not request tails or a
language ranking. The reference toolchain, capacity and failure-model differences
remain disclosed in the reports.

Generated-code review also finds payload-load hoisting in the candidate, larger
instruction text, and a widened optimized 32-byte source read for a 31-byte field
inside its live backing allocation. Emitted exact chunks do not establish exact
optimized traffic. No safety defect is inferred from that in-allocation read;
the existing strict input accounting was not bypassed or declared passed for the
candidate. The short-copy gain cannot be attributed solely to call elimination.

**Outcome: retain the experiment, restore the original compiler.** All compiler,
runtime and configuration inputs match the captured baseline after restoration.
The stronger boundary fixture and reusable paired measurement/report tools stay.
No experimental compiler flag, second lowering path, public syntax, layout
default or semantic promise is introduced. The Goose mechanism ledger below
continues to apply: flatness, growth and fixed-marker reclamation are preserved;
automatic data stacks, broader representations, generic specialization, queue/
image transfer, pools and the remaining backing/failure-cost evidence stay open.

### Whole-input copy trial — November 9

The [combined evidence record](../evidence/copy-and-recovery-20261109/README.md)
preserves a rejected optimization and the separate source recovery improvement.
No Git main exists: the prior compiler is captured by its 46 actual inputs and
release SHA. Its preserved native controls match that identity exactly.

The reversible copy candidate recognizes only
`bytes(len(input), input[index])` for the same formal and the unmodified index.
All width/overflow/capacity checks retain their field order. Captured input length
proves every original read in bounds; checked published-prefix/private-suffix
separation proves the destination range disjoint. One align-1 LLVM memcpy replaces
that loop; arbitrary lengths, other inputs, transformed indices/values and eager
helper applications keep their checked element path. Ninety O0/O3/ASan byte-input
children pass, including a new empty/nonempty eager-argument trap regression.

The first quiet run includes 65 preflights and 585 measured processes: thirteen
same-source Holo pairs and 39 actual C/Rust/Goose references. A second run measures
all thirteen identical-binary Holo pairs again, with 26 preflights and 390 samples.
Pairs are adjacent and alternate orientation, giving 5/4 and 8/7 balances. Every
mean, throughput, p50/p75/p90/p95/p99/p99.9/min/max result, paired ratio, order
stratum, source/native/object hash and outlier is retained and independently
recomputed. Foreign controls are not remeasured in the repeat.

| Case | First marginal p50 | Repeat marginal p50 | First paired median | Repeat paired median | Repeat slower pairs |
|---|---:|---:|---:|---:|---:|
| 31B, default Derived64 | +0.16% | +0.01% | +0.17% | +0.09% | 9/15 |
| 257B, default Derived64 | -8.05% | +0.23% | -1.53% | -0.06% | 7/15 |
| 4096B, wide | +0.68% | +2.88% | +2.49% | +1.41% | 14/15 |
| Mixed 0–65, wide | -0.94% | -0.25% | +2.66% | +0.76% | 9/15 |
| Checked 31B, compact | -0.14% | -0.20% | +0.03% | -0.02% | 7/15 |

Positive elapsed changes are slower. All marginal sample ranges overlap; these
are descriptive whole-run results, not statistical significance or request tails.
Wide 4096 is slower in 9/9 first-run pairs and 14/15 repeat pairs, in both orders.
The default Derived64 cases are effectively flat in the repeat. Mixed-Wide
instruction text grows 1,840→2,828 bytes. This does not justify adopting the candidate.

**Outcome: restore the original byte lowering.** No bulk-copy flag, second
lowering path or new runtime contract remains. A stronger original-input auditor
can verify the reviewed bulk form with exact pointer/extent pairing, reservation
guards and CFG dominance; 69 host tests include bypass/staging negatives. It passes
three actual captured candidate example modules, not a claimed dynamic bulk-copy
count. The existing fourteen trace cases use independent scalar lengths and do
not exercise this matcher. The inspected target's memcpy resolves to 178 reachable
leaf instructions and is eagerly bound in all thirteen binaries; that is evidence
for this host image, not a portable `noalloc`/`noblock` guarantee or adopted policy.

### Acquisition recovery continuation — November 9

The source adapter now accepts an explicit acquisition handler that finishes
normally. Its completion skips the failed owner's success-only lexical tail and
resumes after that block. It can use permitted enclosing aliases/scalars, acquire
an explicitly written replacement, or conditionally `return 0`. Failed owners
never enter the active cleanup stack. There is no implicit retry/fallback,
allocation, data copy, or new backend operation.

Main still needs explicit zero return on every path. A reverse source termination
fold accounts for region declarations consuming their success tail: both that
tail and recovery must terminate to make the region's whole block terminating.
A return that exists only in the success tail cannot conceal handler fallthrough.
Flat declaration lists do not add termination-analysis recursion depth.

Seventeen frontend checks include unavailable failed-owner/success/handler-local
names, invalid top-level fallthrough, reset with a live outer reader, retry-reference
escape and permitted alias-visible writes. Ten owner fixtures run under all three
layouts and O0/O3/ASan, giving 90 native children with hand outputs and an independent
model. They include the maintained [recovery example](../tests/fixtures/recovery.holo),
normal success, NULL with replacement construction, later outer append, both
outcomes of conditional handler exit, and two failed retry acquisitions with only
one acquired/freed outer owner. Exact LIFO and fault counts are observed at the
real allocation boundaries; no foreign record kernel is used.

Only `frontend/lower.rs` and `frontend/lower/declarations.rs` differ from the
captured production compiler. Backend/runtime/build inputs match the old control.
All thirteen original benchmark sources emit byte-identical LLVM through the
final compiler, so the rejected optimization has not leaked into ordinary code.
This is a source correctness/usability improvement, not a claimed native speedup.
Core spellings remain experimental; owned errors, general returns, external
resources, pools and async I/O remain outside this increment. M1 remains incomplete.

### Ordinary construction failure and retry costs — November 9

The [ordinary failure campaign](../evidence/failure-costs-20261109/README.md)
adds finite uninstrumented sequence-cost evidence for the existing compiler.
Fourteen cases cover entry/right-child/trailer rejection, left/right capacity and
size failure, 31/4,096-byte payloads and 0/4/16 nested wrappers. All three layouts
use the same checked lowering. **168 O0/ASan correctness children**, **84 ordinary
O2 preflights** and **756 measured children** pass exact status, ordered digest,
publication, complete byte, source/prefix preservation and reset checks.

The two variants execute the same hot call expressions. Request flags and
requested lengths are seeded with valid/failing values behind one input-only
opacity boundary before timing; identities, payloads and final outputs match.
The earlier preliminary constant-argument control was refined before sampling;
its source checks and tiny native probes are retained as exploration. Every
admitted pair has byte-identical source for request traversal, attempt, conditional
retry and output consumption. Their expected outcome assertions differ.

Output backing is never exposed through an opaque call. There are exactly two
clocks, no per-attempt callbacks or producer instrumentation, and computed
published-data and ordered-status assertions precede the end clock. Bounded
original/optimized CFG and dataflow checks independently bind those computed
values to terminating guards. Code inspection admits ordinary dead-store removal
or recovery fusion; it does not require failed private writes to remain.

Each interval includes traversal, checked construction, caller status counters/
digest, conditional retry, full published checksum and terminal assertions.
Allocation, exact per-byte/source validation, marker reset and backing release
are outside. Output first-touch faults/lazy commitment can contribute inside;
cache residency, affinity and frequency are uncontrolled. Counts are 270,600
requests at 31 bytes and 4,096 requests at 4,096 bytes. Each request publishes one
record with two owned payloads; a retained sentinel is also scanned.

Default Derived64 depth-zero median whole-sequence costs:

| First-attempt outcome | Successful control, ns/request | Failure + retry, ns/request | Elapsed change |
|---|---:|---:|---:|
| Entry rejection | 44.57 | 46.88 | +5.19% |
| Right-child rejection | 43.10 | 55.96 | +29.84% |
| Trailer rejection | 43.27 | 66.56 | +53.84% |
| Left-field capacity | 42.58 | 45.19 | +6.13% |
| Right-field capacity | 43.04 | 55.53 | +29.01% |
| Left-field size overflow | 42.96 | 44.87 | +4.44% |
| Right-field size overflow | 43.06 | 54.97 | +27.66% |

These rows use 31 bytes per field. The report preserves every layout and all
average/p50/p75/p90/p95/p99/p99.9/min/max values, ranges, throughput and process
resources. Many small differences and 4,096-byte comparisons have overlapping
ranges. Nine whole-process samples do not establish request tails or statistical
significance. Failure streams are homogeneous 100% first-attempt failures followed
by successful retries; rare/mixed failure frequencies remain unmeasured.

Inspected generic producer code retains payload-copy loops before later failure
checks, including both copies before trailer rejection. It also retains source
bounds checks and vector alias-versioning paths. Separate requested lengths and
captured input extents give the initial attempt and safe retry different proof
opportunities. No delta is interpreted as isolated rollback/failure latency or a
count of physically initialized failed bytes. The main retains attempt/retry call
sites, while most source wrappers inline: depth-16 Derived64 trailer rows measure
43.19/66.43 ns/request, close to depth zero. Wide retains more headers/checks and
different sizes; source depth does not imply a matching number of unwind frames.

An independent standard-library replay uses residue enumeration and a matrix
recurrence for the status digest, without importing the workload oracle. It
rechecks all native witnesses, schedules, quantiles, artifact identities and
actual object text. The compiler's 46 source/runtime/config inputs and release
binary remain unchanged. Only benchmark tools, bounded observation's explicit
parser interface and documentation change. The existing five-field parser
default remains compatible. Isolated failure bounds, physical traffic, general
error payloads, mid-byte recoverable failure and backing policy remain open.

### Generated cleanup-path acceptance — November 9

The [preserved cleanup record](../evidence/cleanup-audit-20261109/README.md) closes
the finite generated-cleanup task identified by the acceptance review. The
maintained [component](../benchmarks/region-lifecycle/cleanup-audit/README.md)
builds fourteen real parsed-source cases under all three layouts at O0/O3/ASan:
**126 controls, 252 ordinary/observed native executions, 45 independent model
outcomes and twelve allocator-observer calibration gates pass.** Original-LLVM
proofs cover 180 windows; ordinary O0/O3 linked-code audits cover 120 windows
across 84 controls. There are no clock intervals or timing results.

Explicit reset/refill and lexical release each vary record count (1/257), payload
(0/4096 bytes) and inline depth (0/16) independently. Late rejection abandons a
private left child at depths zero/sixteen; early completion varies one/three
independent owners with fixed per-owner work. The same acquisition source runs
normally and with an actual NULL at its 4096-byte request, then constructs an
explicit replacement owner and resumes outer work. Separate observed LLVM only
renames allocator calls; its strict event parser verifies real requests, NULL,
LIFO frees and exact conservation. No native helper implements record operations.

| Path | Original checked cleanup | Actual ordinary native observation |
|---|---|---|
| Explicit reset | Top/end/count: three stores for every selected shape/layout. | O0 has three fixed-frame writes plus the separately identified endpoint print argument. O3 has only checkpoint argument setup. |
| Lexical release | One saved-top restoration. | O0 has one fixed-frame write plus endpoint setup; O3 has only endpoint setup. |
| Failed append | One pre-call marker restoration on the nonzero-status edge; no publication. | The native attempt window includes construction and is reported separately. Its payload writes/calls are not claimed as isolated rollback work. |
| Early completion | One restoration per active batch and one free per acquired owner, statically emitted in reverse order. | O0 has one/three frame writes and one/three frees, then exit. O3 removes those private stores and retains exactly one/three frees, then exit. |
| Inner/replacement owner release | One restoration plus one backing free; failed acquisition contributes no owner. | Both O0/O3 retain the one free. O0 includes the restoration candidate and possible spills; O3 has no non-checkpoint stores in the selected window. |

All cleanup-only native windows are acyclic with explained fixed-frame writes
and expected free/exit calls. Store roles are not inferred merely by counting
all stack writes: actual Darwin variadic arguments and annotated spills are
separate. Original SSA origins establish marker meaning; native inspection
establishes actual instructions and surviving work. This is scoped structural
evidence for the existing fixed-marker requirement, not hardware latency or
a formal optimizer-correctness proof.

Original nested producer errors have bare scalar/pass-through return suffixes.
At O0, 558 mapped native error/shared-return blocks have acyclic frame-only
suffixes. Optimized wrappers mostly disappear. One retained wide/depth16 producer
has 36 literal status-1/2/3 native tails; its split shared LLVM return block has
no one-to-one native mapping. Those tails are reported separately without a
general source-frame, depth or isolated error-cost claim. Earlier construction
and allocator free remain outside the fixed logical-reset claim.

The preserved source includes independent hand arithmetic and the existing safe
value model, which derives layout without compiler planning. An early host oracle
omitted Pair's required retained header and was corrected; the compiler was right.
An initial diagnostic link duplicated helper symbols; a host adapter link required
Rust's matching ThinLTO. Development collections also refused changed reader
sources or an overstrict ordinary-print proof. These failures and partial records
remain separate; only the frozen v3 collection supplies the counts above.
Postcollection load-provenance and standalone-replay hardening reaudits preserved
bytes without recompilation or native reexecution; all 84 native audit objects
remain identical. The independent replay checks full unique matrix coverage,
raw source/model values and artifact/source hashes, not a second structural proof
or an independent reimplementation of the raw allocator-event parser.

Compiler/runtime inputs and the release CLI remain byte-identical to the prior
accepted implementation. Physical reset timing, target/runtime helper effects,
generic commitment, VM/trim policies and the final M1 decision remain open or
unavailable as already documented. This closes the listed finite cleanup item;
it does not close M1 or establish full Goose-compatible performance.

### Current M1 evidence boundaries

This ledger reviews the required Goose mechanism checklist without promoting
deferred mechanisms or experimental source forms to approved language rules.
The full [specification checklist](spec.md#goose-performance-compatibility-checklist)
remains the design authority; the entries below identify what this increment
establishes and why the broad memory/reclamation gate remains unchecked.

| Mechanism | Implemented eligible case and strongest evidence | Remaining boundary |
|---|---|---|
| Data stacks, flatness and growth | Explicit finite regions, bump construction, inline nesting and stable published-prefix append; native alias/growth checks and original final-tail provenance pass. | Automatic stack assignment and VM backing are pending; stable addresses do not establish check-free growth or backing-cost parity. |
| Compact scalars, lengths, links and tags | Explicit 32/16/12-byte prefix policies pass native/model conformance and input-copy controls. | General narrow scalars, relative links, tagged modes, alignment policies and automatic width inference remain deferred. |
| Static abstractions and optimization | Acyclic statically resolved producer calls and checked readonly descriptors operate in one backend; no mandatory closure or indirect call is introduced. | General generics/callbacks, check-free contracts, multiversioning and independent target-path optimization remain pending. |
| Destination construction and errors | Complete readonly byte fields construct owned outputs in final slots; calibrated diagnostics account for payload/metadata and partial failure. The ordinary failure campaign adds 42 matched success/failure-retry sequence pairs without output barriers, plus optimized timing/dataflow admission. | General aggregate transfer, owned-error movement, streaming failures, rare/mixed failure rates and isolated/general failure-cost bounds are unestablished. |
| Bulk release and backing | The [generated cleanup audit](#generated-cleanup-path-acceptance--november-9) adds original/optimized/native reset, rollback and owner-exit evidence, with independent outputs and actual allocation faults. The [bounded-backing acceptance](#bounded-backing-acceptance--november-9) establishes many-cycle refill/exhaustion and live-page retention under bounded competing demand. | General physical reset timing remains unresolved at the local clock floor. Automatic VM backing, genuine pressure, trim/recommit policy and generic commitment remain unestablished; the tested finite policy is accepted within its documented restrictions. |
| Flat queue messages and validated images | Not implemented by this increment; no serialization, validation or fixup work is silently added to plain data. | Eligibility, copying, concurrency, validation and relocation costs remain deferred. |
| Pools and stable handles | Not implemented by this increment; ordinary scoped data keeps its existing bulk reset. | Reuse, fragmentation, stale identity and link-update costs remain deferred. |

The next performance choice must follow the matched input/source-view and ordinary
failure-sequence results and actual code, with explicit copying/checking costs.
The acceptance review below reconciles these results with the remaining composite
gate. No diagnostic or source extension alone establishes M1 completion or full
Goose compatibility.

### M1 acceptance review — November 9

**Verdict at this review: M1 not complete; superseded by the [November 10 decision](#m1-acceptance-decision--november-10).** This is a review of the existing
approved finite, single-threaded region slice. It changes no language rule,
backend, acceptance guarantee or benchmark result. The checked items above remain
scoped to their eligible cases. Native evidence is linked where available; source
inspection alone is not a substitute for preserved generated-output evidence.

| Acceptance area | Strongest available evidence | Review outcome |
|---|---|---|
| Checked semantics and source execution | [Source acceptance](../evidence/source-frontend-20261109/README.md), independent model/native checks, and the latest [163-test validation](../evidence/copy-and-recovery-20261109/record/final-validation/acceptance.json). | Demonstrated for the implemented subset; finite testing is not a general soundness proof. Temporary `core.*` APIs remain experimental. |
| Flat destination construction and input copies | [Fresh construction](../evidence/copy-accounting-20261109/README.md) and [readonly-input accounting](../evidence/input-copy-20261109-v2/README.md) each cover 672 generations, with separate provenance and access-event checks. | Selected fresh/complete-field cases have no accepted unexplained staging. General aggregate/error transfer and physical traffic are unestablished. |
| Stable growth and compact representation | Native/model retained-alias checks, [layout conformance](../tests/compiler/storage/layout_native.rs), and [matched layout evidence](../evidence/layout-reservation-comparison-20261109/report.md). | Stable bounded append and explicit 32/16/12-byte prefixes are supported. Component guards remain; this is not check-free growth or universal compactness. |
| Populated release and cleanup | [Ten-shape release audit](../evidence/reset-scaling-20261109/README.md), followed by the [generated cleanup audit](#generated-cleanup-path-acceptance--november-9) across explicit reset, rollback and owner/early-exit paths. | Finite structural/correctness evidence is established. Native checkpoint/spill work, construction-inclusive failure windows, backing frees and physical latency remain distinct. |
| Finite backing and recovery | [Bounded-policy acceptance](../evidence/backing-acceptance-20261109/README.md), [resident-page observations](../evidence/memory-residency-20261109/README.md), and [acquisition recovery](../evidence/copy-and-recovery-20261109/README.md#acquisition-recovery). | Approved M1-R3 has scoped correctness/reuse/retention evidence. Acquired-owner frees are distinct from logical resets; VM backing, trim and genuine pressure are not established. |
| Ordinary failure cost | [42 success/failure-retry pairs](../evidence/failure-costs-20261109/README.md), with 756 measured children and independent output/dataflow replay. | Complete attempt/retry/consumption costs are measured. No isolated rollback latency, arbitrary error bound or rare-failure result follows. |
| Execution contracts | The experimental frontend checks closed scalar/producer graphs and rejects allocating constructors under `noalloc`. The [November 10 audit](#generated-helper-contract-audit--november-10) preserves42 ordinary outcomes,6 guard-selected outcomes, original/optimized call graphs, linked calls and selected runtime code. | Finite inventory complete; native contract acceptance remains open. `abort` has a potential cleanup/sleep frontier, and introduced memcpy/memset still require an explicit runtime boundary. No fatal-path exemption or trusted helper list is approved. |
| Four-language comparison | Actual C/Rust/Goose/Holo-new [lifecycle](../evidence/lifecycle-comparison-20261109/report.md), [layout](../evidence/layout-reservation-comparison-20261109/report.md) and [input](../evidence/input-comparison-20261109-v6/accepted/report.md) controls. | Comparable finite workloads exist, including tuned arena controls. Different check/backing policies, LLVM versions and timing variation limit attribution; no overall language ranking is supported. |

#### Remaining generated-cleanup audit

**Status: finite task completed by the [preserved cleanup audit](#generated-cleanup-path-acceptance--november-9).**
The original [owner tests](../tests/runtime/owner_lifecycle.rs) established outputs,
references, reuse and exact acquisition/free order, while their temporary successful
modules did not supply a focused retained native cleanup record. The new record
fills that [documented evidence gap](#independent-owner-reset-and-cleanup-coverage--november-9).
The acceptance contract used for this task remains explicit:

1. Preserve original checked LLVM, optimized LLVM, assembly and linked native
   paths for explicit `core.reset`, append rollback, ordinary lexical exit, early
   entry completion and acquisition-failure continuation. Retain successful and
   failing paths, including nested producer status propagation.
2. Vary populated record count, payload size and inline nesting while holding
   the active owner/batch shape fixed; vary independent owners separately. Reuse
   existing shape anchors, owner/recovery fixtures and independent expected/model
   outputs. Cover path topology under every layout at unoptimized and optimized
   settings, with separate sanitizer checks where needed for new fixtures. An
   exhaustive cross-product or a new timing campaign is not required by this audit.
3. Account for surviving marker/end/count updates and acquired-owner frees per
   reachable path. Reject unexplained payload walks, per-object cleanup,
   relocation or new allocator work. Explain removed/fused stores honestly;
   private metadata need not remain materialized after optimization. Separate
   backing allocator work and producer-frame status propagation from logical
   reset. Preserve identities, commands, raw output and unsuccessful audits.

The emitter provided a specific hypothesis, now corroborated for the selected
generated paths: [append rollback](../compiler/src/backend/llvm/publication.rs) emits one
saved-top restoration; explicit reset emits three metadata stores (top, end and
count); [lexical batch exit](../compiler/src/backend/llvm/scope.rs) emits one
restoration; [early completion](../compiler/src/backend/llvm/exit.rs) emits the
active cleanup list in reverse order during compilation. Its Rust loop is not
a generated runtime object loop. The cost can scale with the fixed set of active
owners, and backing frees remain separate. The finite audit establishes structural
work for these eligible cases, not a general physical latency guarantee.

#### Generated helper and contract boundary

The user approved [inference with optional explicit promises](spec.md#inference-and-explicit-promises)
on November 10. This is the annotation policy for continued work: infer provable
effects, require no redundant helper labels, and preserve written transitive
requirements. Omission does not prohibit allocation/blocking or certify unknown
native effects. Existing closed-graph tests already accept an annotated caller
using an unannotated pure helper, so a compiler rewrite or broad effect-system
expansion is not a prerequisite for the audit below. Detailed contract boundaries,
external trust and general summary/export mechanisms remain unfinished.

The [source graph checker](../compiler/src/frontend/functions.rs) admits contract
labels only on its closed restricted forms. That controls source calls and
destination allocation; it is not a general source effect checker or a portable
proof of the linked runtime. The [preserved unchanged-backend probes](../evidence/short-copy-20261109/record/knowledge-probes/findings.md)
already show optimized platform `memcpy` calls. Rejecting the later explicit
memcpy candidate did not remove the optimizer's ability to introduce such calls.
The candidate's [host runtime inspection](../evidence/copy-and-recovery-20261109/record/runtime-probe/README.md)
is scoped evidence, not an adopted trusted-runtime policy.

The audit task was to enumerate the generated/native helpers reachable
from accepted scalar helpers and destination producers on the supported target,
including recoverable status and failure/trap paths at O0/O3. Keep uncontracted
entry allocation, printing and cleanup separate. Distinguish
operations proved inside the closed graph from explicitly trusted target/runtime
operations and unresolved effects. A libc name or LLVM intrinsic alone is not an
effect proof. If an explicit trust boundary or changed accepted behavior is
needed, prepare it for review under the existing rules; do not silently weaken
the contract, add a runtime fallback or claim the source graph alone proves it.
This is an identified acceptance concern, not a demonstrated blocking operation
or a decision to implement the full-language effect system in M1.

#### Generated helper contract audit — November 10

**Initial audit disposition: finite inventory complete, policy unresolved, M1 incomplete.**
The later [implementation and acceptance](#m1-acceptance-decision--november-10)
resolves that policy gap for the selected profile; the original audit record is
preserved unchanged.
The [preserved record](../evidence/native-contracts-20261110/README.md) and
[maintained component](../benchmarks/region-lifecycle/contract-audit/README.md)
cover seven actual parsed sources, all three layouts and O0/O3:42 ordinary native
outcomes plus6 separate guard-selected outcomes. The scalar control reads an
opaque-backed input before all five arithmetic operations, preventing complete
constant folding. Nested unannotated children participate in annotated roots;
capacity1, overflow2 and rejection3 propagate normally and permit successful retry.

| Boundary | Observed evidence | Disposition |
|---|---|---|
| Copied scalar helper bodies | Closed arithmetic templates with no native function or dynamic allocation; caller actual evaluation and entry effects are separate. | Current restriction supports inference without redundant annotations. This is not a general summary/export system or proof for future operations. |
| Dynamic producer storage | Original checked producer graph reserves fresh region storage. | Strict `noalloc` rejection remains correct; no malloc is not no allocation. |
| Recoverable status paths | Nested status1/2/3 paths execute with rollback/retry; original root closures add no foreign error constructor. | Selected scalar error paths remain distinct from fatal checks, backing frees and physical error latency. |
| Byte initialization/copy | Ordinary optimized linked controls call memset/memcpy; original producer graph contains no such explicit foreign call. | Optimizer-introduced helpers participate in contracts. Whole-binary malloc/free/printf/exit/opaque imports belong to uncontracted entry work, not every producer. |
| Producer bounds failure | Original `first` root reaches `holo.assert -> abort`; all6 ordinary controls receive actual SIGABRT and all6 selectively redirected guard controls exit86. | Bounds termination is real, but is not a transitive `noblock` certificate. No noreturn exemption is adopted. |
| Selected installed runtime | Own-process memcpy has178 reviewed instructions without a call/wait frontier. Memset has69 decoded instructions but3 mnemonic classes remain outside this probe's reviewed leaf set. Abort's local graph contains `_cleanup` and `usleep$NOCANCEL` callsites. Exact image UUIDs/words are retained. | These are scoped static observations. They neither demonstrate an executed sleep in a normal crash nor bind every Holo child under arbitrary interposition. External contracts remain unproved. |

The LLVM reader rejects unknown direct targets, indirect calls, invoke/callbr and
unsupported forms. Original source-to-producer maps use the real checked vector;
scalar templates are not fabricated functions. All generated assembly definitions
are matched instruction-by-instruction to linked native code. O3 inlining can mix
producer and entry paths in main, so optimized main imports are inventoried without
assigning each edge to every original root. The own-process runtime reader stops
at call frontiers, conservatively follows possible fallthrough, records nearest
symbols and may enter adjacent code after a nonreturning call. It supplies no
transitive proof of those callees. All production inputs/release bytes are unchanged.

**Approved M1 direction, November 10:** preserve strict contracts
and the optional-annotation policy. Prepare a small explicit target/runtime
boundary rather than rewriting general effect inference:

1. For fatal compiler checks, prefer a specified fail-stop target trap with no
   Holo-generated formatting, libc abort, cleanup or user callback before the
   target transfers control to the host. Define that host/exception boundary
   explicitly; this does not promise OS crash handling is free of work or handlers.
   This approved change affects observable termination/signal behavior.
   Recoverable append statuses and normal cleanup stay specified.
2. Admit memory helpers only through an explicit reviewed target/runtime contract
   description, recording assumptions about binding/interposition, access ranges,
   initialization, allocation and blocking. Prefer allowing the optimizer its
   existing efficient helper choices once established; libc names or LLVM
   attributes are not sufficient evidence. Unsupported environments must remain
   unestablished/reject under explicit promises, without an invisible fallback.
   The current manifest/schema, enforcement mechanism and portable helper policy
   require implementation and acceptance evidence before the profile is admitted.
3. Keep native contract acceptance open until the selected policy has positive
   and negative coverage across generated/failure paths. Do not close M1 by
   excluding failure paths merely because they are nonreturning.

Alternatives are to reject annotated producers whose unproved fatal/helper paths
remain reachable, or explicitly weaken the public contract to exempt fatal paths.
The former reduces currently accepted programs; the latter weakens the intended
guarantee. Neither is selected. This is the concrete design review required by
[the repository rules](../AGENTS.md#decisions-that-need-the-users-review), not a
request to reapprove settled annotation ergonomics or the region memory model.

##### Concrete decision package for M1

**Approved policy, November 10; implemented and verified in the final review below.** The user
instructed proceeding with the next priorities after the concrete approval request.
This adopts the strict trap/runtime-profile package for the existing checked subset;
it does not certify unverified native output. The user-directed order
above keeps new comparison campaigns behind its verification and final M1 review.

| Question | Recommended decision | Observable effect / limit |
|---|---|---|
| Fatal generated checks | On a failed bounds/assertion check, transfer immediately through LLVM's target trap; emit no Holo formatting, libc abort, resource cleanup or callback before that transfer. Protect the invalid access as before. | Termination stays fatal, with no recovery/cleanup promise; signal and debugger behavior changes. Host exception/signal/debugger processing after trap transfer is outside Holo execution contracts, as OS scheduling/page faults already are. This boundary must be specified, not treated as a general nonreturning-call exemption. |
| Native memory helpers | Use an explicit reviewed M1 compiler/target/runtime profile for backend-generated memcpy/memset, with documented noalloc/noblock and valid-access obligations. Profile acceptance must identify compiler flags, runtime code/bindings and interposition assumptions. | This is a finite trusted implementation boundary, not an automatic effect proof for every compiled binary, any libc, sanitizer or interposed environment. Unknown effects remain unestablished; no silent fallback or blanket libc whitelist. |
| Enforcement scope | Keep current closed source call checking; verify the selected lowering/profile and its positive/negative acceptance cases. Treat annotated and inferred helpers consistently. | General effect exports, callbacks, foreign-call contracts and a universal post-link verifier remain deferred. Existing source acceptance cannot be advertised as a native guarantee outside the accepted profile. |

The accepted profile is the audited Darwin arm64 generic-target
LLVM22.1.2 configuration and its captured platform-memory implementation. The
runtime image identities/binding assumptions and reviewed fill/zeroing instructions
and hardware obligations are explicit in the [profile component](../benchmarks/region-lifecycle/contract-audit/README.md).
Other existing targets remain experimental; this profile does not certify them by analogy or require
adding synchronous runtime identity checks to every function.

This package preserves optional `with` promises, strict dynamic
region allocation accounting, recoverable append statuses1/2/3, ordinary cleanup,
aliases and the current storage/layout rules. It would not introduce an arbitrary
error allocation, library feature or public annotation syntax. The gate stays
was held open until the relevant generated/native checks and final gate review passed.

Independent preparation under that sequencing instruction is preserved in the
[contract follow-up](../evidence/native-contract-followup-20261110/README.md).
Audit admission now reads actual declaration kinds and written contracts through
the compiler's public AST, rejecting unknown/missing/misclassified roots instead
of assuming an unmapped root is scalar. Its fresh matrix has42 ordinary native
and6 selected-guard outcomes;49 focused host checks pass. Production compiler and
runtime inputs remain unchanged. This hardens evidence admission, not language
effect inference or native enforcement.

The separate isolated trap proposal copies existing checked LLVM and changes
only the assertion failure call target. All18 experimental native outcomes pass:
6 bounds checks receive SIGTRAP with linked BRK instructions, while12 copy and
rejection/retry outcomes retain expected values. These are labeled experimental
copies, not new compiler defaults, policy approval, or memory-helper certification.

#### Measurement limits and final gate disposition

These limits need explicit treatment in the final M1 decision, not an endless
search for a result or an automatic new feature requirement:

| Limit | Required treatment |
|---|---|
| Physical reset latency | Keep it unresolved when the operation is eliminated or the clock cannot resolve it. The accepted one-marker campaign uses a 1,000 ns clock and has no optimized marker store in the gap. Zero gaps are not zero-cost claims. A finer clock alone cannot time eliminated work; do not force volatile stores or an extracted reset kernel. |
| Copy/failure breadth | Carry forward the finite fresh/input-copy and ordinary failure results with their diagnostic/sequence boundaries. The cleanup audit must explain rollback work; it must not turn whole-sequence deltas into isolated error latency. General owned errors, mid-byte recovery and rare/mixed rates remain unmeasured. |
| Backing/commitment | Retain the approved bounded malloc policy, real failure/reuse evidence and separate request/usable/mapped/resident quantities. Generic committed bytes, genuine OS pressure and Holo trim/recommit remain unavailable or unsupported. No host substitute may be presented as Holo support. |
| Comparative provenance | Preserve the identified Goose executable and dirty checkout separately: source-to-existing-compiler rebuild correspondence is unverified in the input campaign. Early residency launches omitted allocator environment knobs. These limits restrict mechanism/default-policy attribution; they do not erase validated outputs or justify stronger claims. |

With the cleanup and finite helper audits recorded, resolve the native boundary and review
the composite gate against the exact claims it supports. Record any remaining
blocker. If completion requires
accepting an unresolved measurement boundary or changing the gate, present that
as a concrete decision for the user; this review does not grant that approval.
Do not silently turn an unavailable metric into a pass or substitute tests for
the required memory/generated-code evidence.

Automatic data-stack assignment, alternative VM backing, pools/stable handles,
queues/images, tagged alternatives, general generics/callbacks, broader ownership
and real asynchronous I/O remain deferred. They belong in the full
[Goose mechanism checklist](spec.md#goose-performance-compatibility-checklist),
but implementing every mechanism is not a prerequisite newly added to M1. A
realistic request-processing benchmark is a useful follow-on after acceptance;
it must use supported Holo operations and disclose unavailable network/storage
integration instead of substituting a host kernel.

### M1 acceptance decision — November 10

**Accepted: the finite, experimental, single-threaded checked-region core under
`m1-darwin-arm64-llvm22.1.2-platform-v1`.** The approved trap/runtime policy is now
implemented. The [complete acceptance record](../evidence/native-contract-acceptance-20261110/README.md)
preserves source/tool identities, positive/negative profile checks, native results,
independent replay and the final gate disposition. The implementation remains an
experimental subset; this decision neither stabilizes public APIs nor certifies
all native builds, compiler versions, runtime images or targets.

Only two production inputs differ from the preceding sealed control:
`runtime/llvm/prelude.ll` changes assertion failure from libc abort to LLVM's
target trap, and `compiler/src/driver.rs` requests eager chained imports on Darwin
arm64. The other44 compiler/CLI/runtime/workspace inputs are byte-identical.
Parsing, access/lifetime checking, representation planning, producer/storage
lowering and cleanup algorithms retain their prior source identity. New tests and
diagnostic admission/profile code are recorded separately.

| Gate | Evidence and disposition |
|---|---|
| Source/checker/IR and native correctness | All165 offline workspace tests pass; workspace formatting and warnings-denied Clippy pass. Existing positive/negative lifetime, alias, initialization, destination, reset/growth and failure coverage remains. The complete language is not implemented. |
| Flat storage, compact representation, copies and growth | Earlier independently accounted fresh/input-copy paths and retained-reference/layout evidence remain scoped to their actual inputs. The responsible checker/layout/producer source is unchanged; no new relocation, per-object free, payload staging or mandatory per-call profile work is introduced. General aggregate/error transfer stays unsupported. |
| Fatal generated checks | Source/runtime guards still protect invalid accesses. Original LLVM calls the target trap, followed by unreachable; native BRK/UD2 and exact target signals are tested. No Holo formatting, libc abort, cleanup or callback precedes transfer. ASan and selected-guard witnesses are separate correctness diagnostics, not certified production helper profiles. |
| Native helper profile | The fresh8-case ×3-layout ×O0/O3 matrix passes48 ordinary native outcomes and6 selected-guard outcomes. All48 linked inventories and eager bindings are replayed. Reviewed memcpy178/memset69/bzero50 instruction graphs, tool hashes, platform image UUID, hardware zeroing capability, default binding and valid-memory assumptions are explicit. Unknown graph/tool/runtime/helper, lazy/weak/non-system bindings and loader/sanitizer overrides reject profile admission. This is explicit trusted implementation acceptance, not a universal per-build effect proof. |
| First-call behavior | Actual linker probes exposed lazy binding. The driver now requests `-Wl,-fixup_chains` for Darwin arm64, with actual strong eager libSystem chained imports required by the profile. Resolution moves to startup before contracted calls; startup/loader work is not claimed allocation-free or free of blocking. Launch cost and file-size effects are unmeasured. |
| Bulk release, rollback and cleanup | The sealed full cleanup matrix retains126 controls,252 native executions and45 independent model outcomes. A fresh selected refresh adds24 O0/O3 controls,48 ordinary/observed executions,15 model outcomes and12 allocator-observer calibrations across explicit reset, deep rollback, three-owner exit and acquisition recovery. It supplements the full record rather than replacing it. Backing frees remain separate from logical marker reset. |
| Finite backing and recovery | M1-R3's checked bounded allocation policy, exhaustion, repeated refill/reuse and acquisition recovery keep their retained evidence. VM growth/trim, genuine pressure and asynchronous resource cleanup are not newly implemented. |
| Measurement/reporting and comparison provenance | Required separate release/copy/failure and memory investigations were performed and their limits retained. Existing C/Rust/Goose comparisons remain historical results for their actual compiler/flags. No new language timing campaign ran in this work and no old timing is relabeled as current. |

The composite gate is closed by **reviewing the finite evidence and its limits**,
not by supplying invented numbers. Physical reset latency remains **unresolved**
when clock resolution and eliminated/moved private stores prevent isolation.
Generic committed bytes, genuine OS-pressure behavior and trim/recommit remain
**unavailable or unsupported**. Existing backing-request/usable/mapped/resident
quantities stay separate. Broad owned errors, mid-byte recovery, rare/mixed
failure rates and universal physical memory traffic are not established. Earlier
Goose source-to-compiler correspondence and allocator-environment omissions retain
their documented attribution limits. None of these is labeled a measured pass,
Goose parity, a latency bound, or a whole-language performance result.

The native profile is enforced by explicit acceptance orchestration and its
trusted obligations; it is not an always-on universal CLI verifier. Ordinary
experimental compilation on other targets is not certified by this decision.
Future operation/backend/runtime changes must establish their own compatible
effects or remain unestablished. General effect exports, foreign calls/callbacks,
automatic stack assignment, pools, queues/images, richer layouts and actual
network/NVMe/async integration remain deferred under the mechanism checklist.

This completes the user-directed current work before returning to comparisons.
M2 should select its next change from measured evidence with current source/tool
correspondence, tuned controls and the same correctness obligations.

### User-directed source tooling — November 10

After finite M1 acceptance, the user requested Rust source tools, terminal
presentation and a TypeScript VS Code integration before the next storage
comparison. Shared lossless scanning/highlighting now lives in `compiler/src/`;
CLI adapters provide `highlight` with optional filename header, line numbers,
compact frame and compiled CSS-like palette, plus `fmt` stdout/check/explicit
replacement modes. The formatter protects token/comment spelling and parsed
structure and remains limited to the experimental grammar. Editor highlighting
uses a separately tested TextMate grammar; format-on-save calls the same CLI
engine and an optional theme derives from the terminal palette. No LSP, semantic
tokens, source-language expansion or native storage optimization is claimed.

The [source-tools record](../evidence/source-tools-20261110/README.md) retains
correctness checks and separate scanner/classifier versus CLI-rendering timings.
These host-tool measurements are not Holo-generated program performance or a
cross-language comparison. The Goose mechanism checklist and accepted M1 native
profile are unchanged; no new storage/representation mechanism is introduced.
Existing native regression gates must still pass after shared scanner reuse.

### Fresh matched scorecard — November 10

**Priorities 1 and 2 are complete; optimization was deferred at this stage.** The
[archived scorecard](../evidence/native-scorecard-20261110/README.md) preserves
[lifecycle results](../evidence/native-scorecard-20261110/lifecycle/report.json),
[existing-input results](../evidence/native-scorecard-20261110/input/report.json),
the freshly rebuilt [Goose control](../evidence/native-scorecard-20261110/controls/),
and [independent review](../evidence/native-scorecard-20261110/review/independent-analysis/analysis.md).
The review replays complete schedules, mathematical output oracles, raw streams,
reported statistics, binary hashes and captured source identities without rerunning
native work. No sample or unfavorable result is excluded.

| Campaign | Matched work and coverage |
|---|---|
| Construction/scan lifecycle | 512, 65,536 and 1,048,576 records; three Holo layouts, matched flat C/Rust layouts and natural compact Goose; combined and diagnostic-phase protocols. All 60 controls have 16 measured samples and one preflight: 960 measured observations plus 60 preflights. Every combined/phase pair has exactly eight runs in each order. |
| Existing-input copy | 31, 257 and 4,096 payload bytes; all three Holo layouts, C/Rust known-length spans and complete-record input views, and natural compact Goose. All 48 controls have 15 measured samples and one preflight: 720 measured observations plus 48 preflights. Full rows are shuffled in a reproducible order. |

All 108 native controls use O2/generic comparison settings, SDK 15.5 and a checked
macOS 11.0 deployment minimum. C, Holo and Goose use Clang/LLVM 22.1.2; Rust 1.92
uses LLVM 21.1.3. The real local modified Goose checkout is freshly rebuilt with
verified compiler/runtime/stdlib correspondence, rather than assuming that an
older executable matches current source. These new controls do not retroactively
remove attribution limits from earlier campaigns. Safety, representation, backing
and exhaustion differences remain disclosed.

This work changes no native engine or optimizer. Unrelated source-tool/CLI and
shared lexer changes were already present before both preparations, whose compiler
input identities match each other. The [control verification](../evidence/native-scorecard-20261110/review/controls-verified.json)
compares all 27 actual benchmark LLVM modules with the accepted M1 compiler:
every module is byte-identical. The whole compiler source and executable are
different; module equality is the narrower evidence for this workload.

The combined lifecycle medians put default 16-byte-prefix Holo at 8.44–8.56 ns
per complete record, with cost 7.70–11.19% above the natural 12-byte-prefix Goose
control, 3.46–5.82% above matched C and 2.36–3.92% above matched Rust. Compact
12-byte-prefix Holo does not close that time gap. At the largest count, compact
Holo and Goose both use a 27.5 MiB logical image, versus default Holo's 31.5 MiB;
logical size, backing capacity and peak process RSS remain separate quantities.
The diagnostic phase medians nominate construction for investigation, while their
extra clocks prevent treating the phase split as a causal explanation.

For 31-byte input, Holo is close to C/Rust complete-record views but materially
behind their stronger known-length spans. The 257-byte results overlap broadly;
the 4,096-byte results also retain substantial spread and drift. These are complete
copy/build/scan/reset intervals over reused input, not isolated copy bandwidth or
network/storage latency. Full distributions and all controls stay in the record;
whole-run quantiles from 15 or 16 samples are not operation-latency tails.

The evidence establishes neither a global language ranking nor a new accepted
semantic rule. Physical reset latency, generic committed bytes, hardware-counter
causality and actual async/network/NVMe integration remain unresolved or outside
this campaign. Any next optimization must preserve the accepted M1 profile and
be evaluated against these unchanged controls, including the stronger spans.

### Construction-success branch-layout trial — November 10

**Decision: not adopted.** The user authorized investigating the construction gap,
testing one general improvement and comparing it with the sealed baseline. The
[complete trial](../evidence/construction-branch-20261110/README.md) preserves the
candidate, original binaries, favorable results, repeated regression and removal.
This completes that experiment; it does not complete M2 or change an accepted
language rule.

The candidate attached fixed 2000:1 LLVM branch weights to existing compiler-
generated construction overflow, capacity, narrow-width, nested-status and
append-publication decisions. These were layout heuristics, not measured
probabilities or semantic assumptions. Predicates, successors, field order,
statuses, private construction and rollback stayed intact. Explicit `FailIf`,
initializer loops, assertions and user control flow stayed unweighted. No check,
`assume`, `expect`, alias promise, new helper or alternate reservation path was
introduced. Across 115 actual before/after emitted modules, removing only the
new metadata attachments/definitions yields identical original LLVM text.

| Campaign | Complete measured coverage | Preflights |
|---|---:|---:|
| Three sizes/layouts, combined and diagnostic lifecycle, real C/Rust/local Goose and preserved Holo controls | 1,248 | 78 |
| Ordinary successful and frequent-failure attempt/retry/consume sequences | 2,688 | 168 |
| Complete input-copy/build/scan/reset comparison, all layouts and retained strong foreign controls | 910 | 65 |
| Exact-original-binary repeat of all 13 Holo copy pairs | 364 | 26 |
| **Total** | **5,210** | **337** |

The [lifecycle/failure replay](../evidence/construction-branch-20261110/record/independent-analysis/replay-v1/analysis.md)
and [copy/repeat replay](../evidence/construction-branch-20261110/record/independent-analysis/copy-replay-v1/analysis.md)
independently check the complete raw outcomes, schedules, identities and reported
distributions without rerunning native work. Before/after Holo pairs use the same
logical source, support and native settings; real competitor differences retain
their disclosed safety/backing/representation limits. There is no Git main here,
so the captured unchanged compiler is the causal baseline. No sample is excluded.

Every one of the nine combined lifecycle paired medians improves, by 2.04–3.65%.
Default Derived64 cost changes are **−3.09%, −3.06%, −3.65%** at 512, 65,536 and
1,048,576 records. Its separately instrumented build-plus-boundary changes are
−10.23%, −10.15%, −9.29%. Inspection observes changed branch placement with the
guards still present. This supports a local code-layout opportunity; phase clocks
perturb optimization/execution and do not prove isolated instruction costs or
that branch placement explains the full original Goose gap. Failure-stream
changes include complete attempts, conditional retry and consumption, not isolated
error latency.

The wider copy controls prevent adopting those wins alone. Wide 4,096-byte
copy/build/scan/reset has a paired cost regression of **+2.24%**, middle50%
**[+1.34%, +3.01%]**, slower in 13/14 pairs. An exact-binary repeat gives
**+2.49% [+1.49%, +3.26%]**, slower in all 14 pairs, in both launch orders.
The initially adverse default 31-byte result does not repeat: its paired median
changes from about +1.07% to −0.13%, with the repeat IQR crossing zero. All 13
copy pairs, full ranges, order strata and whole-run mean/quantile deltas remain
visible; a reproducible wide regression is not concealed by selecting another
format or a favorable phase.

After that trial the five changed Rust files were restored, and all 52 compiler-
input hashes matched the pretrial baseline. No broad branch-weight flag, layout-
specific exception or second lowering path remained. Candidate correctness checks
and final restoration
verification are separately recorded in the archive. Quantiles from 14/16
whole-run observations are not request tails or a statistical significance result;
shared-host drift, unresolved physical reset latency and commitment limits remain.
The trial changes no flatness, stable growth, compactness, failure, ownership or
bulk-reclamation rule. General queues/images, pools, automatic data stacks and
network/NVMe/async integration remain outside this experiment.

### Narrow construction and vector follow-up — November 10

**Decision: accept narrow scalar-input construction hints as a finite compiler
improvement.** The user authorized investigating the copy spill/reload
tradeoff, testing an exact byte-vector prefix while retaining scalar semantics,
and making lifecycle/copy/failure comparisons routine. The
[follow-up archive](../evidence/construction-followup-20261110/README.md) preserves
both candidates independently, their screenings and full campaigns, code review,
tests and the final compiler selection. The broad rejected heuristic above is
not re-enabled. No public syntax, ownership, failure or layout rule changes.

The selected heuristic adds fixed 2000:1 success-layout metadata only inside
producers whose checked signatures contain solely U64 inputs and at appends to
those producers. In this checked subset, producer byte arguments can only forward
Bytes formals; direct published-field captures belong to entry code. An all-U64
producer therefore cannot introduce a transitive borrowed-payload read. Even an
unused Bytes formal conservatively excludes that producer. Entry append decisions
are callee-specific, so alternating fresh/copy calls cannot inherit stale scope.
Explicit rejection, initializer loops and user branches remain ordinary. This is
an optimizer heuristic, not a probability, noalias assertion or safety assumption;
every check and the original field/error/publication/rollback order remains.

| Campaign, per independently tested candidate | Measured | Preflights |
|---|---:|---:|
| Complete input screening, including all thirteen Holo pairs and strong real references | 910 | 65 |
| Full lifecycle, input and frequent-failure acceptance matrix | 4,846 | 311 |
| **Both candidates, including both screenings** | **11,512** | **752** |

The full matrix contains 1,248 lifecycle observations with 78 preflights, 910 input
observations with 65 preflights and 2,688 ordinary success/failure-retry observations
with 168 preflights. Native competitors run sequentially after preparation;
lifecycle/failure pairs use 16 observations and balanced 8/8 directions, input pairs
14 and 7/7. Captured original Holo supplies the causal before/after comparison;
real C/Rust/local modified Goose controls keep their sealed code/source/settings.
The two optimization candidates are each compared with that original compiler,
not with each other or an unmeasured combination. Historical timings are not reused.
Independent replay retains the complete native witnesses, schedules, identities
and reported statistics for the
[narrow lifecycle/failure matrix](../evidence/construction-followup-20261110/record/independent-analysis/narrow-life-failure/analysis.md),
[narrow copying](../evidence/construction-followup-20261110/record/independent-analysis/narrow-copy/analysis.md),
[vector lifecycle/failure matrix](../evidence/construction-followup-20261110/record/independent-analysis/vector-life-failure/analysis.md)
and [vector copying](../evidence/construction-followup-20261110/record/independent-analysis/vector-copy/analysis.md).
Both independent pilot replays remain separately recorded, rather than pooled
into the full matrix's distributions.

For the narrow candidate, default Derived64 combined paired cost changes at 512,
65,536 and 1,048,576 records are **−3.22%, −3.05%, −3.47%**. Eight of nine combined
lifecycle middle-50% intervals are wholly below zero; wide 65,536 remains uncertain
at −0.46% with an interval crossing zero. All thirteen full-copy intervals cross
zero, and ten paired copy executables are byte-identical to the original compiler's
executables. The prior wide 4KiB copy regression does not recur in these controls.
One frequent-failure row has a descriptively adverse +0.60% interval but also
byte-identical before/after native files; it is preserved and cannot establish a
compiler-generated regression. This scoped result is not a claim of zero noise,
universal speedup or statistical significance.

The separate vector trial uses exact eight-byte i8 stores only for closed
Const/Param/Index wrapping Add/Sub/Mul/Xor/And trees after original reservations.
Applications and memory observations keep scalar lowering. V1 is an untimed
code-generation/190-test probe. V2 adds an equal-value private-counter `index=end`
store after the prefix, preserving its mathematical exit value without an
`assume`, arithmetic flag or branch hint. Seven focused O0/O3/ASan tests validate
v2 before timing. Original full-u64 scalar expression source remains unchanged,
but actual v2 scalar machine bodies have five instructions instead of four and
retain an unreachable eight-byte u64 vector fallback. Source-level preservation
therefore does not establish native-tail equivalence or a speed advantage.

V2 completes the full measured matrix with valid outputs but gives less consistent
default/compact lifecycle benefits than the narrow candidate and eight descriptively adverse
review flags across input/failure rows. Those flags are not reported as reproduced
regressions. V2 is **not adopted**; its complete source, binary and unmeasured v1
preparations remain in evidence. No active vector module or combined hint/vector
policy remains. Boundary and mixed-helper semantic regressions stay maintained.

The [maintained optimization gate](../benchmarks/region-lifecycle/optimization-gate/README.md)
now makes these controls one routine prepare/measure/report workflow. Screenings
cannot stand in for the full gate. Reports retain full mean/quantile/extreme,
resource, direction and paired distributions and flag adverse middle intervals;
they never automatically approve a candidate. Reproducible material regressions
still require a decision or rejection. The finite M1 mechanism ledger is unchanged:
flatness, stable growth, compactness, destination construction and bulk markers
retain their existing rules/evidence; queues/images, pools, new backing policies
and network/NVMe/async features remain deferred. Final formatting, strict workspace/
all-target Clippy and all 191 workspace tests pass. The active 52 source-input
hashes and rebuilt release executable equal the measured narrow candidate, and
the vector module is absent. These exact identities and command streams remain
in the archive's final validation, rather than an assertion that every target or
future program benefits.

### M2 — first finite compiler improvement accepted

- [x] Select the narrowly scoped native branch-layout heuristic from measured evidence.
- [x] Preserve the M1 semantic tests and compare both candidate mechanisms independently
  against the captured original compiler and unchanged real reference controls.
- [x] Record favorable, uncertain and adverse results, retain the unadopted vector
  alternative and update implementation status without promoting a public rule.

M0 is not a language victory, and M1 is not a complete systems language. Each is a
reviewable gate that makes the next design choice more informed. Do not expand
the compiler merely to accumulate unmeasured syntax or unsupported guarantees.

### In-memory ingestion follow-on — November 10

The next authorized workload is a maintained
[preframed-event ingestion campaign](../benchmarks/ingestion/README.md), using the
accepted compiler without another optimizer or language change. Each cycle reads
256 complete packed input records, checks available header size and magic, decodes
a two-byte little-endian payload length, rejects out-of-frame lengths, filters an
application flag and constructs accepted owned nested records through ordinary
calls. A full id/byte checksum consumes output before one scoped marker reset.
Warmup checks exact output order, ids, lengths, every byte, category counts and
logical image sizes outside timing. The mathematical host oracle independently
constructs and decodes actual wire bytes.

Short, medium and larger profiles permit payloads up to 63, 511 and 4,095 bytes.
Each deterministic 256-frame cycle contains 168 accepted events, 8 bad-magic
frames, 8 truncated headers, 8 oversized lengths and 64 filtered events. Short inputs
include an accepted zero-length payload. These structured, correlated distributions
are test fixtures rather than sampled production traffic. No live acquisition,
general streaming frame extraction, network/NVMe, concurrency or async I/O is added.
Input/output allocation, source initialization, warmup, exact validation and final
backing release remain outside native timing. Marker reuse retains backing.

All three Holo prefix policies have matched complete-record input and output
arena controls in actual C and Rust; Goose supplies its actual compact form.
Source metadata is loaded normally, not made volatile. C/Rust fuse output
reservation; Holo retains its checked component/bounds work. Goose uses data
stacks and abort-on-exhaustion rather than Holo's finite append statuses. Different
alias/checking/backend rules remain explicit comparison limits. Owned copying is
required work, not zero-copy transport or demonstrated physical memory traffic.

The Goose checklist keeps its scoped M1 status: flat nested destination storage,
supported stable growth, checked compact metadata and fixed-marker reclamation
are exercised together by this pipeline, with byte/code inspection separate from
elapsed measurements. Automatic data-stack assignment/VM growth, general relative
links, tagged alternatives, generics/callbacks, multiversioning, queues/images and
nonnested pools stay deferred. Rejected malformed wire frames are application
classification, not evidence of new owned-error or streaming recovery semantics.
No public proposal is promoted by using the existing experimental source adapter.

The [completed record](../evidence/ingestion-20261110/README.md) retains all 30
controls, 360 measured observations, 30 preflights and complete independent replay.
Compact input rates in millions of frames/second are Holo/C/Rust/Goose:
short 91.15/93.92/92.60/122.43, medium 71.84/67.88/71.66/72.58, and larger
8.92/8.75/8.87/8.82. Short Goose throughput is about 34% above Holo compact;
larger full ranges overlap. Small median differences are not established wins.
All wide/default/compact rows and unfavorable observations remain visible.

Native inspection demonstrates that LLVM already proves the validated copy
subrange and emits `memcpy`, with NEON checksum reductions in all four controls.
Holo retains two component capacity checks; C/Rust fuse reservation, while Goose
has inline short-copy paths and a different exhaustion policy. These are profiling
hypotheses, not causal timing attribution. A premeasurement Rust counter correction
removes unnecessary overflow panic checks using explicit wrapping updates while
preserving capacity, extent and slice safety. Superseded preparations remain
unmeasured, and the unchanged Holo compiler receives no credit for that harness fix.

Twenty-three host checks pass. The selected 45 O0/sanitizer diagnostics cover all
nine profile/layout combinations; nine superseded Rust diagnostics are also kept,
all passing. Formatting and strict Clippy pass. The 52 compiler input hashes and
actual CLI equal the previously accepted narrow candidate; a fresh full compiler
test run is not claimed for benchmark-only changes. Graphing, analysis and builds
occur outside the serial native window. Process resource totals, launch-to-reap
polling overhead, shared-host activity and hot correlated inputs stay explicit.

### Short-message ingestion profile — November 10

The sealed compact binaries repeat a 2.45 ns/input short-message gap between
Holo and local Goose. The corresponding paired medians are +0.18 ns/input for
medium and −0.72 for large, with the large middle 50% crossing zero. Separate
short diagnostics put the larger difference in the ingest path (+2.43 ns/input)
and a smaller difference in the scan path (+0.22 ns per equivalent input).

Holo's timed path calls libc `memcpy` for 164 nonempty payloads per short batch.
Goose uses inline copies for all 164. A guarded copy replacement at the Holo
assembly site reduces median paired cost by 2.04 ns/input. Forcing the Goose
generated-C site through guarded libc copying increases it by 2.05 ns/input.
Both unmodified relinks are byte-identical to the original controls. These
reciprocal diagnostics support a general checked small-copy experiment. They
leave the cost of Holo's two reservation checks unresolved. The inline variant
in that record edits one ARM64 copy site. [Timing, native evidence and profile](../evidence/ingestion-short-profile-20261110/README.md)

### Checked short subrange copies — November 10

The production macOS arm64 policy handles eligible nonzero constant-offset byte
initializers with bounded inline transfers through 64 bytes. Larger valid ranges
take the bulk intrinsic, and a failed range proof preserves the checked loop. The
check runs after the original reservation and zero-length behavior is preserved.
The policy selects automatically and `inspect-copies` reports the decision.

Two same-source compiler campaigns each compare 39 controls. The selected
head-and-tail version lowers short paired cost by 19.1–19.9% across the three
layouts. With matching compact representations, the median Holo/Goose input rates
are 130.29/134.22 million frames/s for short, 76.65/77.07 for medium, and
9.026/9.034 for large. The comparison uses the verified local Goose executable.
Upstream `f4637bf` keeps the same copy helper and has a different array length
encoding. A repeat balances early and late positions in four-launch blocks.
The large paired median changes stay below +0.3% and each middle interval
crosses zero. The existing 115 lifecycle, input, and failure controls reproduce the
same LLVM apart from inspection comments. All scoped native semantics and the
workspace suite pass. [Complete result, variability and upstream check](../evidence/small-copy-followup-20261110/README.md)

### Mixed-lifetime telemetry worker — November 10

The [telemetry worker](../benchmarks/telemetry/README.md) keeps scalar service
totals alive across repeated validation, batch discard and reuse. The direct
form keeps only a local tally during validation. The staged form writes six
owned bytes into each packed destination and consumes the admitted records in
a second pass. Both exercise the same explicit admission failure and retry.
This uses existing bounded regions, compact layouts, destination construction
and lexical marker reset. No language or compiler behavior changed.

The longer four-language run gives Holo-new 550.75 million input records/s
direct and 364.37 staged. Local Goose reaches 571.22 direct and 675.26 staged.
Holo trails the tuned C and Rust controls in both variants. Both full campaigns
retain all 192 timed observations and their 16 preflights. Generated assembly
shows inline payload copies and an actual staged consumer in both Holo and
Goose. Holo retains reservation branches and uses a different tally lowering.
Those observations warrant targeted attribution before another copy or
reservation optimization. [Full comparisons and validation](../evidence/telemetry-20261110/README.md)

### Telemetry phase attribution, chunked input, and mixed sizes — November 10

The fixed six-byte [phase diagnostic](../benchmarks/telemetry/phases/README.md)
reruns exact staged binaries, adds separately calibrated clocks, and tests a
Goose scalar-tally alternative with the staged kind guard retained. The paired
Holo-minus-Goose gap is about 0.87 ns/input in the clocked classification and
append window and 0.14 ns/input in the output observation and scan window.
Clock effects remain visible. The uninstrumented tally intervention costs
about 0.40 ns/input in Goose. It changes the counter representation and loop
shape together. Neither result isolates the cost of individual append guards.

The [chunked input slice](../benchmarks/telemetry/chunked/README.md) preserves
only scalar framing state between chunks and commits counters at complete-frame
boundaries. It covers bytewise splits, partial input, malformed kinds and magic,
and bounded rejection through the declared body. This is native correctness
evidence for preloaded RAM chunks. External input and reusable caller-fed chunk
ownership remain outside the experimental source interface.

Two matched [telemetry profiles](../benchmarks/telemetry/README.md) add empty,
short, and bulk payloads through 256 bytes. The balanced and hit-heavy profiles
share payload sizes, bytes, and the malformed schedule. Staged Holo medians
are 126.69 and 129.62 million input records/s, compared with local Goose at
130.15 and 133.57. Holo leads the C and Rust controls in these two staged runs.
The short-record fixed-profile gap remains reproducible. No compiler behavior
changed. [Native measurements and validation](../evidence/telemetry-followup-20261110/README.md)

### Proved reservation, reusable chunks, and copy-boundary sweep — November 10

The [reservation diagnostic](../benchmarks/telemetry/reservation/README.md)
specializes one reviewed six-byte staged append. A bound on the entire
admitted batch permits removal of seven instructions, including three native
reservation branches, from the admitted append. The original checked producer
handles failed proofs. Real-buffer probes exercise exact fit, private partial
writes, failure ordering, and marker restoration. The paired time change is
−0.001 ns/input, with the middle half spanning −0.024 to +0.019. This experiment
does not support adopting a production optimization for the observed gap.

The [chunked decoder](../benchmarks/telemetry/chunked/README.md) also accepts a
single caller-owned slot in its checked entry body. The caller reuses the same
address and overwrites the entire slot as soon as each lexical read ends. Both
input modes match the independent framing oracle under all native profiles.
Seven negative checks reject an expired chunk binding. General borrowed-buffer
functions and external acquisition remain outside this slice.

The staged four-language comparison uses 63-, 64-, and 65-byte payloads over
either one packed batch or 4,096 sequentially rotated batches. The larger
corpus reaches 17.53 million packed source bytes. All four have lower rates at
63 than 64 in both corpus sizes. Holo's generated scan uses 32-byte vector
blocks, an eight-byte vector epilogue, and a scalar tail. The 63-byte length
uses both tails while 64 uses two main blocks. The 65-byte case also crosses
the current Holo inline-copy cutoff. The sweep changes both consumption and
copying, so it does not isolate either cause. An interleaved repeat retains the
same patterns. [Campaign results and limitations](../evidence/telemetry-boundary-20261110/README.md)

### Isolated scan and copy policy, borrowed-call design — November 10

The [scan-only diagnostic](../benchmarks/telemetry/scan/README.md) prebuilds and
materializes every owned payload before timing. Hot vector scans take
13.206, 6.461, and 6.781 ns per consumed record at 63, 64, and 65 bytes.
The matching scalar controls take 19.184, 19.453, and 20.344. The cliff remains
without timed payload construction or copying and mostly disappears under
scalar code generation. Vectorization remains faster at every tested size.
Costs normalize 52 scanned records per group, a distinct denominator from the
end-to-end fixture's 64 input records.

The [copy-policy diagnostic](../benchmarks/telemetry/copy-boundary/README.md)
preserves the sealed classifier, reservations, source checks and full consumer
while extending only the already proved 65–128-byte arm. Paired median cost
falls 8.63% for hot 65-byte input, 6.46% for its rotating corpus, 3.78% for the
mixed profile, and 2.50% for hit-heavy. The wider arm uses two overlapping
64-byte windows and retains bulk fallback above 128. This experiment does not
adopt a compiler policy. [Measurements, checks, and scope](../evidence/telemetry-isolation-20261110/README.md)

The [synchronous borrowed-buffer function proposal](spec.md#synchronous-borrowed-buffer-functions)
uses the existing `readwrite reference` state and `readonly reference` byte
modes. A proposed `core.prefix` actual captures the received logical bound.
It preserves M1 aliases and effect inference while separating ordinary mutable
calls from destination producers. The subsection records acceptance cases and
review choices. The implementation and native acceptance follow below.

### Copy regression review, bounded byte sums, and borrowed calls — November 10

The 128-byte copy candidate retained exact LLVM for all 115 original gate rows
and reduced elapsed cost by 5.24–12.84% in the new fixed 65–128-byte cases. At
129 bytes, the wide layout regressed 2.06% in the full campaign and 2.69% in
the exact-binary repeat. The derived layout also regressed in both campaigns.
The compiler keeps the accepted 64-byte cutoff. The extra comparison before
bulk copy and its code placement effects remain measurable candidates for later
work. [Full gate and correctness record](../evidence/copy128-regression-20261110/README.md)

The [bounded byte-sum prototype](../benchmarks/telemetry/byte-sum/README.md)
replaces an audited scanner reduction with 16-byte partial sums and guarded
8/4/2/1-byte tails. It preserves exact initialized loads and modulo-u64 results.
All 204 adjacent pairs favor the prototype on the 17 tested scanner cases.
Hot 63-, 64-, and 65-byte medians fall from 13.835, 6.578, and 6.902 to 3.363,
3.075, and 3.078 ns per scanned event. This experiment admits an exact sealed
IR shape. General compiler adoption requires checked eligibility and the broader
regression matrix. [Paired results and protected-range probes](../evidence/telemetry-byte-sum-20261110/README.md)

The approved [synchronous borrowed subset](spec.md#synchronous-borrowed-buffer-functions)
now lowers ordinary functions through a separately checked call graph. A typed
actual captures caller state or a byte pointer/extent without payload copies.
`core.prefix` traps before the next actual or callee execution if its logical
extent is invalid. The callee returns a scalar and may mutate existing fixed
state. The checker preserves compatible aliases and rejects effects or escapes
outside the subset. The chunk decoder uses this path in both input modes.
The runtime storage representation, destination construction, nonmoving growth
and marker reset requirements continue to apply. [Native acceptance](../evidence/telemetry-borrowed-calls-20261110/README.md)

### Checked byte sums, direct refill, and copy dispatch — November 10

An isolated macOS arm64 compiler now recognizes checked full-span wrapping byte
sums over published fields and bounded byte formals. The loop's exact extent
and private accumulator justify regrouping the loads. The source-native tests
exercise all three layouts with checked logical prefixes, unaligned data,
wrapping seeds, and unoptimized, optimized, and sanitized builds. The emitted
helper passes 49,644 protected-range checks over three profiles. The isolated
workspace passes formatting, Clippy, and all 209 tests.

The scanner paired median falls 49.87–77.36% across 17 cases. All 115 complete
legacy workloads also improve. Each baseline executable reproduces the sealed
native control, and all 230 baseline and candidate preflights pass the independent
oracles. The focused exact-binary repeat confirms higher cost for the wide
65,536-record build phase (+5.73%) and the derived 512-record reset phase
(+2.22%). Those phase costs keep the optimization out of the default compiler.
The comparison covers one macOS arm64 host. [Correctness, timings, and repeat](../evidence/checked-byte-sum-20261110/README.md)

The [synchronous refill proposal](spec.md#synchronous-file-and-stream-refill)
binds the bytes from a positive read to a scoped view of the original writable
slot. A short read produces data, and zero on a positive request ends the
decoding session. The caller controls retries and finalization. The planned
native adapter needs handle and result checks, and the checker needs to reject
reuse of a slot with a live received view, including through another alias.
The current decoder still uses initialized RAM, and file and stream behavior
remain acceptance work.

The upper-bound-first copy trial keeps its 65–128-byte gains and reduces the
129-byte penalty by sending bulk copies through one length comparison. The
exact-binary repeat confirms that the extra inline-path comparison comes with
higher 31-byte cost across all three layouts. Paired median increases are
1.46% wide, 0.92% derived, and 0.79% compact. The active 64-byte cutoff remains
in place. [Full and repeat measurements](../evidence/copy128-upper-first-20261110/README.md)

### Outlined byte sums and checked refill — November 10

The arm64 byte-sum implementation now runs the checked reduction in a separate
native function. The isolated trial changed one LLVM inlining attribute from
the previous candidate. Its 17 scan costs fall 43.23–73.80% relative to the
scalar compiler, and the 115-row gate improves every complete workload. Exact
repeats of the previously affected wide construction and derived reset clocks
have paired middle intervals spanning zero. A same-session three-way replay
reproduces both inlined phase costs and shows the outlined path's extra call
cost on short scans. The active compiler reproduces all 132 measured LLVM
outputs byte for byte. [Adoption, correctness, and phase gates](../evidence/outlined-byte-sum-20261110/README.md)

The isolated checked refill prototype borrows a regular file or nonblocking
stream from a trusted host. A successful attempt supplies a lexical readonly
prefix of one initialized slot. The checker rejects refilling a live view's
root and field, including through aliases, and conservatively treats different
record indices under that root and field as possibly aliasing. Positive
counts, short reads, EOF, would-block, and invalid bounds are exercised with
real descriptors and injected outcomes. The existing telemetry decoder passes
12 file and socket framing runs at O0, O3, and O3 with AddressSanitizer. The
prototype uses programmatic IR and leaves public source and handle design open.
[Native and checked-IR evidence](../evidence/checked-refill-20261110/README.md)

The fresh four-language comparison covers the complete staged telemetry worker
at hot and rotating 63-, 64-, and 65-byte payloads and a mixed direct/staged
profile. Holo leads each staged row on the measured macOS arm64 host, and
Goose leads the mixed direct row. The uninstrumented hot 63-byte Holo pair
rises from 76.35 to 213.08 million records per second in the same session.
The instrumented scan falls from 11.253 to 2.755 ns per input record, with
paired staging and commit/reset medians within 0.5% of the previous compiler.
All 468 measured children and 39 preflights pass the independent oracle.
[Results, phases, and source identities](../evidence/telemetry-outlined-20261110/README.md)

### Matched reductions and byte-sum coverage — November 10

The [tuned telemetry comparison](../evidence/tuned-telemetry-20261110/README.md)
preserves the exact ordinary controls and applies the same bounded reduction to
C and Rust using Arm NEON. The pure Goose source expresses the same arithmetic
with guarded slices and generates a different kernel. A separate Goose worker
calls the C NEON kernel through its native slice ABI. On the hot 63-byte row,
Holo reaches 213.00 million inputs per second, pure Goose reaches 158.74, and the
foreign-helper diagnostic reaches 206.00. Tuned C and Rust both lead Holo at 64
and 65 bytes. The separate 63-byte phase clocks place Holo's remaining lead over
tuned C and Rust in validation and staging. Their tuned scans take less time
than Holo's scan in that diagnostic. All 480 measured executions and 40 preflights
pass the original independent workload oracle.

The maintained differential test uses the same checked program for ordinary
and outlined lowering. It generates 212 packed records with lengths through
16,385 bytes, varying borrowed prefixes, poisoned suffixes, and wrapping seeds.
Native offset observations cover all 16 byte-alignment classes for vector-sized
owned and borrowed spans in each layout. Wide, Derived64, and Compact32 builds
at O0, O3, and O3 with AddressSanitizer match the independent `u128` oracle on
7,632 sums. The test checks both outlined call sites and the ordinary fallback.
[Differential evidence](../evidence/byte-sum-differential-20261110/README.md)

The [macOS crossover campaign](../evidence/byte-sum-crossover-20261110/README.md)
holds the prebuilt scanner input and source fixed across 75 paired cases. On
the hot Compact32 rows the outline adds 1.013, 0.287, and 0.610 ns per scanned
record at lengths zero, one, and two. It lowers median cost at three bytes and
all larger sampled lengths in that layout. Three-way exact-binary repeats retain
the signs at zero, one, and four bytes, and attribute part of the tiny-span cost
to outlining. The mixed 0–15-byte cases improve across all three layouts.
These are scanner costs on one host. The current backend selection remains
the adopted outlined path while short-span alternatives remain experiments.

The [Linux code-generation probe](../evidence/byte-sum-linux-codegen-20261110/README.md)
verifies and lowers the exact outlined helper to assembly for generic AArch64,
baseline x86-64, and x86-64-v3. The available native host is macOS arm64. Linux
server crossover timing and target acceptance remain open.

### Telemetry phase attribution and tiny-span trials — November 10

The exact hot staged worker comparison reproduces the 64- and 65-byte lead
for tuned C and Rust. At 64 bytes the clocked Holo scan costs 2.578 ns/input,
versus 2.076 in C and 2.307 in Rust. At 65 bytes Holo's scan stays near the
same cost, while Holo validation and staging rise from 2.175 to 2.987 ns/input.
The individual validation and publication clocks perturb the workers enough
that their calibrated absolute times are inconclusive. The four per-batch
clocks and exact uninstrumented workers provide the stable comparison.
[Phase and sparse-clock evidence](../evidence/telemetry-64-65-phases-20261110/README.md)

The reciprocal Holo-only copy intervention keeps the outlined scan and
checked copy proof. Forcing the 64-byte worker onto the existing bulk path
adds 0.826 ns/input in paired measurements. Moving the 65-byte worker onto
the proved inline head and tail transfers saves 0.734 ns/input. Both controls
that keep the original route have paired middle intervals spanning zero.
These results explain much of Holo's 64-to-65-byte cost step. The previous
full cutoff trials still govern the active 64-byte policy.
[Copy intervention and range probes](../evidence/telemetry-copy-attribution-20261110/README.md)

Both isolated tiny-span variants preserve the checked wrapping result. The
zero-only guard skips the outlined call on empty spans. The 0–2-byte variant
also uses exact unaligned one- and two-byte loads before falling back to the
existing helper. Both pass the randomized differential test and the guarded
native probes. All 190 scanner, input, failure, and lifecycle rows change
LLVM, so the gate reruns them against the exact outlined baseline.

Exact repeats show a 39.16% empty-scan gain for zero-only and 12.79% and 29.09%
gains at one and two bytes for the 0–2-byte path. The zero-only guard also
increases a frozen-input control's cost 4.10%. The 0–2-byte path increases the
Compact32 hot short mix 13.59% and the earlier wide build phase 6.81%. Those
reproduced costs keep both variants isolated. The adopted helper remains active.
[Correctness, full gate, and focused repeats](../evidence/tiny-byte-sum-bypass-20261110/README.md)

### Rebalanced copies, matched scans, and static tiny sums — November 10

Two [isolated copy dispatches](../evidence/copy-dispatch-recovery-20261110/README.md)
send bulk lengths out first and separate the existing small-width transfers
from the wider inline arm. Both preserve all 115 original gate emissions.
The 65–128-byte version recovers the 65-byte gain and repeats higher cost at
Wide 33 and 64 bytes. The 65–80-byte head-and-tail version reduces hot 65-byte
cost 16.07% and mixed cost 2.23% in the exact repeat. Wide 33- and 64-byte
costs increase 3.71% and 1.89%, respectively. Both retain the original range,
reservation, and invalid-source checks. The active 64-byte cutoff remains.

The [matched scan diagnostic](../evidence/matched-telemetry-scan-20261110/README.md)
holds packed Compact32 bytes, verified end, scalar tally, subtotal merge, and
bounded NEON reduction shape across Holo, C, and Rust. C and Rust validate the
immutable image before timing. In the quiet exact-binary repeat, their paired
differences from Holo at 64 and 65 bytes are at most 0.067 ns per scanned record
and all four middle intervals span zero. At 63 bytes C and Rust retain paired
leads of 0.219 and 0.176 ns per scanned record. This diagnostic jointly matches
traversal and tally, and its timing excludes staging.

The static tiny-sum candidate captures the ordinary arguments in their written
order before substituting an exact zero-, one-, or two-byte reduction. A
literal logical prefix remains subject to the original eager bounds check.
The isolated O2 benchmark reduces scan cost 44.15%, 23.60%, and 37.99% at the
three selected widths. Its literal three-byte and dynamic two-byte controls
relink byte for byte. The active release reproduces the five measured LLVM
modules and the 190 archived dynamic controls. [Isolated test and timing](../evidence/static-tiny-byte-sum-20261110/README.md) · [Active verification](../evidence/static-tiny-byte-sum-adoption-20261110/README.md)

### Hosted worker and byte library — November 10

The active experimental source frontend resolves `using holo.<leaf>` from one
explicit library root. The worker uses `holo.bytes` from
`lib/holo/bytes.holo`. Its exported low-octet join has total scalar semantics
and checked `noalloc` and `noblock` contracts. The runtime entry admits and owns
the input descriptor. Holo entry code supplies a fixed
initialized byte slot and explicitly handles retries, readiness, EOF and native
failure. A checked data arm lends only the successful read's logical prefix.

The hosted telemetry example passed 54 native file and stream sessions against
an independent framing oracle, across all three layouts at O0 and O3. All 27
O3 AddressSanitizer replays passed. For the same worker and helper in one file,
the full emitted LLVM and the O3 executable
are byte-identical in all three layouts on the measured macOS arm64 host.
The compact O3 code has no staging copy or per-refill allocation. The
already accepted non-hosted controls retain their emitted LLVM. These are
correctness and code-generation results. Native throughput and Linux execution
remain unmeasured for this slice.

Holo-owned handles, public outcome types, nested module resolution, exported
type identity, and published dependencies remain open.
[Worker and verification guide](../tests/fixtures/telemetry_worker/README.md) ·
[Native and compiler evidence](../evidence/telemetry-worker-library-20261110/README.md)

### Gzip fixed-header inspector — November 10

The gzip inspector uses `holo.bytes` for both halves of an unsigned 32-bit
timestamp. The hosted entry requests only the remaining bytes of the ten-byte
prefix, and the checked decoder retains scalar state across short reads.
The example exposes malformed and truncated prefixes as distinct outcomes.
Optional-field indicators are reported as raw flags. Full gzip validation and
decompression are outside the inspector's scope.

The independent oracle passed 144 file and socket cases across three layouts
at O0/O3 and 24 Compact32 O3 AddressSanitizer cases on macOS arm64. A stream
test supplies one octet at a time, waits for each fragment to drain, and
verifies that a queued suffix remains unread while the peer stays open.
Emitted LLVM and O3 executables are byte-identical to the single-file control
in every layout. These are correctness and code-generation results.
[Example guide](../tests/fixtures/gzip_header/README.md) ·
[Native evidence](../evidence/gzip-header-20261110/README.md)

### Local packages and locked telemetry — November 11

The hosted telemetry worker uses a manifest-driven package. Its frame
namespace spans two files and shares a private signature helper. The wire
namespace selects `holo.bytes` from the local `holo` dependency. Package checks
exercise file-scoped `using` directives, visibility, namespace and provider
conflicts, cycles, and locked builds that reject modified dependency inputs. The example's lock
identifies the exact local standard-library version and source fingerprint.

The packaged worker emits byte-identical LLVM to the same authored code in
one file in all three layouts. The worker and the compatibility-profile gzip
example passed 198 native cases at O0 and O3. Their emitted LLVM and native
executables match the preceding evidence. The existing 195 archived compiler
controls also re-emit exactly. These checks measure code and behavior equality.
[Worker guide](../tests/fixtures/telemetry_worker/README.md) ·
[Verification record](../evidence/local-packages-20261111/README.md)

### Library-only package checking — November 11

`holo check --package` now checks a manifest without an application entry.
The frontend discovers every namespace owned by the root library, follows
reachable namespace dependencies, and validates all selected declarations through
the checked IR path. It creates no executable entry or native artifact.
The shipped standard library checks with LLVM unavailable. An uncalled private
ordinary function with a recursive call fails in the common checker.

The telemetry worker and gzip inspector passed all 198 native regression cases
with six identical LLVM modules and twelve identical binaries. The active
compiler also reproduced the 195 sealed LLVM controls exactly.
[Library package rules](packages.md#checking-a-library-package) ·
[Verification record](../evidence/library-package-check-20261111/README.md)

### Public types across packages — November 11

The [working proposal](packages.md#public-types-across-packages) assigns
declaration identity from the resolved package, namespace, and type name.
The package experiment implements public fixed `u64` records, destination
constructors, and qualified batch schema selection. Package and flat examples
produce identical record sizes, field offsets, emitted LLVM, and O0/O3 native
executables across Wide, Derived64, and Compact32. Native rollback, retry,
capacity exhaustion, and retained references pass in all three layouts. Negative
cases reject equal-shaped but distinct schemas, private types in public
constructor signatures, unauthorized field writes, and resets with live
references. [Package conformance tests](../tests/compiler/packages/packages.rs) ·
[Verification record](../evidence/public-records-20261111/README.md)

Foreign public record types in signatures and nested or byte-bearing public
records need separate review.
[Design controls](../evidence/public-type-design-20261111/README.md)

### Payload-free typed errors — November 11

The experimental compiler implements ordinary `Result<u64, E>` returns,
nominal public and private tag-only families, exhaustive `on … then` arms,
error fallback, and exact-result forwarding. The
[package tests](../tests/compiler/packages/packages.rs) exercise declared error families across
package boundaries. The checked IR and native lowering use one ordinary
call path with a private `{ i64, i8 }` return for fallible functions. The
[design record](../evidence/typed-error-design-20261111/README.md)
retains the earlier runnable numeric Holo control and standalone ABI witness.

The scalar conformance checks cover entryless package validation, family
visibility, full-width success, and exactly-once handling through the native
backend. The [scalar comparison](../evidence/scalar-errors-20261111/README.md)
records four-language throughput, batch latencies, and exact-child resources
for three failure profiles. The measured Holo mixed loop retains additional
conditional control flow and falls behind the other three controls. The
follow-up [runtime-input comparison](../evidence/scalar-error-runtime-20261111/README.md)
prepares its corpus outside timing, keeps one binary per implementation, and
adds numeric Holo. The twofold gap does not recur. The select candidate takes
3.44% more paired elapsed time in the mixed profile and 3.49% more in the
exact-binary repeat, so the switch lowering remains active. The ordinary
result implementation now also supports one `u64` case payload and
same-family `let value = try call(...)`. The numeric `core.append` path remains
available alongside standalone typed append. The
[typed append fixture](error-handling.md#typed-append-acceptance-fixture)
checks publication, rollback, full-width payload survival, and retry across
Wide, Derived64, and Compact32 at O0, O3, and with ASan. The Derived64 boundary
also distinguishes exhaustion before the child from rejection before the
child's own reservation. The [byte-bearing frame example](../tests/fixtures/typed_frame.holo)
reuses final-slot range initialization and tests a copied truncation extent
after rollback and successful retry. The
[hosted frame example](../tests/fixtures/typed_refill_frame.holo) passes the live
refill data name directly to a producer. Its file and nonblocking stream
witnesses cover exact received extent, stale tail, rollback, and payload
ownership after reuse. The package experiment supports
`Result<Record, E>` destination constructors for fixed and byte-bearing
namespace-local schemas with public nominal components for exports.
Locked-package tests compare package and standalone LLVM and exercise
publication, rejection, exhaustion, and retry at O0, O3, and under ASan.
The byte-bearing fixture also checks nested rollback and ownership of the
published payload after the source changes. General aggregate values and
foreign signature types remain open. [Conformance fixtures](../tests/compiler/packages/packages.rs)

The locked hosted-refill fixture passes the actual fresh read through the
packaged decoder. Real files and nonblocking streams exercise short-read
truncation, a capacity failure after private writes, retry into the same
suffix, and unchanged published bytes and address after refill-buffer
reuse. Package and flat LLVM match in all three layouts.
[Hosted package acceptance](../evidence/stable-staging-slots-20261111/README.md#native-conformance-and-remaining-platform-evidence)

The [fixed-record append comparison](../evidence/typed-append-20261111/README.md)
records a repeatable 3.5% success-path cost against matched numeric append.
Nested rejection elapsed time is tied within measurement variation. Both
typed loops retire more instructions. The subsequent direct publication-edge
trial changes the emitted LLVM, but Clang produces byte-identical native
binaries on four runtime profiles. The active append lowering remains the
shared join. [Dispatch trials](../evidence/error-edges-20261111/README.md)

The scalar return-edge trial removes the intermediate tag for a bounded
checked guard chain with copied scalar inputs and an immediate handler. In
the repeat, paired elapsed time falls by 3.80% on all-success, 3.35% on rare
failure, and 4.05% on mixed inputs. The optimized scan matches the numeric
instruction sequence. This lowering is active for the eligible slice, and
the ordinary result ABI remains available for general calls and propagation.
[Scalar results](../evidence/error-edges-20261111/README.md#ordinary-scalar-return-edges)

The [byte-bearing append comparison](../evidence/byte-bearing-append-20261111/README.md)
records separate success and truncation distributions around the 64-byte
copy cutoff and for a larger payload. The
[outlined producer diagnostic](../evidence/refill-and-append-20261111/README.md)
keeps parent and child calls visible and shows the typed parent's return-pair
normalization and stack frame. Numeric append tail-branches to the child in
the equivalent outlined build. No optimization from this diagnostic is active.

The [release evaluation](../evidence/error-release-20261111/README.md) verifies
fixed-record append against matched C, Rust, and pinned Goose controls at
three native call topologies. Direct result-pair forwarding recovers a tail
branch. The broad trial regresses 64-byte success. The fixed-only trial
improves fully outlined rejection and mixed input, with a repeated rare-failure
cost. Both trials remain isolated. The inline comparison approaches the
release threshold and the outlined comparisons keep it open. The
[incremental reader](../tests/fixtures/typed_stream_reader.holo) verifies the first
slice end to end across files, fragmented sockets, and partial EOF.

The [late-forwarding evaluation](../evidence/error-late-forward-20261111/README.md)
replays the exact four-language matrix and traces the outlined success path.
Late forwarding preserves the inline and parent-only binaries and recovers
the fully outlined tail branch. That trial still costs elapsed time on
success and rare failures. A scoped metadata diagnostic removes three
parent-only reloads and narrows the measured gap against Rust. These trials
stay outside production and the release performance gate remains open.

The [fixed-reservation evaluation](../evidence/error-reservation-20261111/README.md)
subsequently closes the representative fixed-record gate on macOS arm64.
The active checked backend passes the cursor, capacity, and destination
through fixed typed producer calls. It commits the marker at the enclosing
success boundary and forwards the last child's result pair. In both native
runs, inline success and rare failures are within 1% of C, with p99 matching
or improving. Both outlined builds beat the C, Rust, and Goose controls
on those profiles, with lower p99. Mixed and all-rejection runs check the
tradeoffs separately. The evaluation covers this fixed-record workload
and target. The [staged frame-reader comparison](../evidence/frame-reader-20261111/README.md)
measures variable-size construction and fragmentation separately against
C, Rust, and Goose. The scalar reader trails C and Goose on all seven
nonempty-payload profiles. Its staging loop checks both bounds per byte,
while the final-slot copy and published-payload sum already use specialized
native paths.

The [checked staging experiment](../evidence/staging-copy-20261111/README.md)
then measures staging separately and reruns the complete frame reader.
A checked transfer with a four-byte scalar bypass reduces paired full-reader
elapsed time by 45.5–78.9% on six repeat profiles. It recovers most of the
two-byte-fragment regression from the first prototype. Empty-frame staging
still regresses, so the compiler API retains this as an opt-in experiment.

The [stable-slot evaluation](../evidence/stable-staging-slots-20261111/README.md)
proves distinct live backing and captures frozen descriptors outside the
timed loop. The combined range proof and exact tiny copy recover the
empty-frame regression in both runs. The full reader improves against the
previous prototype across all eight profiles. Its large wide-chunk
profile still trails C and Goose. The proof stays opt-in.

The [proved-only release qualification](../evidence/proved-staging-release-20261111/README.md)
retains scalar lowering for unproved copies and adds eleven wire profiles.
Empty headers split into one- and two-byte fragments repeatedly regress
against the ordinary compiler. Default enablement remains open.
The [bulk phase replay](../evidence/frame-reader-phases-20261111/README.md)
directly times admission and owned-payload consumption. At 192 bytes,
the admission gap against C and Goose persists after input is staged.
The owned scan adds a separate gap. Generated code retains a typed
producer call and per-field reservation loads in admission, and an
outlined sum with 16-byte reductions during consumption.

The [Linux ELF comparison](../evidence/error-linux-append-20261111/README.md)
replays the fixed-record gate in an arm64 Linux VM and runs x86-64 ELF
under Rosetta. Both outlined shapes pass the VM throughput and latency
comparisons on success and rare failures. The arm64 inline timing gate
remains open. The guest exposes no hardware PMU events, and the translated
run supplies no native x86-64 measurements.
[Strict replay and fresh availability probe](../evidence/native-pmu-availability-20261111/README.md)
[Verified native replay inputs](../evidence/native-pmu-replay-readiness-20261111/README.md)

### Named error unions

The experimental compiler accepts `from ChildFamily as Alias` in error
declarations and checks `on Alias.Case` against the expanded leaves. Explicit
`try` accepts a declared direct child in scalar calls and nested destination
fields. The child range gives LLVM a constant tag offset on the rejection
edge, while the payload lane, destination, and marker protocol stay the same.
The independent model and native tests cover full-width payloads,
short-circuiting, capacity failures, rollback, and retained published
references in fixed and byte-bearing layouts. Package checks exercise both
child visibility and file-local `using`.
[Specification](error-handling.md#error-union-rules) ·
[Runnable dashboard](../tests/fixtures/error_unions/README.md) ·
[Native tests](../tests/compiler/frontend/frontend_native.rs) · [Package tests](../tests/compiler/packages/packages.rs)

The scalar header package, fixed typed append, byte-bearing frame, typed
stream reader, and numeric region example emit byte-identical LLVM in all
three layouts against the sealed proved-staging evaluation's ordinary
compiler.
[Sealed compiler record](../evidence/proved-staging-release-20261111/README.md)

The [native union comparison](../evidence/error-unions-performance-20261111/README.md)
measures the same result ordinals and payload handling with a direct child
or the exact same Holo family. Inlined scalar union elapsed time falls by
4.3–7.4% against same-family Holo across the two runs. Fixed append takes
25.9–27.7% more elapsed time on success and rare failures. Its optimized
loop includes status and payload selection after child success.

The [fixed terminal lift](../evidence/union-rejection-lift-20261111/README.md)
moves the offset into the child's declared rejection constants. It reuses
the private destination ABI, composes verified tag ranges, and caps additional
definitions at four per producer. The candidate lowers inline and parent-only
union append to the same native binary as exact-family append. The fully
outlined parent tail-branches to its specialized child. The macOS arm64
success and rare-failure gates pass in all three call shapes in both runs.
The baseline fixed-append union's paired elapsed time falls 15–22%.
The inlined mixed profile remains slower than Rust in both runs.
The fixed terminal lowering is active. Continuing and variable-layout
children use the checked failure-edge translation.
[Fresh native Linux host probe](../evidence/native-pmu-replay-2213-20261111/README.md)

The [tiny staging replay](../evidence/tiny-staging-scalar-20261111/README.md)
compares two isolated short-copy paths with the exact scalar and proved-only
controls. Both still regress fragmented empty headers and lose bulk
performance against the proved-only control. The default scalar lowering
and existing opt-in proof policies remain in place.

The [loop-level staging trial](../evidence/staging-schedule-20261111/README.md)
proves a stable upper bound and selects the scalar or proved-slot path
before the enclosing loop. It recovers the one-byte empty-header profile
and retains bulk throughput. Two-byte empty headers still take 5.3% more
paired full-reader elapsed time and 8.0–9.7% more isolated staging elapsed
time than ordinary Holo. The trial stays isolated because of the
split-header regression and code-size growth.

The [outlined bulk staging trial](../evidence/staging-outlined-20261111/README.md)
keeps the small-bound loop inline and calls the proved bulk loop through a
private function. It reduces the split-header penalty to 0.5–2.7% in
isolated staging and 3.0–3.2% in the full reader. One-byte and wide empty
profiles also regress. The trial stays isolated.

The [scalar staging helper trial](../evidence/staging-scalar-helper-20261111/README.md)
keeps the final-byte index in registers. LLVM also unrolls the scalar
fragment loop and spills other captures. The two-byte empty case takes
8.6–9.3% more elapsed time in isolated staging and 7.6–9.0% more in the
full reader than ordinary Holo. One-byte regressions persist, so the
candidate stays isolated.

### Byte-bearing append reservations

Derived64 and Compact32 typed byte-bearing producers advance the checked
cursor and receiving address in SSA and return the successful end in the
existing result pair. Append commits that end once. Width checks, overflow
before exhaustion, field evaluation order, payload ownership, and rollback
keep their checked semantics. A continuing variable parent resumes at the
child's checked end. Numeric producers and the Wide layout retain the
marker-based convention. The change targets the checklist's destination
construction, check reuse, and error propagation mechanisms. Physical layouts
and the allocator are unchanged.

The [reservation replay](../evidence/byte-reservation-comparison-20261111/README.md)
checks the separate prestaged and split admission intervals and the full
reader with ordinary and proved staging. All six proved-staging profiles
improve in the complete reader in both runs. The default scalar staging
path improves empty two-byte headers by about 6%, with the other profiles
within 1.4% of baseline. Prestaged 64-byte input has a small admission
regression and an extra constant reload in the owned walk. Wide 192-byte
reading still trails C and Goose even with proved staging.

### 192-byte reader phase diagnostics

The [admission call intervention](../evidence/admission-call-20261111/README.md)
forces both producer definitions inline in the checked module. It removes
the remaining native producer edge and reduces paired 192-byte admission
elapsed time by 7.0–15.4% in the repeat. Wide prestaged admission drops
from 10.32 to 8.63 ns/frame and still trails C and Goose. Inlining only the
parent moves the call to the child and worsens the split reader's owned walk.

The [owned-scan intervention](../evidence/owned-scan-isolation-20261111/README.md)
separates the helper call from reduction width. In the two wide 192-byte
rows, 64-byte groups with the call intact cut repeat paired consumption
elapsed time by 37.5–40.7%. Removing only the 16-byte helper call cuts
11.7–15.7%. Combining both gives 2.82–2.83 ns/frame, ahead of all three
foreign controls in those consumption intervals. The larger group keeps
the initialized extent proof, guarded tails, wrapping sum and packed
layout. Both phase interventions stay isolated pending broader gates.

### Broad reduction gate and bounded producer inlining

The [outlined 64-byte reduction replay](../evidence/outlined-64-sum-gate-20261111/README.md)
uses the active compiler and 17 scanner and 115 full-workload rows.
Hot 64-byte scanning improves 22.4%, while hot 65-byte scanning slows
13.3%. The focused repeat confirms 8.0–18.1% slowdowns in all nine
lifecycle scan intervals. Those workloads scan lengths 0–31 and never
enter the wide loop. The 64-byte helper stays isolated.

The [bounded reader comparison](../evidence/bounded-reader-20261111/README.md)
qualifies whole-pair producer inlining in Derived64 on macOS arm64.
Standalone inlining improves all six full-reader profiles and all six
direct-phase totals with proved or stable staging. Ordinary scalar
staging improves 21.0% on empty two-byte fragments and 18.1% on
fragmented 32-byte frames in the repeat, with no repeatable latency
penalty across the six profiles. The dedicated success/truncation
corpus already inlines the same pair in the active baseline, yielding
identical machine images. Native checks verify exact-fit publication and
rollback for the selected pair. The policy is active in Derived64.

Combining inlining and the outlined 64-byte reduction improves
the full proved-staging reader by 9.0–19.9% in the repeat. On the wide
192-byte profile it reaches 62.8 million frames/s, against 61.9 for C,
35.6 for Rust, and 63.9 for Goose. The wider reduction's adverse gate
keeps the combined build experimental.

### Ordinary staging and large checksum qualification

Four ordinary CLI staging trials lower wide 192-byte elapsed time by
78–81% in the repeated full-reader gate. Their isolated staging
intervals improve by 89–91% on the same profile. Each trial still
regresses a tiny-fragment case. The exact one- and two-byte trial
improves isolated empty two-byte fragments by 8.2%, while isolated
one-byte fragments slow 25.1%. The other three candidates slow
isolated empty two-byte staging by 9–20%. The default staging loop
remains scalar. All trial bodies and native distributions are retained.
[Ordinary CLI qualification](../evidence/cli-staging-large-checksum-20261111/README.md)

The 64-byte checksum trial with a runtime length guard improves wide
scans. The retained branch slows all 13 short guarded scanner rows.
The literal 128-byte minimum also regresses the 129-byte row. The
active macOS arm64 backend uses the wide reduction for a recognized
pure call with a literal checked prefix of at least 256 bytes. All
six measured large spans improve by 11.6–56.6% in the repeat. All 18
short literal controls retain exact LLVM and native code. The proof
preserves initialized extent, argument order, the wrapping sum,
and the existing packed layout.
It changes check reuse and target specialization in the Goose
compatibility checklist. It adds no backing, copies, or reclamation
work to the scan. All 132 generic scanner and full-workload controls
retain exact prior LLVM. [Large-prefix evidence](../evidence/proved-large-sum-20261111/README.md)

The native Linux probe confirms the sealed append executables and
cannot produce hardware events on the available macOS host. The
frame-reader comparison still requires native Linux measurement
support and target qualification on both arm64 and x86-64.
[Replay handoff](../evidence/native-pmu-replay-readiness-20261111/README.md#native-host-handoff)

### Dominating staging ranges and packaged checksums

The staging trials match unsigned bounds in the checked lexical block
before a copy. They establish both a nonwrapping index range and the
extent of the exact initialized field on each side. Distinct live
allocation identities permit the transfer without changing overlap
semantics. Copies lacking either proof retain their forward checked
loop. The candidates preserve all caller assertions, physical layouts,
allocation rules, and append publication. This exercises check reuse
and target specialization from the Goose compatibility checklist.

Reusing checked endpoints for the next fragment cuts wide 192-byte
full-reader elapsed time by 82.8% against ordinary Holo in the repeat.
Isolated empty two-byte staging takes 2.6% longer than ordinary and
16.0% less time than the same asserted source compiled without the
proof. Isolated empty one-byte staging still takes 14.7% longer than
the same asserted source. All three candidate policies stay isolated.
The original unasserted reader and stage binaries reproduce exactly.
[Range proofs, native checks, and gate](../evidence/staging-range-proof-20261111/README.md)

In the locked checksum package, about half the accepted records use
checked 257- or 320-byte literal prefixes in the mixed corpus. The
adopted checksum lowers repeat elapsed time by 18.1% in the complete
worker and 35.7% in the owned scan. Sparse full elapsed time rises
5.3% in the main run and 2.3% in the repeat, while sparse scans improve.
Sharing the narrow helper with the wide remainder changes how LLVM
specializes the narrow seed in this linked image. The measurements
leave the cause of the sparse full slowdown open.
[Locked package and distributions](../evidence/packaged-mixed-checksum-20261111/README.md)

The Linux reader handoff now contains verified static Holo, C, Rust,
and Goose ELF sets for both architectures and all 19 wire fixtures.
The native runner requires the matching target and usable branch and
cache events before timing. Linux execution remains pending a native
runner for each target.
[Linux reader handoff](../evidence/native-linux-frame-reader-20261111/README.md)

### Initialized slot cursor induction and checksum tail isolation

The macOS arm64 Derived64 emitter can prove staging ranges in the
unchanged reader. Exact extents come from successful lexical slot
construction and stable borrows. The existing source-room assertion
and cursor advance preserve the source bound. The receiving minimum,
advance, and retry or reset preserve the receiving bound. Selection
requires disjoint live backing and leaves original assertions in
order. The proof adds no runtime metadata, allocation, or error
work. Other layouts and unproved shapes use their prior lowering.
This qualifies check reuse and target specialization without changing
the storage, reclamation, or publication mechanisms in the Goose
compatibility checklist.

The first dispatch regresses the isolated empty one-byte profile by
26.7% and the full empty two-byte profile by 4.9%. The qualified
one-byte-first dispatch lowers repeat elapsed time by 15.4% and
16.7% for isolated empty one- and two-byte fragments. Full-reader
elapsed time falls 10.8% and 8.2%. Wide 192-byte full reads improve
81.3%. The 19-profile replay compares the same source with ordinary
Holo and the sealed C, Rust, and Goose controls.
[Cursor proof and release gate](../evidence/staging-cursor-proof-20261111/README.md)

A separate checksum tail restores constant-seed narrow instructions
at a cost of 168 native text bytes. Its sparse uninstrumented elapsed
change moves from +1.31% to −1.05% against the active shared tail,
and its short scan slows roughly 3% in both runs. The same-invocation
phase clock shows lower sparse admission and scan elapsed time for
the active shared tail against the pre-specialization compiler. The
instrumented worker and the original worker give different sparse
total directions. The shared-tail implementation remains active.
[Private-tail trial and split clocks](../evidence/packaged-checksum-tail-20261111/README.md)

### Prestaged admission and native replay

On the maintained macOS arm64 compiler, prestaged wide 192-byte admission
takes 8.48 ns per frame in the repeat. C takes 7.45 ns, Rust takes 10.11 ns,
and Goose takes 7.09 ns. Holo's repeat p99 is 10 µs per 1,024-frame batch.
The diagnostic measures admission after slot initialization and measures
owned payload consumption in a separate interval. The private copy
wrappers increase paired admission time by 4.4% and 6.4% in the repeat.
The native loop still loads variable slot extents where the C control
uses its asserted fixed stride. Extent reuse across publication and
iteration is a candidate for the Goose check-reuse gate. It would need
to preserve published layout, received-prefix checks, and rollback.
[Prestaged distributions and copy edges](../evidence/prestaged-admission-current-20261111/README.md)

The separate historical ordinary CLI replay shows that the current cursor
proof lowers 192-byte wide split admission from 93.88 to 13.98 ns per frame
in the repeat. The earlier stable-slot opt-in takes 12.74 ns per frame in
the same replay. The previous and current ordinary CLIs produce identical
prestaged LLVM. The copy-wrapper alternatives pass all 24 O0 and O3
AddressSanitizer oracle checks and remain diagnostic builds.
[Historical ordinary and ABI checks](../evidence/prestaged-admission-current-20261111/diagnostics/README.md)

Both sealed Linux reader sets, all 19 fixtures per architecture, and the
matching replay tools reverify. The strict runner rejects the available
macOS arm64 host before creating results. Native branch and cache counts
for both Linux architectures remain pending.
[Native replay commands](../evidence/native-linux-frame-reader-20261111/README.md#native-replay)

### Fixed extents in prestaged admission

An assertion that each initialized prestaged slot has extent 258 reduces
paired wide admission elapsed time by 11.45% and 11.79% on macOS arm64.
The repeat takes 7.88 ns per frame, with C at 7.82 ns and Goose at 7.23 ns.
LLVM uses the asserted equality to remove backing checks in the timed
decoder and still loads each extent for the cursor advance. The frame's
received-prefix checks and typed truncation remain in order. The failed
extent witness traps before printing a batch.
[Checked assertion, native distributions and witness](../evidence/prestaged-extent-20261111/README.md)

The opt-in emitter proves a common extent across all append sites of an
eligible lexical batch and uses it for captured traversal and byte bounds.
The unchanged prestaged source emits a 266-byte advance and retains the
received-prefix checks. It takes 7.80 ns per wide frame in the repeat,
versus 8.83 ordinary, 7.81 C, 10.43 Rust and 7.20 Goose. Combined admission
and scan take 12.88 ns per frame versus 11.00 C and 10.57 Goose. This
exercises the Goose check-reuse objective with the same flat layout,
publication, allocation, rollback and marker reclamation. Ordinary emission
keeps its prior policy while the trial is limited to macOS arm64 Derived64.
[Proof, comparison and optimized LLVM](../evidence/prestaged-extent-20261111/README.md#opt-in-compiler-proof)

### Split-reader wide staging

A separate native experiment replaces the proof-selected staging `memcpy`
with disjoint 64-byte transfers inside the same initialized ranges. Direct
inlining improves wide admission and raises full empty two-byte elapsed by
9.58% in the repeat. A private register-preserving leaf keeps the tiny
copies inline and limits code at the staging call site. It lowers wide
split admission to 12.14 ns per frame, versus 14.99 ordinary, 13.40 stable
slots, 13.76 C, 17.14 Rust, and 12.62 Goose. Full-reader wide throughput
rises to 59.15 million frames per second, with C at 59.69 and Goose at
63.14. The full empty one- and two-byte medians stay within 1.74% of
ordinary in both runs. The 17-byte fragment split repeat still has a
higher p99. The leaf stays an isolated lowering experiment. It keeps the
existing bounds, disjointness, producer copy, flat layout and rollback.
[Staging code, full gate and regressions](../evidence/split-bulk-inline-20261111/README.md)

### Checked outlined staging and owned consumption

The private leaf is now available through the opt-in checked emitter on
macOS arm64 Derived64. It uses the original cursor proof, preserves the
one-, two-, and three-byte paths, and emits only for proved disjoint
initialized ranges wider than 64 bytes. It reproduces all three isolated
outline binaries. The evidence records the fresh tiny-fragment and
wide-split gates and preserves the earlier unfavorable repeat. The ordinary
CLI and other targets retain their previous lowering. The scope is the
Goose check-reuse objective with no new allocation, layout, rollback, or
reclamation cost.
[Checked outline and acceptance evidence](../evidence/checked-staging-20261111/README.md)

Expanded extent-proof tests exercise zero-byte records, live mixed branches,
hosted refill reuse with both uniform and mixed append sites, and same-root
cursor reassignment with retained aliases. The mixed output batches keep
ordinary cursor lowering. The phase replays also isolate the sum helper
on the outlined staging and uniform extent images. The inline 64-byte
diagnostic lowers the repeat wide split owned scan from 4.80 to 2.70 ns per
frame and reaches 67.75 million frames/s in the full reader. C takes 60.35,
Rust 35.21, and Goose 63.51 million frames/s in the same replay. The dynamic
scan trial stays separate from the compiler's existing large-prefix rule.
[Extent conformance](../tests/compiler/backend/uniform_batches.rs) · [Owned scan and full reader](../evidence/checked-staging-20261111/README.md#owned-scan-isolation)

### Packaged wide staging default

Locked split and full applications call the typed decoder in a provider
package and reproduce the original flat LLVM and native images. In the
packaged repeat, the outline reduces wide split admission elapsed by
20.15% and full wide-reader elapsed by 11.68%. A 32-pair focus pass gives
equal split p99 at 56 µs and full p99 of 58 and 57 µs for 32-byte payloads
with a two-byte limit. The broader runs preserve the observed p99 variation.
Actual file and nonblocking stream reads separately verify typed truncation,
capacity rollback, exact-slot retry and payload ownership after refill reuse.

The default applies on macOS arm64 Derived64 when the cursor proof establishes
a maximum of 65–256 bytes. Larger maxima retain the existing bulk intrinsic,
and the size decision adds no runtime branch. The release CLI reproduces the
measured split and full binaries. This advances check reuse and target
specialization with the same flat layout, publication, allocation, and
marker reclamation. Linux PMU qualification remains separate.
[Locked package gate and default identity](../evidence/packaged-reader-gate-20261111/README.md)

### Experimental error-handling milestone complete

The first experimental slice is accepted. It covers typed scalar results,
nominal child unions, explicit `try`, exhaustive `on`, and fixed and
byte-bearing typed append across local packages. The locked staged reader
remains the regression gate for hosted refill, rollback, retry, and owned
payloads. Matched measurements qualify fixed-record error handling on macOS
arm64 and report the packaged reader's current performance. Owned scanning
and native Linux counters remain compiler and reader performance work.
Independently stored aggregate results, foreign public types in signatures,
and borrowed error payloads remain follow-on ownership and package work.
[Scope and regression command](error-handling.md#implementation-stages-and-acceptance) · [Packaged evidence](../evidence/packaged-reader-gate-20261111/README.md)

### Exact sparse worker profile

With their original clocks, the sealed shared-tail worker takes 2.33% and
2.00% longer than the pre-specialization worker in paired runs. The repeat
retires 0.73% fewer instructions and consumes 0.68% more cycles by paired
whole-process medians. Separate interrupted-PC sampling sees the wide
helper in the mixed control. At two sampling intervals, admission plus its
copy call and the owned scan show no consistent shift large enough to
localize the sparse slowdown. The shared-tail policy remains active.
[Original-clock replay and profile](../evidence/sparse-native-profile-20261111/README.md)

### Owned scan qualification after the error milestone

The locked staged reader's fresh regression tests and release re-emission
match the accepted default images. An isolated guard on the checked
checksum descriptor selects the 64-byte helper at 128 bytes or more.
The wide 192-byte full reader improves 10.21% in paired repeat elapsed
time and reaches 67.76 million frames/s, versus 60.61 C, 35.26 Rust and
64.17 Goose. The full 64-byte profile regresses 6.57% and split empty
two-byte admission regresses 9.78% in the repeat. The dynamic guard stays
isolated. The experiment changes reduction and dispatch while preserving
packed storage, publication and reclamation.
[Locked reader and guarded scan](../evidence/owned-scan-qualification-20261111/README.md)

Both guarded variants also regress the locked worker's short and sparse
owned scans. Separating dynamic and literal wide helpers preserves the
existing narrow and literal native instructions. The separate guard still
raises repeat short and sparse scan elapsed by 7.34% and 9.83%. An isolated
literal 192-byte proof improves its measured repeat by 14.79%, while the
broad 192-byte minimum regresses the 240-byte profile by 9.35%. Exact
three-block selection is a candidate for proved call sites. The ordinary
256-byte literal threshold retains its qualification.
[Locked worker guard](../evidence/packaged-dynamic-scan-20261111/README.md) · [Exact literal width](../evidence/exact-literal-owned-scan-20261111/README.md)

The next locked package trial adds an explicit checked 192-byte prefix
alongside short, dynamic, 257-byte, and 320-byte spans. Using the shared
wide helper lowers repeat scan elapsed by 56.19% for the fixed 192-byte
corpus and 17.62% for the extended mixed corpus. The original package
without a 192-byte proof reproduces its LLVM and linked image. The
three-block policy remains a measured candidate for checked compiler
selection. Dynamic widths use the existing path.
[Locked three-block scan](../evidence/packaged-exact-192-scan-20261111/README.md)

### Checked exact three-block scan

The opt-in checked emitter selects the existing wide helper for a proved
literal 192-byte span on macOS arm64. It preserves the prefix
check and the dynamic and ordinary selection paths. Both policies reproduce
the respective locked experimental binaries, and the unextended package
reproduces its original binary under either policy.

The locked native repeat reduces owned scan elapsed by 56.79% for fixed
192-byte records and 16.27% for the mixed 192-byte corpus. The full sparse
elapsed median increases slightly in both broad passes. Full mixed 192-byte
elapsed improves in both passes, with a pooled p99 increase from 71.00 to
109.53 µs in the second pass. The focused exact-binary rerun reduces elapsed
by 6.68% and pooled p99 from 74 to 69 µs. This is an opt-in result on native
macOS arm64. Native Linux branch and cache counter replay is pending.
[Checked package qualification](../evidence/checked-exact-192-package-20261111/README.md)

The exact sparse replay adds a byte-identical control and the sealed private
helper. The old sparse input has no accepted 192-byte records. The shared helper
changes native 257- and 320-byte paths, adding 22,144 executed helper
instructions per full sparse child. The earlier elapsed regression did not
reproduce. Shared elapsed was 0.60% and 2.57% lower in the two new passes.
The scan stayed opt-in during this review. [Sparse isolation](../evidence/exact-192-sparse-replay-20261111/README.md)

The O3 release review puts the eligible call into the locked full reader
and keeps the same checksum, staging, admission, and batch clocks in both
policies. The repeat cuts 192-byte elapsed by 13.46%, 3.78%, and 0.92% at
fragment limits 255, 17, and 2. The two-byte empty case also improves,
while the two-byte 32-byte case moves from −0.04% elapsed in the main
pass to +0.26% in the repeat. Its pooled repeat p99 increases from 53 to
54 µs. Native text grows by 136 bytes with the mapped text segment size
unchanged. The unextended split and full readers reproduce their sealed
LLVM with either policy.

The review supported a macOS arm64 Derived64 default candidate for this exact
proof, with short-frame tail and footprint costs. This O3 fixture
tests one module with the 192-byte edge and dynamic fallback. The prior
mixed-width package measures the shared 257/320-byte helper effects at O2.
[O3 proof-bearing fragment review](../evidence/exact-192-o3-fragment-20261112/README.md)

The mixed-width O3 replay qualifies the shared helper with checked 192-,
257-, and 320-byte calls in the same locked module. The repeat improves
full mixed-192 elapsed by 5.27% and scan elapsed by 17.56%. The original
mixed full case moves by +0.15% and +0.05% across the two passes, within
the byte-identical control variation. The shared helper adds 28 native
text bytes in this image. The earlier proof-bearing fragment reader's
short p99 and code-size costs still apply to the adoption decision.
[Mixed-width O3 gate](../evidence/exact-192-o3-mixed-20261112/README.md)

### Exact three-block default on macOS arm64

Ordinary Derived64 emission on macOS arm64 now selects the wide checksum
helper for the exact checked literal 192-byte prefix. The adoption accepts
the measured 1 µs short-frame p99 cost and the extra linked text in the
qualified readers. The 32-byte elapsed changes were small, the original
mixed reader stayed within the byte-identical control variation, and
the eligible mixed and fragmented workloads repeatedly improved.
The compiler still evaluates and checks the prefix before the call.
The policy adds no dynamic width dispatch or storage work.

The default follows the checked candidate through the ordinary CLI and
retains the sealed O3 mixed-width and two-byte readers as regression
controls. Both locked eligible modules and benchmark links reproduce
their qualified bytes. The post-adoption repeat lowers full mixed-192
elapsed by 6.41% and two-byte-fragment 192-byte elapsed by 0.86%.
The 32-byte fragmented p99 is equal within each of the two new passes.
The original mixed reader's pooled p99 increase in the main pass did
not recur in the repeat.

Replaying the pre-adoption ordinary arm uses the archived
emitter or executable. Rebuilding the old ordinary driver from current
source would pick up the new policy.
[Release CLI and native gate](../evidence/exact-192-default-20261112/README.md)

### Whole-result aggregate forwarding

The first aggregate-return increment admits a constructor whose body is
one `return other_constructor(...)` with the same nominal record and
result family. This reuses existing source spelling and checked argument
evaluation. The forwarded call receives the original destination and owns
all initialization and size metadata. The append remains the publication
and rollback boundary. This advances the Goose destination-construction
objective without new backing, aggregate transfers, or per-object cleanup.
Materialized aggregate locals and moves across lifetimes remain separate
ownership decisions.
[Forwarding source rules](error-handling.md#forward-a-whole-destination-result) · [Native and package acceptance](../evidence/aggregate-return-forwarding-20261111/README.md)

### Fresh aggregate locals

The entry-only experimental slice selects an explicit bounded region and
introduces one readonly lexical record on successful construction. The
checked producer receives the final pointer. There is no collection, indexed
lookup, extra backing, or payload staging. The failure arm sees the original
marker and can reuse the tail. Normal and early success exits restore one
marker. Native acceptance covers fixed and byte-bearing records across all
layouts. This implements the scoped destination construction, stable-address,
and bulk release cases from the Goose compatibility checklist. Implicit
storage, aggregate transfer, and propagation of storage failures remain open.
[Storage and lifetime rules](spec.md#fresh-aggregate-locals) · [Acceptance evidence](../evidence/scoped-result-20261111/README.md)

A hosted read can lend its exact fresh prefix to the scoped constructor.
The conformance fixture uses a short read with initialized stale backing,
checks nested truncation and rollback, and retries at the restored address.
The same fixture runs with files and nonblocking sockets across all
layouts. The current compiler also reproduces the two sealed Linux
reader modules exactly. The portable handoff verifies outside the
workspace and still needs native Linux hosts for PMU capture.
[Hosted conformance and Linux handoff](../evidence/scoped-hosted-refill-20261112/README.md)

The locked dependency regression runs the same hosted boundary through
the production decoder. The imported constructor's nested rejection
preserves its required extent and the region marker. The failure handler
retries from the same live view, and the package emits the same LLVM
as its flat counterpart across all three layouts.
[Locked decoder acceptance](../evidence/locked-hosted-scope-20261112/README.md)

### Automatic placement experiment

**Programmatic prototype.** The [working proposal](spec.md#compiler-selected-backing-for-one-scoped-aggregate)
selects one bounded backing for one scoped aggregate site. The original
[explicit-region control](../evidence/scoped-placement-design-20261112/README.md)
acquires its backing before a counted loop, succeeds twice, and rejects
twice after the parent has written a private field. Its two successful visits
use the same address. Every visit leaves the marker at the checkpoint.

The prototype compares two forms of the same fixture. The control names
the region explicitly. The candidate obtains the compiler identity from
the profile's owning scope. The two forms share capacity, acquisition
recovery, handlers, input, and physical layout. Candidate code cannot name
the private backing. The conformance test compares ordinary emitted LLVM
before adding test-only backing accounting.

| Gate | Required evidence |
|---|---|
| Selection and lifetime | Accept repeated visits to the one site. Reject a site in the acquisition-failure branch, a second site, competing ownership, and an escaping alias or byte view. Preserve readonly access and live input references rooted elsewhere. |
| Failure and cleanup | Match the independent model and explicit control for exact fit, one byte short, zero usable bytes, overflow, and injected acquisition failure before argument evaluation. Preserve the lifted rejection payload, left-to-right actual effects, and one logical release after success, including early return. Retry by revisiting the same site. |
| Native and generated code | Verify both forms across Wide, Derived64, and Compact32 at O0, O3, and under AddressSanitizer. Require LLVM identity for the matched ordinary forms. Inspect the direct final destination, one setup allocation and release per owner activation, unchanged cursor on cursor-mode failure, and marker rollback on marked failure. |
| Cost accounting | Report producer work separately from setup and backing release. Count allocation requests, copies, layout metadata, and logical capacity. Distinguish observed instructions from semantic marker resets, and requested bytes from reserved, committed, and resident bytes. |

Twelve matched pairs emit byte-identical LLVM across all three layouts.
Twenty-four model runs and 36 native O0, O3, and AddressSanitizer
executions validate the outcomes. Compiler selection leaves the generated
costs of the matched program equal. Physical backing costs and compiler
latency were outside this conformance run.
[Prototype verification](../evidence/scoped-placement-prototype-20261112/README.md)

This exercises compiler selection, finite backing, existing checked growth
bookkeeping, inline layout, static destination calls, error payload dispatch,
and bulk reclamation from the
[Goose compatibility checklist](spec.md#goose-performance-compatibility-checklist).
Goose's documented compiler-wide stack accounting and virtual backing are
broader than this proposal. The pinned implementation accounts for live
stack slots across calls and manages platform reservation separately.
General stack coloring and growth, relative links and tagged layouts, generic
or callback specialization, queues and images, and pools and stable handles
remain deferred for this experiment.
[Goose data-stack reference](https://aardappel.github.io/goose/docs/reference.html#data-stacks) ·
[Pinned source verification](../evidence/scoped-placement-design-20261112/goose-reference.log)

### Source budget and acquisition recovery

The [experimental source form](spec.md#supplying-a-stack-budget-in-source)
uses a braced `with core.region(bytes)` success block and a
mandatory `else` block. An eligible one-operand `core.construct` selects
the private backing. The existing two-operand form still provides the
explicit-region control.

The source adapter is ready to leave this milestone when these gates pass.

| Gate | Required evidence |
|---|---|
| Syntax and scopes | Check and format the new form while preserving function-header `with` and ordinary names. Reject absent recovery, zero or multiple implicit sites, a site outside the acquired scope, an ineligible path, and a nested owner. Keep named region construction valid. |
| Setup and continuation | Observe budget evaluation exactly once and before acquisition. Force a setup failure and observe `else` before any constructor actual. Check that both branches may reach the following statement and that success-local bindings cannot reach recovery or the continuation. |
| Outcomes and cleanup | Cover exact fit, a one-byte shortage, zero capacity, overflow, lifted rejection after a private write, retry at the same site, early exit, and an escaping reference. Check normal release before the continuation. |
| Compiler identity | Compare verified LLVM with a matched control whose named region lives in an ordinary brace scope. Require byte identity across layouts, matching independent-model outcomes, and O0, O3, and ASan execution. Include one package-qualified producer and preserve diagnostic spans. |

The budget and owner boundary are supplied by source. Cross-function
placement, multi-site allocation, VM backing, and broader growth remain
separate under the [Goose compatibility checklist](spec.md#goose-performance-compatibility-checklist).
[Explicit source control](../evidence/scoped-placement-source-20261112/README.md)

The standalone, locked-package, and hosted source fixtures pass the LLVM
identity gate in all three layouts. Fault-injected acquisition keeps only the
budget effect, skips the constructor, and joins the ordinary continuation.
The success path releases its backing before that continuation. The hosted
control verifies short views and owned payloads through file and Unix stream
input. [Source implementation evidence](../evidence/scoped-placement-source-impl-20261112/README.md)

The next application milestone is [offline RESP2 parsing](milestone-0.1.md#1-parse-resp2-in-holo).
It exercises fragmented `GET` and `SET` requests through the existing input
path. Native Linux PMU replay and a toolchain-triggered macOS timing replay
track performance separately.

### Checked error-handling closure

The implemented slice includes scalar results, declared child errors,
`try` and exhaustive `on`, typed append, direct destination forwarding,
and scoped fixed and byte-bearing construction. The two locked hosted
tests cover real reads, exact received counts, rollback, retry, and owned
payloads. Their repeat passes through the ordinary compiler.

The fixed-record macOS throughput gate has passed. Whole-reader
comparisons measure staging, typed admission, and owned scanning together.
The matched generic O3 macOS replay leads the measured C, Rust, and
Goose controls on throughput across seven wire profiles. The exact
192-byte reader's two-byte-fragment p99 differs from the plain Holo
source form, so both remain in the performance record. Native Linux
replay remains pending until both architectures are available with the
required events. The checked slice can proceed to the next language
milestone with the release replays retained.
[Supported error slice](error-handling.md#implementation-stages-and-acceptance) · [Closure record](../evidence/error-slice-closure-20261112/README.md) · [Matched O3 reader](../evidence/frame-reader-macos-o3-20261112/README.md)

The maintained macOS release runner pins the qualified replay or relinks
the frozen ordinary and current release LLVM with one LLVM and SDK.
Preflights verify the work before timing and unexpected LLVM changes
produce a diff for review. Both paths passed on the installed toolchain.
[Release gate and commands](../benchmarks/scalar-errors/frame-reader/README.md#macos-release-replay) · [Gate validation](../evidence/exact-192-release-gate-20261112/README.md)

### Experimental socket library

Manifest packages can now use `holo.net` to interpret hosted receive, send,
and readiness outcomes as nominal typed results. Package tests exercise
result boundaries and error payload forwarding. A native connected-stream
test covers readiness, send progress, would-block, and receive EOF. Existing
hosted applications continue to use the native session owner.

The first backend qualification used a single-flight Linux `io_uring`
socketpair fixture and a native macOS readiness comparison. Batched
`io_uring` gained throughput in the arm64 guest at depth and lost throughput
at depth one. The standalone Linux probe source was removed. Ordinary
`kevent` passed the matched native TCP comparison with its strongest gains
on sparse and burst workloads.
Native Linux qualification remains pending. The checked hosted entry owns
one lexical `TcpStream` at a time and can now run for successive clients
behind one listener. Darwin's opt-in kqueue reuses one queue across those
clients. Native tests reject an injected prior-generation event at the same
descriptor number and verify release after mid-sequence failures. Matched
checked Holo trials show modest central throughput gains and inconsistent
tail gains, so poll remains the default. The opt-in concurrent host keeps
at most 64 independent checked owners and routes waits through a bounded
shared broker. Its native tests cover an idle admission during broker
failure, worker startup, and reused descriptor identity. Matched native
macOS throughput was within 1% of parity across active and mixed clients.
The sealed Linux replay now has a physical-host and hardware-counter
gate. Both seals passed the repeat preflight, and physical Linux PMU
measurements remain pending. With paced readers on native macOS, the
loopback workload's throughput plateaued at eight concurrent workers.
Sixteen workers reduced admission p95 from 127–131 ms to 3.6–4.5 ms
while increasing admitted drain time. In the mixed-reader follow-up, an
eight-worker cap filled with eight paced readers gave fast arrivals an
admission p95 of 104–108 ms. Sixteen workers cut it to 3.7–4.1 ms while
slow throughput stayed near 250 MiB/s. At a fixed eight-worker limit, the
follow-up sweep located the fast admission p95 jump between seven and eight
slow readers. Interleaving admitted the first seven fast peers within
10 ms at saturation while the fast tail remained near 90–100 ms.
The opt-in parking profile now separates the live-slot cap from checked
execution permits. Each suspended owner retains its native thread and
checked backing. With eight slow readers, an 8 ms arrival gap and a 4 ms
read pause, held-eight admission p95 was about 97 ms. Both 16-live
profiles measured about 1 ms, including parking with eight permits.
The matched-live throughput ranges included parity. With eight permits
and 24 slow readers, the live-slot sweep measured a fast admission p95 of
286–287 ms at 24 slots and 10.8–11.6 ms at 32 slots. Median peak process
RSS rose by about 0.2 MiB across those two settings. Native reset churn
and generated Holo read and write resets passed on both macOS backends.
The 25–31 follow-up located the fast admission improvement at the first
spare live slot for 24 blocked slow senders. The 64 KiB replay worker
reduced server peak RSS from about 98.9 MiB to 4.0 MiB in the heavy
32-live workload. Sampled socket-buffer charges peaked near 37 MiB
in both variants. The lighter eight-slow workload showed higher fast
admission tails with bounded backing at the tested spare live limits.
General shared Holo state and in-flight buffer leases remain separate.
[Socket ownership and API](packages.md#owned-tcp-capabilities) ·
[Headroom and bounded reply comparison](../evidence/socket-streaming-20261112/README.md) ·
[Live-slot sweep and reset recovery](../evidence/socket-live-slots-20261112/README.md) ·
[Readiness parking and paced arrivals](../evidence/socket-parking-20261112/README.md) ·
[Worker threshold and joined readiness traces](../evidence/socket-readiness-threshold-20261112/README.md) ·
[Mixed readers and serial idle diagnostic](../evidence/socket-mixed-idle-20261112/README.md) ·
[Paced readers and serial diagnostic](../evidence/socket-serial-tail-20261112/README.md) ·
[Concurrent qualification](../evidence/socket-concurrent-20261112/README.md) ·
[Successive client qualification](../evidence/socket-many-20261112/README.md) ·
[Checked TCP conformance](../evidence/socket-owned-20261112/README.md) ·
[Initial qualification](../evidence/socket-library-20261112/README.md) ·
[TCP and batch qualification](../evidence/socket-tcp-qualification-20261112/README.md)

## Related evidence

- [Working specification and open decisions](spec.md)
- [Research map](research.md)
- [Compiler architecture](compiler.md)
- [Maintained measurement practices and workload protocols](../benchmarks/README.md)

The existing Holo suite, Goose/Rust experiments, and upstream Goose/C++/Rust
benchmarks are useful methods and fixtures. None is already the four-language
campaign described here, and a C++ row is not a C implementation.
