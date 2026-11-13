# Research map for the fresh language design

**Prepared November 8, 2026.** Companion to [the working specification](spec.md).
The [compiler architecture](compiler.md) records the backend direction and the
compiler-specific sources behind it; [the plan](implementation-plan.md) limits the
immediate implementation scope.

The sources below inform mechanisms and experiments. A paper's result applies to
its stated model, workloads, and implementation; it does not establish a universal
language advantage. “Design consequence” entries are our interpretation and
proposed work. They are not features that the cited work implements for Holo.

Use primary documentation and papers. For reproducible experiments, pin the
specific revision and toolchain; documentation links here may track newer releases.

## Hardware research notes

The [hardware research index](research/README.md) expands six opportunities from
the November 10 discussion into readable explanations and future prototype plans.
These are **Research** notes, not additional specifications, selected syntax,
implemented capabilities, or new Holo performance evidence. The existing M1
acceptance and M2 scope remain defined by the implementation plan.

| Topic | Detailed note | Main connection to this map |
|---|---|---|
| Independent memory accesses | [Batches](research/batches.md) | R06: overlap independent traversals while preserving effects. |
| Device buffers and completion | [I/O](research/io.md) | R08–R10: queue submission, cancellation, reuse, and durability boundaries. |
| Core, memory, and queue placement | [Locality](research/locality.md) | R07: coordinate workers, NUMA resources, and device queues. |
| Compact processing-friendly data | [Layouts](research/layouts.md) | R01, R03–R04: storage, mappings, encodings, and access costs. |
| Target-specific instructions | [ISA](research/isa.md) | R05, R11–R12: checked optimization facts, schedules, and data movement. |
| Pages and cache policies | [Pages](research/pages.md) | R01, R07, R10: backing, placement, logical reset, and persistence distinctions. |

Each note includes examples, restrictions, staged experiments, correctness gates,
measurement questions, an assessment, and annotated primary references. The index
records shared reproduction and compatibility requirements.

## Compatibility with Goose-style storage

Classify each proposed improvement before calling it an extension of Goose:

| Class | Examples | What must be established |
|---|---|---|
| Execution changes on the same storage | Fusion, tiling, batching, independent traversal interleaving | Preserve lifetimes, dependencies, numeric behavior, and effects; measure incremental gains. |
| Representation changes | Alignment, columns, blocking, codecs | State which flatness, growth, addressability, and reference guarantees still hold. |
| Permission or lifetime changes | Finer-grained exclusive access, ownership transfer, shared mutable data | Specify new alias, escape, synchronization, and reclamation rules; account for their costs. |

Goose already has `simd fn` multiversioning and `checkfree fn` obligations. Treat
those as part of the baseline, not new capabilities established by our schedule
syntax. [Goose reference](https://aardappel.github.io/goose/docs/reference.html#function-qualifiers)

Its deliberate aliasing is also part of the comparison. Adding exclusive access
may expose optimization opportunities while rejecting useful existing programs.
In particular, test keeping an element reference while growing its stable-address
array. See [the compatibility audit](spec.md#how-closely-this-draft-follows-goose).

## Goose, representation, and lifetime design

### R01 — Goose's memory architecture

Sources: [Goose overview](https://aardappel.github.io/goose/),
[language reference](https://aardappel.github.io/goose/docs/reference.html).

Goose distinguishes native execution frames from compiler-assigned data stacks.
Its constraints enable contiguous representations, stable growth in supported
cases, relative links, and direct construction. Its reference also describes
arrays, trees, and reusable pools; typed queues constrain cross-thread
communication, not the entire set of data structures. Current pool APIs include
`slot_pool<T>` and `slice_pool<T>`; old `reusable` syntax is deprecated.

Goose already supports long-lived ownership and pool reuse. Its documented pools
retain initialized backing; a stale reference can observe a replacement value,
so memory safety does not establish stable logical identity. Its October 2026
[coroutine research note](https://aardappel.github.io/goose/docs/design/coroutines.html)
explicitly describes unimplemented syntax/mechanisms: fixed-size frames, external
pooled buffers and library scheduling. Treat these as research, not executed
async-I/O evidence. Completion-based buffer lifetime and cancellation obligations
still require their own specification and acceptance cases in Holo.

**Design consequence:** investigate representation/lifetime guarantees rather than
requiring every target to implement one virtual-memory mechanism. Include pools
and stable logical identities alongside nested regions. Determine what happens
when a slot is reused while an old handle still exists.

### R02 — Goose's benchmark methodology

Source: [Goose benchmark design](https://aardappel.github.io/goose/bench/design.html).

The comparison discusses tuned arena/index controls, representation tradeoffs,
working-set sizes, construction, teardown, and memory use. Already-flat scalar
arrays offer less opportunity for its representation changes.

**Design consequence:** compare mechanisms causally. Measure compactness, copying,
indirection, allocation, and reclamation separately. Include expertly tuned native
baselines. Capture the user's concrete experimental results with their metadata
before promoting any result to a general claim.

## Layouts and encoded data

### R03 — Programmable mappings

Sources: [LLAMA mappings](https://llama-doc.readthedocs.io/en/latest/pages/mappings.html),
[Kokkos views](https://kokkos.org/kokkos-core-wiki/ProgrammingGuide/View.html).

LLAMA supports user-provided mappings and distinguishes physical and computed
access. Keeping mappings available at compile time is important to its optimization
approach. Kokkos provides additional examples of multidimensional layout and
memory-space abstractions.

**Design consequence:** specify a small mapping/access foundation for layouts,
including specialization and computed access. Establish which mappings return
native references, decoded values, or accessors. A convenient custom-layout API
is not automatically safe or free of dispatch overhead.

### R04 — Columnar and compressed representations

Sources: [Arrow columnar format](https://arrow.apache.org/docs/format/Columnar.html),
[Parquet encodings](https://parquet.apache.org/docs/file-format/data-pages/encodings/).

Arrow documents locality and SIMD-oriented representation choices with mutation
tradeoffs. Parquet provides concrete dictionary, delta, and run/bit-packing
encodings. These are useful reference points, not a demand to make either format
the language's universal in-memory representation.

**Design consequence:** separate arrangement, encoding, collection algorithms, and
operation capabilities. Preserve exact values unless precision loss is explicitly
authorized. Evaluate lookup and updates as well as scans and byte size.

## CPU, memory, queues, and persistence

### R05 — Optimization facts and aliasing

Sources: [LLVM alias analysis](https://llvm.org/docs/AliasAnalysis.html),
[LLVM language reference](https://llvm.org/docs/LangRef.html#parameter-attributes).

LLVM has distinct concepts for aliasing, memory effects, capture, alignment, and
initialization. Proven memory dependencies enable optimizations; read-only access
and non-aliasing are not interchangeable facts.

**Design consequence:** preserve checked ownership and access information through
lowering. Do not map every friendly keyword to a superficially similar backend
attribute. Establish the exact implication and its lifetime/scope first.

The local [readonly-input follow-up](implementation-plan.md#readonly-input-accounting-and-comparison--november-9)
keeps strong known-length spans alongside record-derived input views. The
subsequent short-copy trial confirmed constant-length propagation through
ordinary calls in the cases tested. The dynamic-length control deliberately
hid the length from the compiler.
An executable length assertion supplies a checked fact without freezing payloads.
A general short-copy candidate produced both gains and repeated regressions and
was not adopted. Its optimized widened read also demonstrates why exact original
IR is insufficient evidence of exact optimized traffic. Future extent propagation
must preserve initialization/lifetime rules, payload aliases and failure order;
these observations do not authorize uniqueness or deep immutability promises.

The later [ordinary failure study](implementation-plan.md#ordinary-construction-failure-and-retry-costs--november-9)
keeps dynamic request operands matched while measuring outcomes without output
barriers. Inspected generic copies retain bounds/alias checks and earlier payload
work before late rejection. This nominates narrowly proved range/check or private-
write optimizations for investigation; it does not establish their legality or
speed. Preserve original reservation, trap/error order and permitted source aliases
before substituting bulk copying or skipping failed private initialization. Source
nesting also differs from retained native frames after ordinary inlining.

The stable staging-slot trial reused initialized field extents across a loop
only for stable slots with distinct live backing. It retained dynamic subrange
checks and the scalar failure path. Its combined descriptor proof and inline
tiny copy improved the empty-frame profile against both controls.

The [whole-input intrinsic follow-up](implementation-plan.md#whole-input-copy-trial--november-9)
uses the narrower exact captured-length/index proof, retaining all reservation
order and aliases. It passes semantic checks but gives no broad performance gain;
the wide 4096-byte paired regression repeats, so the old lowering is restored.
Source-level proof and fewer original loop instructions do not establish a faster
machine path. Its actual target runtime inspection is scoped to one system image;
LLVM memcpy can become a runtime call, so byte semantics alone do not prove an
external implementation's transitive `noalloc`/`noblock` contract.

The November 10 scorecard rechecked these mechanisms with a locally verified,
modified Goose, C and Rust controls, and matched O2/generic, SDK 15.5, and
macOS 11.0 deployment settings. An independent replay validated all 1,680
measured observations and 108 preflights. Holo's 31-byte complete-record input
path was close to C and Rust record views and slower than
their known-length spans; selected ordinary assembly preserves that input-knowledge
distinction. Lifecycle phase diagnostics nominate construction for investigation.
Neither observation establishes causality, authorizes stronger alias promises or
selects a new optimization. All 27 actual benchmark LLVM modules match the accepted
M1 compiler despite unrelated lexer/source-tool/CLI changes; no optimizer or
accepted semantics changed. Distributions, drift, unresolved reset latency and
unavailable generic commitment prevent a global performance ranking.

The subsequent [construction branch-layout experiment](implementation-plan.md#construction-success-branch-layout-trial--november-10)
adds 5,210 measured observations and 337 preflights. Fixed success-biased branch
weights preserve the original checked LLVM after metadata removal and improve
all nine lifecycle paired medians by 2.04–3.65%. A wide 4,096-byte copy regression
repeats at about +2.24% and +2.49% paired cost with adverse IQRs, so the candidate
is rejected and the original compiler inputs are restored. The trial included
independent replay and 13 repeated copy pairs. This supports
investigating code placement under broader controls; it does not establish that
fewer branches, a favored success edge, or a diagnostic phase predicts a general
speedup. No performance policy, alias fact or language rule was adopted by that trial.

The [subsequent narrow/vector comparison](implementation-plan.md#narrow-construction-and-vector-follow-up--november-10)
tests two candidates independently against the same captured original compiler.
The selected private heuristic scopes success weights to checked all-U64 producer
signatures and their append calls, excluding forwarded borrowed payload reads.
Default lifecycle paired costs improve about 3% across three sizes, while full-copy
intervals cross zero and ten copy native pairs remain identical. The competing
exact byte-vector prefix retains the original scalar expression source but changes
actual native tail/transition code and has less consistent default/compact results;
it is not adopted. Eight vector review flags remain descriptive, not established
reproduced regressions. The comparison included 11,512 measured observations and
752 preflights, including screenings, with the finite runtime trust boundary
and language semantics intact. This supports narrow eligibility, native-code review
and broad regression controls; it does not prove that metadata, fewer vector
instructions or source-identical tails predict hardware cost. The comparison
used complete paired distributions to guide review of compiler policy.

The subsequent ingestion trial combined guarded binary-header decoding,
malformed-input rejection, filtering, owned destination copying, full consumption,
and scoped reuse. All 360 measurements and 30 preflights passed independent
replay.
Actual optimized Holo code already turns the validated subrange's checked byte
initializer into `memcpy`; all four controls vectorize the checksum scan. This
supports inspecting established range facts and native code before adding manual
SIMD. Goose's short-message advantage and closer medium/larger results nominate
copy dispatch, record reservation and code shape for profiling. They do not
establish which mechanism causes the elapsed gap or predict external I/O behavior.
The preserved Rust counter correction strengthens the reference before timing;
it is not a Holo optimization. Hot structured inputs and shared-host distributions
limit extrapolation beyond this workload.

### R06 — Latency hiding for irregular access

Source: [Cimple: Instruction and Memory Level Parallelism](https://arxiv.org/abs/1807.01624).

The work explores interleaving independent computations to expose instruction and
memory-level parallelism in data-processing workloads.

**Design consequence:** evaluate batching and multiple independent traversals as
well as contiguous SIMD scans. A graph or index workload may need latency hiding
even when arithmetic is cheap. Establish independence and measure on the target.

### R07 — NUMA and network queues

Sources: [Linux NUMA memory policy](https://www.kernel.org/doc/html/latest/admin-guide/mm/numa_memory_policy.html),
[DPDK PMD guide](https://doc.dpdk.org/guides/prog_guide/ethdev/ethdev.html).

Linux exposes memory-placement policies. DPDK documents queue ownership patterns,
burst processing, and NUMA-local resources; supported transmit sharing depends on
the driver/device capabilities.

**Design consequence:** runtime policies should be able to coordinate workers,
queues, and memory. Keep those policies explicit and backend-aware. Do not encode
one queue-sharing rule as a universal hardware law.

### R08 — NVMe submission and completion

Sources: [SPDK NVMe submission](https://spdk.io/doc/nvme_spec.html),
[SPDK NVMe driver](https://spdk.io/doc/nvme.html).

SPDK describes submission/completion queues, DMA-accessible buffers, and submission
that returns before completion. Its queue-pair API has a single-thread-at-a-time
usage requirement.

**Design consequence:** check buffer lifetime and permissions until the backend's
actual completion boundary. Represent queue access obligations. Pooling, pinned
storage, polling, and interrupt strategies should be measurable backend choices.

### R09 — Cancellation races

Source: [liburing cancellation manual](https://raw.githubusercontent.com/axboe/liburing/master/man/io_uring_cancelation.7).

Cancellation can race with completion, and cancellation/original-operation
completions have no guaranteed relative order. Some submitted hardware work cannot
be canceled at the requested point.

**Design consequence:** separate requesting cancellation from obtaining permission
to reuse resources. Model each operation's terminal protocol, including multishot
requests, and test race outcomes deterministically.

### R10 — Completion versus durability

Source: [Linux writeback-cache control](https://www.kernel.org/doc/html/latest/block/writeback_cache_control.html).

Volatile device caches can acknowledge completed writes before persistence.
Flush and FUA provide additional persistence guarantees within the documented
device/block-layer model.

**Design consequence:** give submitted, completed, and durable operations distinct
contracts. Database and recovery examples need explicit persistence protocols and
failure assumptions.

## Schedules and AI workloads

### R11 — Extensible checked scheduling

Sources: [Exo 2](https://arxiv.org/abs/2411.07211),
[MLIR Transform dialect](https://mlir.llvm.org/docs/Dialects/Transform/),
[MLIR Transform tutorial](https://mlir.llvm.org/docs/Tutorials/transform/).

Exo 2 moves substantial scheduling functionality into libraries using a foundation
of trusted operations and inspection. MLIR Transform separates transformation
programs from payload IR, with handles, failures, and operation-specific
requirements. Some operations leave correctness obligations to their users.

**Design consequence:** make schedule legality and trusted boundaries explicit.
Explore library-defined transformations without assuming arbitrary rewrites are
proved. Retain enough loop/data structure for useful transformations before
lowering to low-level code.

### R12 — Tiling and data movement

Source: [FlashAttention](https://arxiv.org/abs/2205.14135).

FlashAttention demonstrates an exact-attention algorithm organized to reduce
traffic between GPU memory levels through tiling.

**Design consequence:** preserve shape, stride, memory-space, alias, and numeric
information useful for fusion and tiling. Measure bytes moved and intermediate
storage. The work does not establish one best schedule for all CPUs or kernels.

### R13 — Dynamic block-based serving storage

Source: [PagedAttention / vLLM](https://arxiv.org/abs/2309.06180).

PagedAttention addresses growing and shared KV-cache storage through block-based
management in LLM serving.

**Design consequence:** investigate independently reclaimable blocks and explicit
sharing alongside contiguous regions. Keep model-specific policies in libraries
where possible; do not make LLM-specific objects core language syntax by default.

### R14 — Accelerator interoperability and lifetimes

Sources: [CUDA stream-ordered allocation](https://docs.nvidia.com/cuda/cuda-programming-guide/04-special-topics/stream-ordered-memory-allocation.html),
[DLPack Python specification](https://dmlc.github.io/dlpack/latest/python_spec.html).

These interfaces expose execution ordering, ownership/lifetime, device, and
interoperability obligations that a pointer and element type alone do not capture.

**Design consequence:** explicit memory spaces and completion protocols matter
before claiming safe sharing or zero-copy interoperability. Registration,
transfers, synchronization, and cleanup need inspectable costs.

## Composition, permissions, and checked optimization

### R15 — Capability scopes and captured dependencies

Source: [Effekt captures](https://effekt-lang.org/tour/captures).

Effekt tracks captured capabilities in types to prevent use outside their valid
scope. Its documentation distinguishes open effect requirements from fixed
lexically captured capabilities.

**Design consequence:** explicit function arguments can coexist with effect and
capture tracking. Investigate scoped test backends and dependency permissions
without requiring an implicit `uses` declaration in ordinary source.

### R16 — Restricted versus general effect handlers

Sources: [Koka tail-resumptive handlers](https://koka-lang.github.io/koka/doc/book.html#sec-tail-resumptive),
[Zero-Overhead Lexical Effect Handlers](https://cs.uwaterloo.ca/~yizhou/papers/zero-oopsla2025.pdf).

Handler strategy and restrictions affect implementation costs. Tail/linear paths
can avoid machinery required by general continuation capture. In the lexical
handler paper, zero overhead concerns mainline paths where effects are not raised;
raising and handling effects still requires work.

**Design consequence:** if handlers enter the design, begin with restricted,
ownership-compatible use cases and measured generated code. General replay and
multiple resumptions interact with unique resources and cannot be assumed safe.

### R17 — Permissions separate from data representation

Source: [GhostCell](https://plv.mpi-sws.org/rustbelt/ghostcell/).

GhostCell separates permissions from shared data through branded tokens, with a
mechanized soundness result for its API.

**Design consequence:** graph and index safety need not require a blanket ban on
shared structure. Compare permission-token approaches with ownership-based and
runtime-checked alternatives. The result already exists in Rust, so it is not
evidence of an intrinsic new-language performance advantage.

### R18 — Resource-coordinated concurrency

Source: [When Concurrency Matters: Behaviour-Oriented Concurrency](https://www.microsoft.com/en-us/research/publication/when-concurrency-matters-behaviour-oriented-concurrency/).

The work makes ordered, exclusive access to multiple independent resources a
concurrency primitive for asynchronously created work.

**Design consequence:** investigate scheduling from declared resource needs.
Measure coordination costs and specify progress/ordering guarantees. Its atomic
execution units are not automatically crash-durable database transactions.

### R19 — Independently checked compiler rewrites

Sources: [Alive2](https://github.com/AliveToolkit/alive2),
[Trivet: LLVM Translation Validation Automated with Large Language Models and Lean](https://arxiv.org/abs/2609.19583).

Alive2 provides LLVM optimization validation with documented limits. Trivet is a
September 2026 preprint combining LLM assistance with Lean-checked results for
selected LLVM transformations.

**Design consequence:** AI may propose a rewrite without being its correctness
authority. Record the supported transformation domain, trusted semantics,
assumptions, and unresolved cases. A checked local optimization is not a proof of
an entire service, compiler, or distributed protocol.

## AI generation and language design

### R20 — Grammar and semantic constraints for generation

Sources: [Synchromesh](https://arxiv.org/abs/2201.11227),
[Grammar Prompting](https://arxiv.org/abs/2305.19234).

These studies investigate using examples, grammars, and syntax/type/context
constraints to improve generation in their tested structured languages.

**Design consequence:** supply a canonical grammar, examples, compiler feedback,
and machine-readable semantic queries. Test generation and repair tasks using the
proposed headers against equivalent conventional headers. Neither paper measures
our syntax or establishes that longer English-like keywords are universally better.

## Testing architecture

The [compiler testing strategy](compiler.md#7-testing-architecture-and-staged-adoption)
and [M1 acceptance gates](implementation-plan.md#acceptance-checks) apply these
lessons. They are testing responsibilities, not newly implemented Holo features.

### R21 — Simulation and real-system testing

Sources: [Protocol-Aware Deterministic Simulation Testing, August 20, 2026](https://tigerbeetle.com/blog/2026-08-20-protocol-aware-dst/),
[A Descent Into the Vörtex, February 13, 2025](https://tigerbeetle.com/blog/2025-02-13-a-descent-into-the-vortex/).

TigerBeetle's protocol-aware DST checks internal consensus/storage invariants and
individual replica progress, as well as system-level behavior. Its complementary
Vörtex harness tests compiled binaries and real integration layers under faults.
The latter covers behavior replaced by controlled interfaces during simulation.

**Design consequence:** inspect Holo's checked/storage invariants as well as
outputs. Add controlled event simulation with concurrent/I/O components, while
retaining actual native/OS tests. A distributed simulator is not an M1 prerequisite,
and simulated timing is not hardware-performance evidence.

### R22 — Fuzzable interfaces and independent models

Sources: [A Tale Of Four Fuzzers, November 28, 2025](https://tigerbeetle.com/blog/2025-11-28-tale-of-four-fuzzers/),
[Fuzzer Blind Spots, June 6, 2025](https://tigerbeetle.com/blog/2025-06-06-fuzzer-blind-spots-meet-jepsen/).

TigerBeetle describes shaping minimal subsystem interfaces for efficient fuzzing
before implementation. It also reports a real bug missed by several fuzzers
whose workloads excluded the triggering pattern; broader inputs and a detailed
independent model exposed the issue.

**Design consequence:** build a small replayable compiler/region harness alongside
M1. Audit generators as well as the implementation, compare defined native
behavior with an independent oracle, preserve full failing inputs and identities,
and combine targeted and broader cases. Many seeds do not establish completeness.

### R23 — Antithesis as an optional testing platform

Sources: [How Antithesis works](https://antithesis.com/docs/introduction/how_antithesis_works/),
[Writing tests](https://antithesis.com/docs/product/writing_tests/),
[Asserting correctness](https://antithesis.com/docs/product/writing_tests/assertions/),
[Platform prerequisites](https://antithesis.com/docs/faq/prerequisites/).

Antithesis provides DST for compiled software in a controlled environment, with
workload drivers and custom assertions for exploration and property checking.
The application need not implement deterministic execution itself. SDK property
assertions can report failures without terminating the program.

**Design consequence:** keep test interfaces and workloads reusable, and evaluate
integration after an executable Holo slice exists. No vendor-specific language
syntax or mandatory release dependency follows. Reporting assertions must not
replace required safety checks or termination. Verify current deployment/target
capabilities at integration time, retain native tests on other targets, and keep
simulation, tested properties, and proofs distinct.

## What this map does not settle

- Whether this exact syntax improves human or AI task performance.
- Whether a new implementation outperforms an equally tuned Rust, C++, or Goose
  implementation on a specified workload.
- Which allocation/lifetime model should be the default for ordinary programs.
- The safe shared-memory model, complete layout interface, or async syntax.
- Whether the most advanced composition ideas justify their language complexity.

Use [the specification's decision agenda](spec.md#16-design-agenda-and-next-decisions)
to turn a research direction into a concrete rule, counterexample, prototype, and
measured or checked result.
