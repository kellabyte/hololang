# Locality: keep workers, queues, and data near each other

Coordinate where work runs, where its memory lives, and which device queues it
uses, then measure the cost of crossing those boundaries.

**Research note — November 10, 2026.** This is an execution-policy investigation,
not a selected universal runtime or concurrency rule. The
[specification](../spec.md#11-concurrency-and-asynchronous-resources) leaves worker
placement, sharing, and the memory model open. The
[finite M1 core](../implementation-plan.md#m1-acceptance-decision--november-10)
is accepted; NUMA-aware regions, device queues, and concurrent sharing remain
unimplemented in fresh Holo.

## Why this matters

On some machines, a CPU accesses nearby memory more cheaply than memory attached
to another node. This is **NUMA**, short for non-uniform memory access. A NIC or
SSD may also attach closer to one node than another. A NIC is a network interface
card; an SSD is a solid-state drive. A **node** groups CPUs with memory that is
local to them; the actual topology must be discovered on the test machine.

Even without NUMA, sharing frequently modified queue state between cores can
create cache-coherence traffic. A design that saves a few instructions can still
lose by repeatedly moving data or ownership between cores.

The opportunity is to treat a worker, its queues, and its data as a coordinated
unit. DPDK documents local buffer pools and single-core receive-queue use; SPDK
queue pairs avoid internal synchronization by requiring single-thread-at-a-time
access. These are backend contracts, not universal hardware laws.
[DPDK PMD design](https://doc.dpdk.org/guides/prog_guide/ethdev/ethdev.html),
[SPDK queue pairs](https://spdk.io/doc/nvme.html)

## An example to reason about

Consider a request service on a two-node machine. This is an illustrative
placement diagram, not Holo syntax or a claim about any particular machine.

```text
Node A                              Node B
worker A                            worker B
  |                                   |
queue A                             queue B
  |                                   |
buffer pool A                       buffer pool B
  |                                   |
local request state                 local request state
```

Each worker processes requests using its own pool. Cross-worker transfer is
explicit and measurable. Compare this with a shared input queue and one shared
pool, where workers may contend for bookkeeping and access remote memory.

This arrangement does not always win. A single hot flow can overload one worker,
and transferring work can improve balance. Data partitioning, load balancing,
and locality must be evaluated together.

## How the pieces fit

**CPU placement** chooses where a worker runs. Affinity requests can be limited
by operating-system or container configuration; pinning is not proof of exclusive
use of a core or bounded scheduling latency.

**Memory placement** chooses where new pages are allocated. Under local allocation,
the CPU that first faults a page matters, so a main-thread initialization pass
can defeat a later per-worker plan. Changing a policy does not automatically
move existing pages; migration is separate work. Verify placement instead of
assuming it from the allocation call.
[Linux NUMA policy](https://www.kernel.org/doc/html/latest/admin-guide/mm/numa_memory_policy.html)

**Queue steering** directs traffic toward the intended worker. Receive-side
scaling, or RSS, usually hashes flow information to choose a NIC queue. It
distributes flows; it does not guarantee equal work or split one flow uniformly
across all cores. Kernel software steering is a separate mechanism.
[Linux networking scaling](https://docs.kernel.org/networking/scaling.html)

**Batching** amortizes queue operations and device notifications. Processing up
to a maximum batch size is different from waiting to fill a batch. A latency
experiment must record which policy it uses.

**Cache-line separation** avoids unrelated writers updating different words in
the same coherence unit, often called false sharing. Inspect the actual target's
layout and cache behavior before adding padding everywhere: padding also expands
working sets. Queue ordering still needs correct atomics or other synchronization;
locality is not a memory model.

## What this could do for Holo

Holo could make local ownership inexpensive and cross-worker operations explicit
without requiring every program to use thread-per-core execution.

| Component | Responsibility to investigate |
|---|---|
| Language/checker | Represent queue-use permissions and valid transfer; specify sharing, races, and reclamation before claiming safety. |
| Compiler | Preserve proven ownership and layout facts; inspect generated synchronization rather than infer uniqueness from `readwrite`. |
| Runtime/library | Select workers, pools, affinity, NUMA policy, batching, and idle strategies through explicit configuration. |
| Device backend | Report queue counts, topology, supported sharing, steering, and failure behavior. |

The baseline single-threaded path should not acquire mandatory locks, atomics,
reference counts, or remote-free queues merely to support this optional policy.
Current M1 aliases remain valid; neither `readonly` nor stable addresses imply
exclusive access across threads.

Queue capacity, order, full/empty behavior, partial submission, and ownership on
failure must be specified. See [I/O](io.md) for completion-based buffer lifetimes.
Keeping a buffer local does not establish that its device operation has finished.

## First prototype

Begin outside public language semantics with a reversible native experiment.

For a concrete CPU-only starting point, a request carries a sequence number and
a u64 value. A worker returns the value multiplied by three, modulo `2^64`, along
with the same sequence number. Check every result and exactly-once accounting
against an independent oracle. Begin with two workers and bounded queues of 256
entries; vary the amount of useful processing separately. This synthetic fixture
isolates handoff overhead. It cannot establish application or device performance.

1. Inventory CPU nodes, allowed CPUs/memory nodes, cache topology, and device
   attachments. Record permissions and any topology hidden by virtualization.
2. Implement a deterministic request workload with known outputs. Compare a
   shared-queue/shared-pool control, a private-queue/shared-pool variant, and a
   private-queue/private-pool variant with matched memory placement. Keep useful
   work and resource budgets equal so queue sharing and pool sharing have
   separate comparisons.
3. On a real multi-node system, compare verified local and deliberately remote
   allocation. Initialize pages on the intended worker; record observed placement.
   A single-node host can test queue sharing, but NUMA results are **unavailable**.
4. Add one real NIC or NVMe backend after isolating the CPU/queue mechanism.
   Respect finite queue counts and the backend's sharing restrictions.
5. Sweep worker count, batch size, queue depth, load skew, and idle policy. Keep
   placement fixed while testing batching, then vary placement separately.

Use tuned C and Rust controls with equal safety obligations, logical work,
memory budgets, and busy-polling cores. Compare actual intended Goose programs
only where the pinned checkout supports the workload. A host-language model is
not Holo-new execution evidence. Keep fresh Holo **unavailable** until its missing
concurrency and backend support exists.

## Policies worth comparing

| Policy | Potential value | Cost to expose |
|---|---|---|
| Run to completion on one worker | Avoids an extra inter-core handoff. | A slow request can hold up later requests. |
| Dedicated I/O worker and processing workers | Separates different work and scheduling needs. | Adds transfers, queueing, and possibly remote accesses. |
| Busy polling | Can find completions without waiting for an interrupt. | Consumes CPU and energy, particularly while idle. |
| Interrupt-driven or hybrid waiting | Can release CPU capacity at low load. | Wake-up, transition, and scheduling costs. |
| Local processing with occasional stealing | Can recover capacity under skew. | Migration and shared-state costs; ownership transfer needs checking. |

Do not assume one queue per core is always possible or useful. One queue may
already saturate a device. Hardware threads may share core resources, and CPU
hotplug, worker migration, or device removal can invalidate a static placement
assumption. Those cases need an explicit reconfiguration or failure protocol.

## Correctness and failure cases

First specify the concurrency model. Native controls need independently valid
synchronization; a successful timing run is not a race-freedom proof.

Positive cases should include accepted transfer between workers, partial batches,
empty and full queues, release back to the correct pool, and clean shutdown with
outstanding operations. Test uneven flows and a stalled worker without losing or
duplicating requests.

Negative or explicit-failure cases should include concurrent use of a queue that
permits only one issuer, access after transfer, reuse before device release,
stale pooled identities, unavailable affinity/NUMA requests, and insufficient
queue capacity. A deterministic queue/device model can enumerate handoff and
completion orderings. It cannot prove real-device timing or memory placement.

During reconfiguration, establish who owns every queued request and buffer.
Moving a worker is different from relocating live data; never silently move
published region storage to improve locality.

## Measurements and acceptance

Validate results with an independent request/output oracle. Report throughput
and end-to-end latency distributions together, including overload and low-load
behavior. Also record CPU time, polling cores, queue occupancy, actual batch
sizes, cross-worker handoffs, memory placement, allocation/copy counts, and
retained pool memory. Measure energy and remote/cache-coherence events only
where reliable tools exist; label unavailable counters honestly.

Inspect hot queue metadata layout and generated synchronization. Record startup,
page placement, steady work, draining, and pool teardown separately. Run
competitors sequentially, vary order, and retain raw samples plus topology,
kernel/driver/firmware, compiler/runtime, flags, source and input identities.
See the [campaign rules](README.md#a-common-experiment-recipe).

Prefer the simpler policy if locality gains disappear at representative load,
depend on unmatched dedicated cores, or produce unacceptable tail latency or
memory retention. Retain unfavorable results and identify whether imbalance,
coherence, memory distance, or device saturation explains them.

## Compatibility review

Use the full [Goose checklist](../spec.md#goose-performance-compatibility-checklist)
before implementation. These extensions have no acceptance evidence yet.

| Mechanism | Restriction and evidence still needed |
|---|---|
| Data-stack/region allocation and growth | Worker policy must not add per-object allocation or hidden relocation. Inspect backing, capacity checks, and retained-reference growth. |
| Flat storage and destination construction | Build eligible results in their final local destination; account for every explicit cross-worker copy or serialization. |
| Fixed-marker reset | Prove references and device users have ended. External draining, deregistration, and remote reclamation stay separate from plain-data reset. |
| Compact data and static callbacks | Measure padding, metadata, callback dispatch, and bytes touched; local ownership does not prove automatic specialization. |
| Queues, images, and pools | Define transfer eligibility, publication ordering, stale identity, and reuse costs. These remain separate from scoped reclamation. |

## My assessment and open questions

This is a promising runtime/library direction for high-throughput services, but I
would begin with measurement and a small ownership protocol. The hard problem is
maintaining locality under real load imbalance, rather than merely exposing a
`pin worker` option.

Questions to settle later include the first memory model, how exclusive queue
use is represented, how transfer interacts with region ownership, which policies
may fail, and whether migration should move work, data, or neither. Compare the
benefit with [batched access](batches.md) and [layouts](layouts.md) before expanding
the compiler; [pages](pages.md) covers page-policy costs.
