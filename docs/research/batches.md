# Batches: hide memory latency with independent work

Let a CPU make progress on several independent lookups while it waits for memory.

**Research note — November 10, 2026.** This is an exploration plan, not an approved
schedule syntax or an implemented Holo feature. The [specification](../spec.md)
defines semantics; the [implementation plan](../implementation-plan.md#m2--first-finite-compiler-improvement-accepted)
decides whether this becomes an M2 experiment. Start with the shared
[research guide](README.md) for campaign and evidence conventions.

## The problem in plain language

A tree lookup reads a node before it knows which child to read. When the next node
is outside the CPU's caches, the CPU may spend much of that lookup waiting.
Faster arithmetic does little to shorten that dependency chain.

However, a server often has many unrelated lookups available. The CPU can request
memory for one lookup, work on another, and return when the first request is ready.
This uses **memory-level parallelism**: several memory accesses overlap in time.

This is different from adding threads. The initial experiment should interleave
work on one thread. It is also different from SIMD, which applies an instruction
to several values together. The techniques can eventually compose.

## A small example

These are conceptual timelines, not executable Holo or a promise about exact
hardware timing. Each letter names an independent lookup.

```text
One at a time:
  A: read root, wait, read child, wait, finish
  B: read root, wait, read child, wait, finish

Interleaved:
  request A's root
  request B's root
  use A's root; request A's child
  use B's root; request B's child
  finish A and B in their original result positions
```

A practical implementation might keep a fixed number of active slots. Each slot
contains a query ID, current node location, and traversal state. When a query
finishes, its result is placed at its original index and that slot can take another
query. A prefetch hint may start fetching a node before the slot needs its data.

The slot count is a tuning parameter, not a language constant. Too few slots fail
to cover latency; too many increase state, cache pressure, and scheduling work.
Prefetching also consumes bandwidth and cannot erase a dependency whose address
is still unknown.

For example, 16 queries could share four active slots. There are still 16 ordered
results, but only four traversals are active at once. This does not create four
threads or require 16 heap tasks.

## Why it could matter for Holo

Good candidates include batched hash-table probes, tree searches, graph frontiers,
and database indexes with working sets larger than cache. This is less promising
for tiny cached structures, already sequential scans, or computation-heavy work.

Holo could let the programmer keep the logical traversal understandable while a
checked schedule controls how independent traversals overlap. The compiler would
retain dependencies and effects long enough to establish legality, then lower a
bounded state machine. A heap task or allocated coroutine per lookup need not be
the experiment's representation; inspect actual generated storage before making
any allocation claim.

Fresh scratch-region storage still counts as allocation under strict `noalloc`,
even when no general-purpose allocator is called.

The Cimple paper is evidence that this approach can work. Its evaluated database
algorithms reached up to 6.4× single-thread speedup. That number belongs to its
workloads and controls, not to Holo or every pointer-heavy program.
[Cimple](https://arxiv.org/abs/1807.01624)

## What Holo has today

The finite, single-threaded M1 region core is
[accepted under its reviewed native profile](../implementation-plan.md#m1-acceptance-decision--november-10).
It does not implement a general tree/index library, traversal scheduler, or
prefetch scheduling language. The [research map](../research.md#r06--latency-hiding-for-irregular-access)
already identifies independent traversal interleaving as a candidate.

Holo's approved M1 access rules permit aliases. A `readonly` path does not freeze
the same data through other aliases, and `readwrite` does not establish uniqueness.
A transformation must prove independence for the actual operation. It cannot add
LLVM `noalias` merely to obtain better code.

## Keep the responsibilities clear

| Component | Possible responsibility |
|---|---|
| Language/checker | Define legal reordering and establish dependencies, initialization, lifetimes, and observable error order. |
| Compiler | Represent traversal states, specialize known callbacks, choose or honor checked schedules, and emit prefetches where valid. |
| Library | Supply the collection algorithm, batch interface, result ordering, and explicit capacity policy. |
| Runtime policy | Eventually coordinate workers and placement; unnecessary for the first single-thread experiment. |

The [compiler architecture](../compiler.md#4-compute-simd-and-optimization) is the
implementation boundary. This idea does not justify another compiler backend or
a second unchecked execution path.

## Start here: one small prototype

1. **Choose a read-only lookup workload.** Use a fixed index and a pre-generated
   query stream. Define absent keys as ordinary results, so the first experiment
   does not change recoverable-error or trap order. Prevent mutation during the
   call through the workload's established permissions, not the word `readonly`.
2. **Separate storage from scheduling.** Keep the same data layout, key distribution,
   query order, numeric behavior, and result format across schedules.
3. **Build matched controls.** Compare tuned C and Rust sequential traversal,
   interleaving without prefetch, and interleaving with prefetch. Use the intended
   pinned Goose checkout where the operation is supported. Label unavailable
   competitors explicitly.
4. **Sweep a small batch range.** Try active-slot counts such as 1, 2, 4, 8, 16,
   and 32. These are experimental inputs, not recommended defaults. Include
   working sets fitting in cache and exceeding it, plus mixed hit/miss queries.
5. **Only then consider Holo lowering.** First record which source/IR operation is
   missing. A host-language feasibility kernel is not Holo-new evidence. If
   authorized later, add one narrow, checked experimental path and compare it
   against the same logical operation without the transformation.

Use an independent result oracle, such as sorted key/value data searched by a
separate reference algorithm. Validate every result before interpreting timings.
Keep input generation and index construction outside the lookup timing, but
report their costs separately.

### A first fixture with the choices already made

These numbers are starting points for an experiment, not intended defaults:

- Build a balanced binary search tree containing 1,048,575 keys: the even numbers
  from 2 through 2,097,150. Each value is `3 × key + 1`, computed exactly as u64
  arithmetic.
- Use flat nodes with a u64 key, u64 value, and two u32 child indices. Reserve an
  out-of-range index as the absent-child sentinel. Shuffle physical node positions
  with Fisher–Yates and a fixed SplitMix64 seed of 1, then repair child indices.
  Pin the generator implementation and hash the resulting image.
- Generate 1,048,576 queries with a separately seeded stream, seed 2. Alternate
  randomly selected stored even keys and absent odd keys from 1 through 2,097,149.
  Use the same generated stream for every competitor. Exactly half hit; misses
  spread through the tree instead of all following the same out-of-range path.
- The independent oracle checks range and parity: an even key in the stored range
  returns `3 × key + 1`; every other key returns an explicit absent result.
  Competitors search the actual supplied index; the formula belongs only to the
  independent oracle. Preserve query order.
- Begin with active-slot counts 1, 4, and 16. Add smaller and larger trees to
  distinguish cache-resident behavior from memory latency. Record actual cache
  capacity instead of assuming this first tree exceeds it.

## Correctness cases that must survive

| Case | Required result |
|---|---|
| Empty input, absent keys, duplicate queries, and a partial final batch | Same values and result order as sequential execution. |
| Several queries read the same node | Valid sharing remains accepted. |
| Retained element reference across supported collection growth | Existing address and value behavior remain valid; scheduling cannot weaken growth guarantees. |
| A callback mutates data needed by another query | Preserve order or reject this schedule unless independence is established. |
| A later query could fail before an earlier one under interleaving | Preserve specified observable failure order; do not assume finishing order is harmless. |
| Scratch or input storage could be reset while active slots use it | Reject the escape/reset or establish completion before release. |

Prefetch is a hint, not permission to create invalid pointers or bypass bounds.
Any target-specific safety assumptions need explicit documentation and tests.

## What to measure

Measure throughput and per-query latency distributions, especially low-load and
tail latency. Batch waiting time counts toward latency even if the inner kernel
is faster. Report active-slot bytes, stack use, spills, code size, allocations,
and copied bytes. Inspect assembly to see whether the intended overlap exists.

Where real counters are available, record cache misses, memory stalls, branch
behavior, and bandwidth. Mark unavailable counters unavailable. Run native
competitors sequentially, vary their order, retain raw samples, and record CPU,
compiler/backend versions, flags, source identities, and input hashes.

Stop or narrow the idea if scheduling overhead dominates representative workloads,
gains depend on an impractically large batch, latency regresses materially, or
legality requires weakening current semantics. A win over a poor sequential
implementation is insufficient; keep the tuned control.

## Compatibility with the storage baseline

Review the full [Goose mechanism checklist](../spec.md#goose-performance-compatibility-checklist)
with each experiment. The relevant initial accounting is:

| Mechanism | Status, restriction, and evidence still needed |
|---|---|
| Data stacks, stable growth, and marker reset | Intended to remain unchanged. Count scratch backing and inspect reset paths; no per-task frees or hidden relocation. |
| Flat data and destination construction | Preserve the index representation and construct results directly where supported. Account for slot/result staging and copies. |
| Narrow metadata, typed links, and tagged alternatives | Keep the chosen representation fixed in the schedule comparison. General links and alternative layouts remain deferred. |
| Static callbacks and target checks | Candidate later specialization; inspect dispatch, closure storage, remaining bounds checks, and prefetch legality. |
| Error paths | Preserve failure order and cleanup. A faster successful path cannot hide more costly or unsafe failures. |
| Queue messages, validated images, and pools | Deferred; interleaving alone establishes none of their eligibility or identity rules. |

## My assessment and the remaining questions

This is a strong M2 research candidate because it tests a general mechanism without
requiring concurrent shared mutation or an I/O runtime. The first objective is a
repeatable scheduling benefit, not new syntax.

The hard questions are how to establish independence without unnecessary rejection,
how to identify schedule targets robustly, and how much control users need over
batch size. Automatic batching also needs an application latency contract. I would
start with an explicit batch interface and a narrow operation whose legality is
easy to explain.

## References worth opening

- [Cimple: Instruction and Memory Level Parallelism](https://arxiv.org/abs/1807.01624)
  explains interleaved tasks and the evaluated database algorithms. Read its
  controls and scheduling details before reusing the headline speedup.
- [LLVM vectorizers](https://llvm.org/docs/Vectorizers.html) explains existing
  backend vectorization and legality checks. It helps separate a new scheduling
  contribution from optimizations LLVM already supplies.
- [Goose reference](https://aardappel.github.io/goose/docs/reference.html)
  defines the comparison language's documented mechanisms; pin the actual checkout
  before a campaign instead of treating this moving link as source provenance.
- [Holo schedules](../spec.md#10-algorithms-and-execution-schedules) records the
  intended separation between algorithm meaning and checked execution strategy.
  It does not select a complete scheduling language.

For adjacent ideas, see [layouts](layouts.md) and the
[research index](README.md).
