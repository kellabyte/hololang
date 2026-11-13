# ISA: useful CPU instructions

Keep operations such as filtering, copying, and matrix multiplication clear enough
that Holo can select a suitable hardware implementation.

**Research note — November 10, 2026.** These are prototype candidates, not selected
language rules or claims of implemented Holo support. An instruction set
architecture, or **ISA**, defines the instructions a CPU can execute. A published
extension does not mean every machine, operating system, or compiler supports it.

## Why this matters

Some processors can filter several values at once, submit work directly to an
accelerator, or multiply small matrix tiles in dedicated hardware. Those features
can help selected workloads substantially. Their usefulness depends on the shape
of the operation and its data, not merely on choosing a newer instruction.

Holo's opportunity is to retain useful facts until the compiler can use them:
which elements are valid, how many bytes exist, how results are arranged, and
whether operations may be reordered. A library or checked schedule could request
a specialized implementation while the ordinary algorithm stays readable.

## Four mechanisms worth exploring

### 1. SIMD masks and compression

**SIMD** means applying one instruction to several values. A **lane** is one value
inside that group. A **mask** records which lanes participate.

Consider keeping the values greater than five. This is a conceptual example,
not executable Holo syntax:

```text
input:        [3, 9, 1, 8, 7, 2, 6, 0]
keep mask:    [0, 1, 0, 1, 1, 0, 1, 0]
packed output:[9, 8, 7, 6]
```

AVX-512 includes compression and expansion operations that pack selected lanes
or distribute packed values into selected positions. Some subextensions add
byte- and word-sized forms. These operations are interesting for filters,
parsers, and classification kernels. They do not make arbitrary branching code
vectorizable. [Intel's permutation guide](https://builders.intel.com/docs/networkbuilders/intel-avx-512-permuting-data-within-and-between-avx-registers-technology-guide-1668169807.pdf)

The important Holo operation would be an ordered filter with a known output bound.
Its implementation could compare a group, count selected values, and write them
directly into an eligible destination. Output exhaustion and publication must keep
their specified behavior; reserving a maximum output earlier can change failure
order, so that is not automatically a legal transformation.

### 2. Scalable vectors and safe tails

Arm SVE supports vector-length-independent code: software discovers the current
lane count instead of assuming one fixed width. Predicates select active lanes,
including the last partial group. [Arm's SVE explanation](https://learn.arm.com/insights/sve/)

For ten elements and a current width of eight, one group uses eight active lanes
and another uses two. The final group must not read six nonexistent elements.
Predicated operations can express that boundary directly.

This suggests preserving masks, extents, and inactive-lane meaning in Holo's
checked representation. It does not imply that every stored vector type should
have a runtime size. Temporary computation shape and persistent data layout are
different decisions. Different vector widths must produce the specified result;
wider execution alone is no performance guarantee.

### 3. Accelerator queues: DSA and ENQCMD

Intel DSA is a data copy and transformation accelerator. Software supplies a
descriptor containing the operation, memory addresses, and size; hardware performs
the work asynchronously. It can copy, compare, or fill data, but it is not a
general arithmetic processor. [Intel's DSA example](https://www.intel.com/content/www/us/en/developer/articles/technical/accelerate-memory-bandwidth-bound-kernels-with-dsa.html)

`ENQCMD` submits descriptors to shared work queues and reports acceptance or retry.
`MOVDIR64B` supports 64-byte submissions to dedicated queues. Queue setup and
completion handling remain necessary. [Intel's interface explanation](https://www.intel.com/content/www/us/en/developer/articles/technical/scalable-io-between-accelerators-host-processors.html)

The Holo opportunity resembles [device I/O](io.md): source and destination storage
must remain valid, with suitable permissions, until the backend releases them.
Submission acceptance is not completion. Supported shared virtual addressing can
avoid traditional DMA page pinning; that requires platform and IOMMU facilities,
not just stable addresses. [Linux SVA documentation](https://docs.kernel.org/arch/x86/sva.html)

This should be an explicit optional backend. It cannot silently replace every
ordinary copy. Measure descriptor preparation, retries, waiting, and cache effects
before selecting a size threshold. Offloading a copy still moves the data.

### 4. Matrix tiles: AMX and SME

A tile is a small rectangular group of values. Intel AMX and Arm SME provide
specialized matrix facilities; AMX uses tile registers, while SME introduces
streaming execution and matrix storage called ZA. Each has target requirements
and an **ABI**: rules for passing values and preserving processor state across
function calls. [Intel AMX example](https://www.intel.com/content/www/us/en/developer/articles/code-sample/advanced-matrix-extensions-intrinsics-functions.html),
[Arm SME execution state](https://learn.arm.com/learning-paths/cross-platform/multiplying-matrices-with-sme2/3-streaming-mode/)

Holo could preserve dimensions, strides, element formats, and tile operations until
backend selection. Packing input and entering or preserving architectural state
are real costs. A fair experiment includes them. Floating-point reassociation,
reduced precision, or different accumulation behavior needs an explicit numeric
contract; hardware availability does not authorize those changes.

## What Holo has today

The finite experimental M1 region core is
[accepted under its reviewed native profile](../implementation-plan.md#m1-acceptance-decision--november-10).
It does not provide a general source SIMD/schedule system, DSA queue API, or matrix
library. LLVM already performs ordinary vectorization and instruction selection.
See [compiler architecture](../compiler.md#4-compute-simd-and-optimization).

Approved M1 access permits aliases: multiple references to the same data.
`readwrite` is not an exclusivity promise and
`readonly` does not freeze other aliases. Prove independence, retain a correct
aliasing path, or reject the requested transformation. Do not add `noalias` merely
to obtain vector code.

## Where the work belongs

| Component | Responsibility |
|---|---|
| Language rules | Define values, numeric behavior, bounds, evaluation order, and any explicitly granted scheduling freedom. |
| Checked representation | Retain extents, masks, destinations, effects, and proved dependence facts. |
| Compiler/backend | Choose legal intrinsics, inspect generated code, and manage target variants. |
| Library/runtime | Discover usable capabilities, own accelerator resources, and implement explicit dispatch and completion protocols. |

CPU feature bits alone are insufficient admission: include operating-system
state support, compiler lowering, device configuration, and applicable runtime
contracts. An unsupported explicitly requested backend must report that fact.
A separately documented portable dispatch mode may select a compatible path.

## A first prototype

Start with **ordered filtering of unsigned integers**, not all four mechanisms.
It has a simple independent oracle and exercises masks, tails, compaction, and
destination bounds without introducing floating-point questions.

1. Write a reference specification: preserve input order; select values above a
   supplied threshold; write only initialized selected elements.
2. Compare tuned scalar/autovectorized C and Rust with explicit target-specific
   mask-and-compress versions. Keep logical work, safety obligations, and output
   allocation phases matched. Test the intended pinned Goose checkout where
   expressible; report unsupported cases honestly.
3. Vary lengths around vector boundaries, selection density, alignment, and working
   set size. Include cache-resident and memory-bandwidth-limited inputs.
4. Only after native results justify it, prototype one operation through Holo's
   existing checked/backend path. Host-language kernels remain controls, never
   Holo-new results. Use `build/benchmarks/` for working output and record
   the results and their limitations.

### Follow-on prototypes

Keep these separate from the filter campaign so each result has one explanation:

- **SVE tails:** run the same ordered filter at every vector length the machine
  and OS actually support. Test lengths just below, equal to, and just above a
  vector boundary. The output must stay identical when the lane count changes.
- **DSA copies:** begin with nonoverlapping source and destination buffers and
  an admitted work queue. Compare CPU copies and queued copies at 1 KiB, 4 KiB,
  64 KiB, and 1 MiB, then vary the number of outstanding descriptors. Keep NUMA
  placement, initialization, and destination consumption matched. Validate every
  byte after release. Include descriptor preparation, queue retries, and
  completion processing in the timed operation; report one-time setup separately.
  These sizes are a starting sweep, not a dispatch threshold.
- **Matrix tiles:** choose one format and exact accumulation contract supported
  by the target. Compare against a tuned library with that same contract. Begin
  with 64-by-64 and 256-by-256 matrices, then add dimensions that leave partial
  tiles. Report packing and architectural-state setup separately and in the
  end-to-end result. An integer case needs explicit overflow behavior; a
  floating-point case needs an appropriate independent numeric oracle.

Each prototype must verify hardware and OS support before using its specialized
instructions. A simulator can check selected behavior, but its timing is not a
measurement of the target hardware.

## Correctness and acceptance

Positive cases include empty input, all/none selected, partial vectors, exact
destination capacity, and permitted aliases with defined sequential behavior.
Negative cases include insufficient capacity, expired storage, unsupported target
requests, and an attempted independence claim contradicted by aliases. Use guard
pages or memory tools to detect tail overreads; inspect O0 and optimized execution.

For future DSA tests, inject queue-full retries, failed completions, and attempted
reset while work is outstanding. For matrices, test irregular dimensions, strides,
format limits, and the chosen numeric oracle. Do not count emulation timing as
native hardware performance.

Record throughput, latency distributions, instructions, bytes touched, output
size, dispatch/setup costs, and code size. Follow the
[common experiment recipe](README.md#a-common-experiment-recipe). Counters can
help explain results when available; an unavailable counter is not zero activity.

Stop or narrow the experiment if gains disappear against tuned controls, setup
dominates realistic sizes, extra variants cause material regressions, or legality
requires weakening existing semantics. Fewer source operations are not acceptance.

## Goose compatibility review

Use the full [mechanism checklist](../spec.md#goose-performance-compatibility-checklist)
in any eventual campaign. Relevant items for this note are:

| Mechanism | Research status, restrictions, and evidence still needed |
|---|---|
| Checks and target specialization | Main candidate; not a general fresh feature. Prove tails and dependencies; inspect emitted variants and remaining checks. Goose already has SIMD multiversioning. |
| Static abstractions | Prefer statically known predicates and operations; general specialization remains deferred. Account for closures, indirect calls, and code growth. |
| Destination construction and errors | Preserve publication and failure order. Measure output staging, copies, rollback, and error-path work. |
| Flatness, stable growth, and retained references | Preserve admitted storage guarantees; vectorization cannot silently repack or relocate published data. Test valid retained aliases. |
| Bulk reset | Plain-data release stays a fixed set of marker resets. Accelerator completion and resource cleanup are separate explicit obligations. |
| Other mechanisms | Tagged layouts, messages/images, and pools are deferred here. This experiment supplies no evidence for their eligibility or cost. |

## My assessment and open questions

I would prioritize masks and ordered filtering first: the operation is broadly
useful and the correctness question is small enough to examine carefully. DSA is
valuable where asynchronous data movement already fits the application, but it
introduces a resource protocol. Matrix facilities deserve a compute-library path
when Holo has a real matrix workload.

The unanswered design question is how programmers request specialization while
keeping unsupported targets, code size, and semantic restrictions understandable.
We should obtain one measured result before selecting permanent syntax. Related
notes cover [layouts](layouts.md), [batched traversal](batches.md), and
[cache policy](pages.md).

## References for implementation work

The linked vendor guides above explain capabilities, not Holo speedups. Also use
[LLVM's vectorizer documentation](https://llvm.org/docs/Vectorizers.html) for
legality checks and optimization diagnostics, and the
[Intel instruction manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html)
for exact instruction semantics. Pin the consulted revisions, compiler, CPU,
operating system, and flags in a prototype record; these web links can change.
