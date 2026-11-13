# Pages: deliberate backing and cache policies

Let programs choose suitable memory policies without changing what Holo values
mean or hiding the work needed to obtain and release physical storage.

**Research note — November 10, 2026.** These are optional policy experiments, not
approved source syntax or implemented fresh Holo features. They depend on the CPU,
operating system, configuration, privileges, and workload. No speedup or latency
bound is established here.

## Why this matters

A program sees virtual addresses. The operating system maps them to physical
memory in units called **pages**. A CPU keeps a limited cache of these address
translations, the **TLB**. Its data caches separately hold recently used contents.

Consequently, two programs with identical values and algorithms can differ in
translation overhead, page faults, cache pollution, and retained physical memory.
Holo's regions offer a useful place to expose such policies: choose backing once
for a collection of data instead of adding machinery to every object.

The goal is a measurable optional implementation choice. A larger page or a cache
hint cannot fix unnecessary data movement; first consider [layouts](layouts.md)
and [batched access](batches.md).

## Four ideas to keep separate

### 1. Huge pages reduce translation work

A larger page lets one translation describe more memory. As an illustrative
calculation, a fully populated 1 GiB range contains 262,144 pages at 4 KiB each,
or 512 pages at 2 MiB each. These numbers describe page coverage, not guaranteed
TLB occupancy or a predicted speedup. Actual page sizes are platform-specific.

This can help large, frequently accessed working sets. It can also waste memory
for sparsely used ranges or add allocation and compaction delays. Linux provides
transparent huge pages, including multiple sizes on supported configurations, and
explicit HugeTLB pools. A huge-page hint is not proof of actual huge backing.
[Transparent huge pages](https://docs.kernel.org/admin-guide/mm/transhuge.html),
[HugeTLB](https://docs.kernel.org/admin-guide/mm/hugetlbpage.html)

For Holo, test region-sized backing policies with observed page information.
Keep both a strict request that can fail and a separately explicit best-effort
policy conceivable; neither spelling nor default is selected. Record the mode
that actually ran.

### 2. Reservation, population, and release are different operations

A large virtual reservation can leave room for supported nonmoving growth, but it
does not establish physical availability or zero-cost future access. Population
and first touch can acquire backing and fault pages in. Locking pages can prevent
swapping where supported, subject to resource limits. These are separate promises.
[Linux memory locking](https://man7.org/linux/man-pages/man2/mlock.2.html)

An illustrative lifecycle, not executable Holo syntax, is:

```text
reserve address range
    → obtain/populate required backing
    → construct and use data
    → reset logical region markers
    → optionally retain, discard, or unmap backing under a separate policy
```

Logical reset makes storage reusable. Discarding or unmapping backing is OS work,
and later reuse may need faults or reinitialization. Advice can round to page
boundaries and has operation-specific effects, so a live neighboring object cannot
be discarded merely because another object's lifetime ended.
[Linux memory advice](https://man7.org/linux/man-pages/man2/madvise.2.html)

Prefaulting or locking may improve a measured latency distribution. It does not
prove termination time, freedom from all blocking, DMA accessibility, or strict
`noalloc`. Fresh bump-region storage still counts as allocation under Holo's
intended contract. Device registration has additional rules; see [I/O](io.md).

### 3. Non-temporal stores can protect useful cache contents

Some instructions provide a **non-temporal** store hint for output that is unlikely
to be reused soon. A long output stream might otherwise displace a smaller hot
working set. Appropriate stores can reduce that cache pressure and some memory
traffic, depending on the implementation.

The useful contrast to measure is a write-only export buffer versus an output
immediately consumed by another computation. The latter may benefit from ordinary
cached writes. Small sizes, alignment, partial lines, and publication ordering can
also change the result. Follow exact target ordering and fence requirements; a
source hint is not a synchronization operation.
[Intel optimization manual](https://cdrdv2-public.intel.com/814198/248966-Optimization-Reference-Manual-V1-049.pdf)

Holo could expose the intent through an optional library operation or checked
schedule. The compiler must still preserve valid accesses, dependencies, and
observable ordering. It cannot infer non-reuse from `readonly`, and a store hint
must not change plain data into device or volatile memory.

Cache flush instructions are a separate topic. They are not a generic speedup.
Likewise, a memory fence or cache writeback alone is not a complete crash-durability
protocol; persistence needs its own platform and storage contract.

### 4. Hardware tags add detection, not a new lifetime proof

Arm MTE associates four-bit tags with 16-byte memory granules and compares them
with pointer tags in configured modes. It requires suitable hardware, mappings,
and operating-system setup. Detection may be synchronous or asynchronous.
[Linux MTE documentation](https://docs.kernel.org/arch/arm64/memory-tagging-extension.html)

This is useful optional hardening, especially around unsafe or foreign interfaces.
It is not complete bounds checking: an access within a tagged granule can still
cross a logical object boundary. Tags can also repeat, so an old pointer need not
be detected after every reuse.

Retagging every byte range at region reset adds size-dependent work. It cannot
replace the required fixed-marker reclamation path. A diagnostic mode may have
explicitly different costs, recorded separately; any adopted lifecycle policy
needs review. Ordinary checked Holo must remain correct without successful tag
detection.

## What Holo has today

The [accepted finite M1 core](../implementation-plan.md#m1-acceptance-decision--november-10)
has bounded checked backing and scoped marker operations. It does not implement a
general VM growth/trim policy, portable committed-memory accounting, or the page,
store-hint, and tagging policy interfaces discussed here. Physical reset latency
also remains unresolved in the retained evidence; do not relabel it as a measured
constant-time result.

The [specification](../spec.md#required-baseline-bulk-memory-reclamation) already
separates logical reset, OS-memory release, and external-resource cleanup. Preserve
that distinction. Memory placement across sockets belongs with [locality](locality.md);
large pages do not automatically make memory NUMA-local.

## Who should implement which part?

| Component | Responsibility |
|---|---|
| Language/checker | Preserve lifetimes, initialization, valid references during growth, and explicit effect boundaries. |
| Storage planner/runtime | Track backing extents and eligible policy boundaries without per-object reset work. |
| OS backend | Request page policy, handle finite resources and failure, report actual backing, and implement separately requested discard/unmap. |
| Compiler/backend | Lower a legal cache-store operation with correct alignment and ordering. |
| Diagnostics | Report policy requests, actual admission, retained memory, and unsupported measurements. |

No universal policy is selected. A target-specific helper or system call also
needs its own effect contract; M1's reviewed native profile does not automatically
cover new helpers.

## A first prototype

Start with **page backing for a large fixed region**. Keep store hints and MTE as
later, separate experiments so one result has one clear explanation.

A concrete starting fixture uses 64 MiB and 1 GiB regions, subject to a recorded
memory budget. For dense use, initialize u64 element `i` to `i` and read every
element in sequential and fixed-seed permuted order. For sparse use, touch one
u64 at a recorded stride and report both the extent and touched bytes. Use the
same logical accesses in each page mode and verify all returned values. Do not
silently shrink a failed allocation or call an advisory hint a successful strict
request. Add smaller and intermediate extents before drawing conclusions.

1. Build a native harness with ordinary pages and an explicit huge-page candidate.
   Use sequential scans and randomized accesses to the same initialized data.
   Include dense and sparse use, plus a small range where huge pages should offer
   little benefit. Keep worker placement unchanged.
2. Measure acquisition, first touch, warmed traversal, logical reset, refill, and
   optional discard separately. Observe actual page backing before interpreting
   differences. Do not modify machine-wide page policy as an invisible setup step.
3. Compare tuned C/Rust region controls and the intended pinned Goose checkout
   where a matching policy is available. A missing policy is unsupported, not a
   guessed result. Until integrated, the harness is not Holo-new evidence.
4. Add one explicit Holo backend experiment only if the native mechanism helps a
   representative workload without compromising retained references or failure
   behavior. Use `build/benchmarks/` for working output and record the results
   and their limitations.

For non-temporal stores later, compare write-only output and immediate reuse,
including required fences in the timed path. For MTE later, compare detection and
overhead in an explicitly labeled diagnostic profile, without weakening checking.

## Correctness and measurement

Positive cases should retain and read an existing element reference while growing
within admitted backing, refill a reset region, and preserve initialized neighbors
beside a discarded eligible range. Invalid-lifetime cases should reject reset
with usable references or outstanding device access and attempts to discard live
storage. Separately, runtime tests must report finite-capacity exhaustion,
unsupported strict policy requests, and acquisition or population failure through
the specified error path. Repeat O0/optimized execution with memory tools as useful.

Report these quantities separately:

| Quantity | Meaning to record |
|---|---|
| Payload | Initialized application data, separate from representation metadata. |
| Reserved virtual space | Address range set aside, not physical use. |
| Committed/backing obligation | The platform's actual definition and observable measure; mark unavailable when not established. |
| Resident memory | Physical pages currently resident, including retained backing. |
| Release/refill work | Logical marker operations, OS operations, faults, clearing, and resource cleanup in distinct phases. |

Also retain latency distributions, throughput, page faults, actual page sizes,
and TLB/cache events where available. Keep pressure, NUMA placement, background
promotion/compaction, and thermal conditions in the record. Follow the
[common experiment recipe](README.md#a-common-experiment-recipe) for independent
validation, sequential measurement, provenance, and retained raw samples.

Stop or narrow the candidate if it saves traversal time only by hiding setup,
causes reproducible latency regressions, inflates resident memory beyond the
chosen budget, or relies on unavailable platform behavior.

## Goose compatibility review

Use the full [mechanism checklist](../spec.md#goose-performance-compatibility-checklist)
for an eventual campaign; page policy alone establishes none of its other wins.

| Mechanism | Research status, restrictions, and evidence still needed |
|---|---|
| Stable cheap growth | Candidate backing policy only. Inspect reservation limits, growth bookkeeping, faults, slack, and retained-reference behavior. |
| Flatness and destination construction | Preserve existing eligible storage; backing changes cannot justify payload staging or hidden relocation. Count copies and allocations. |
| Compact layouts | Page size is independent of logical metadata width. Report payload, metadata, alignment, and page slack separately. |
| Bulk reset and errors | Keep fixed-marker logical release; measure OS release and partial acquisition cleanup separately, with failure injection. |
| Target paths and contracts | Cache/tag policies are optional and currently unimplemented interfaces. Establish exact native helpers and effects before claiming contracts. |
| Other mechanisms | Tagged alternatives, general generics, images, and pools remain outside this experiment. No new acceptance evidence is supplied for them. |

## My assessment and open questions

I would treat page policy as an application-facing runtime capability. It can be
valuable for large stable regions, but a universal huge-page or prefaulting default
would hide too many workload and machine tradeoffs. Cache hints should follow
measured reuse behavior. Hardware tags belong first in diagnostics and hardening.

Open questions are how callers express a strict versus advisory request, how much
backing the runtime retains, and which observations can be reported portably.
Those need evidence and review before choosing source syntax or a default policy.
The [research map](../research.md) and [compiler architecture](../compiler.md)
remain the broader sources of direction. Pin source-document revisions and the
actual platform configuration when turning this note into a prototype.
