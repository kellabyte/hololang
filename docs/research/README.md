# Hardware research for Holo

Six practical starting points for making modern hardware useful through clear,
checked systems code.

**Prepared November 10, 2026. Status: research.** These notes expand the six ideas
from the hardware discussion into explanations and future experiments. They do
not approve language rules, select public syntax, implement features, or report
new Holo performance results.

## Pick a question

The numbers match the original discussion. Read each note on its own, or follow
the connections below.

| Item | Note | Question it explores |
|---|---|---|
| 1 | [Batches](batches.md) | Can independent lookups take turns so the CPU spends less time waiting for memory? |
| 2 | [I/O](io.md) | Can device queues use buffers efficiently while the checker prevents premature reuse? |
| 3 | [Locality](locality.md) | Can workers, their memory, and device queues stay close enough to avoid unnecessary coordination? |
| 4 | [Layouts](layouts.md) | Can a better arrangement or smaller representation make the same computation touch fewer bytes? |
| 5 | [ISA](isa.md) | Which vector, matrix, and accelerator instructions deserve target-specific implementations? |
| 6 | [Pages](pages.md) | When do huge pages, cache policies, and hardware tagging help, and what do they cost? |

“ISA” means instruction set architecture: the operations a processor exposes to
software. “Locality” means keeping the data and work that use each other close in
the memory and processor topology.

The main connections are:

- **Batches + layouts + ISA:** expose independent work, arrange its data well,
  then choose suitable machine operations.
- **I/O + locality:** transfer buffer permissions at the device's real completion
  boundary, and place each queue and pool near the worker using them.
- **Pages + everything else:** choose backing and cache policies explicitly;
  keep their OS work separate from ordinary logical region reset.

## Where these notes fit

The [specification](../spec.md) owns language semantics and decision statuses.
The [implementation plan](../implementation-plan.md) owns the current milestone.
The [compiler architecture](../compiler.md) owns component boundaries. The
[research map](../research.md) remains the categorized source map; these notes
add worked explanations and experiment plans without becoming a second spec.

The [finite experimental M1 core](../implementation-plan.md#m1-acceptance-decision--november-10)
is accepted under its reviewed Darwin arm64 compiler/target/runtime profile.
That acceptance covers a restricted single-threaded region implementation.
It does not establish a complete language, general concurrent ownership,
device-I/O integration, or portable native effect verification. The next planned
step is [M2: one measured bottleneck](../implementation-plan.md#m2--first-finite-compiler-improvement-accepted).
Writing these notes does not expand that implementation scope.

Examples marked **illustrative** are explanations, not runnable Holo programs or
selected language syntax. Native C/Rust feasibility experiments can test a
mechanism before Holo supports it; their results must stay labeled as native
controls, never as Holo-new measurements.

## My view of the opportunity

The strongest language contribution is keeping useful facts available long enough
to check and optimize them: which operations are independent, where bytes live,
which layouts preserve references, and when a device has finished using storage.
Individual instructions are then implementation choices with explicit eligibility.

I would start with batches or layouts for a measured M2 experiment. They can
isolate a mechanism without first building a concurrent runtime. I/O and locality
have substantial systems potential, but need more lifetime, synchronization, and
deployment work. ISA and page policies are supporting choices to evaluate on a
specific target, rather than universal defaults.

This is a reasoned ordering, not a measured ranking. A negative experiment can
be valuable if it rules out an attractive but ineffective mechanism.

## A common experiment recipe

Each note supplies its own smallest prototype, correctness cases, measurements,
and stopping conditions. Use this shared recipe when turning one into a run:

1. **Write the hypothesis.** Name the workload, target, expected mechanism, and
   result that would falsify it. Identify whether the change affects execution,
   representation, permissions, or backing policy.
2. **Freeze logical behavior.** State input/output values, numeric rules, alias
   permissions, failure order, lifetime boundaries, and unsupported cases before
   changing code. A faster result with different work is a different experiment.
3. **Build real controls.** Include tuned flat/arena C and Rust controls where
   applicable. Use the intended pinned Goose checkout and its own instructions;
   record unsupported cases rather than substituting another installation.
   Keep Holo-current and Holo-new separate. Compare a straightforward control
   too when it answers a distinct question.
4. **Validate before timing.** Use an independent oracle, meaningful valid and
   invalid cases, failure injection where relevant, and actual output consumption.
   Distinguish model tests, checked-IR tests, parsed-source tests, and real hardware
   execution. Simulated queues do not establish NIC or NVMe performance.
5. **Measure one change at a time.** Separate setup, steady-state work, completion,
   logical reset, and OS/resource cleanup. Run measured competitors sequentially,
   vary their order, and record distributions and regressions. Measure request
   latency separately from whole-process timing when making latency claims.
6. **Explain the mechanism.** Preserve generated code, actual layout, allocations,
   copies, metadata, and memory observations. Counters are useful when available;
   unavailable events are a measurement limit, not an inferred result.
7. **Preserve enough to replay.** Capture commands, raw samples, oracle outputs,
   seeds and input hashes, CPU/device/firmware/OS identities, runtime/stdlib,
   backend and compiler versions, flags, dirty source contents and hashes.
   Include compiler/CLI/runtime sources, workspace manifests and
   [Cargo configuration](../../.cargo/config.toml) in Holo provenance. Discover
   the effective Cargo target directory instead of assuming `target/`.

For memory investigations, report **payload, virtual reservation, commitment, and
resident backing** separately where the platform can measure them. Name the
measurement API and mark unavailable quantities. Smaller payloads or logical
resets do not establish smaller resident memory or fewer page faults.

## Guarantees every experiment must review

Use the full [Goose mechanism checklist](../spec.md#goose-performance-compatibility-checklist),
not just allocation counts or teardown time. Each note identifies its most relevant
mechanisms, restrictions, and missing evidence. A future milestone report must
account for the full set, including deferred mechanisms.

In particular, preserve eligible flat nested data, destination construction,
supported nonmoving growth with retained references, and reclamation through a
fixed set of marker resets. Do not introduce mandatory object walks, reference
counts, relocation, or individual frees for ordinary baseline data. Device cleanup
and OS backing changes have separate lifecycles and costs.

M1 permits aliases. `readwrite` is not uniqueness; `readonly` does not freeze other
aliases; stable addresses do not prove iteration independence or LLVM `noalias`.
Strict `noalloc` still counts acquiring fresh bump-region storage. `noblock` does
not establish a latency bound, and neither contract can silently trust unknown
runtime helpers. These constraints apply to generated code and failure paths too.

## Files, commands, and maintenance

This directory contains maintained Markdown research notes only. It has no runtime
dependency, new compiler stage, or runnable prototype. Future maintained compiler
work belongs in `compiler/`, CLI work in `cli/`, target support in `runtime/`,
Holo libraries in `lib/` when implemented, and executable examples in `examples/`.
Follow [repository ownership](../compiler.md#repository-ownership) when a prototype
becomes a maintained component.

Keep working campaigns and generated diagnostics in a fresh
`build/benchmarks/<topic>-<date>/` directory. When documenting a completed
trial, record its inputs, method, outcomes, and limitations, including failures.

For a documentation-only edit, check local links and heading anchors, balanced
code fences, reference applicability, current status claims, and the directory's
README coverage. No compiler build is needed merely to edit these notes. Future
compiler changes follow workspace formatting, warnings-denied Clippy, and
offline tests as relevant. [Test guide](../../tests/README.md)

External documentation links may advance. Before a future prototype, recheck the
relevant primary sources, pin the exact versions used, and record the access date.
Update the authoritative design or milestone only when an approved decision or
new acceptance evidence changes its status.
