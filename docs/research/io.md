# I/O: buffers live until the device is finished

Make fast asynchronous I/O safe by tracking who may use a buffer until the
backend's real release event.

**Research note — November 10, 2026.** This is a prototype guide, not an approved
ownership syntax or an implemented Holo I/O API. The
[specification](../spec.md#completion-based-resource-lifetimes) already requires
storage to remain valid while a backend may access it. How Holo expresses that
requirement remains open. The [finite M1 core](../implementation-plan.md#m1-acceptance-decision--november-10)
is accepted; NIC, NVMe, and asynchronous-resource integration remain deferred.

## Why this matters

A device can read or write memory independently of the CPU. This is called
**direct memory access**, or DMA. Instead of moving every byte itself, a program
can submit a small description of the operation and do other work.
Here, a **backend** is the library integration with a particular operating
system or device. A NIC is a network interface card; an SSD is a solid-state drive.

The difficult part is keeping that memory valid. Returning from `submit` does
not mean that a NIC has finished reading a send buffer or that an SSD has
finished filling a read buffer. Reusing the memory too early can corrupt data.

Efficient I/O also benefits from reusing buffers and submitting several requests
together. For example, NVMe can accept several commands after one write to its
notification register, commonly called a **doorbell**. [SPDK submission guide](https://spdk.io/doc/nvme_spec.html)

Holo could make these efficient paths easier to use correctly. The benefit would
be a checked resource protocol, with backend libraries supplying the device
mechanics. A new instruction or a special-purpose networking runtime is not
required to begin exploring it.

## A concrete example

Imagine a service sending a response from a reusable buffer pool. This diagram
describes a proposed protocol; it is not executable Holo syntax.

```text
Available buffer
    |
    | successful submission grants the backend access
    v
In-flight buffer
    |
    | backend-specific event proves that access has ended
    v
Reusable buffer
```

For example, buffer A contains `hello` when submitted. Replacing its contents
with `world` before release could change bytes the device has not read yet.
The service can prepare buffer B meanwhile; it recycles A only after release.

Submission rejection must leave the buffer available. Partial batch submission
must identify exactly which buffers were accepted. A timeout or cancellation
request leaves accepted buffers in flight until the terminal protocol releases
them.

The release event is operation-specific. An `io_uring` zero-copy send usually
produces a result completion and, when its flags indicate one is coming, a later
notification completion that permits buffer reuse. The first completion alone
is insufficient in that case. [Send-ZC manual](https://www.man7.org/linux/man-pages/man3/io_uring_prep_send_zc.3.html)

A multishot operation can produce several results before ending. Its request
lifetime and each delivered buffer's lifetime need separate tracking. Dropping
the task that requested I/O must not destroy memory the backend can still touch.
[Cancellation manual](https://raw.githubusercontent.com/axboe/liburing/master/man/io_uring_cancelation.7)

## What this could do for Holo

A useful design would distinguish these facts:

| Fact | What it establishes |
|---|---|
| Lifetime | The storage still exists. |
| Address stability | Its address will not change during the operation. |
| Registration or pinning | The selected backend has accepted the required memory arrangement. |
| Device accessibility | The mapping, address range, alignment, and device restrictions permit access. |
| Access permission | CPU and device accesses cannot conflict under the operation's rules. |
| Completion and initialization | The reported bytes may now be read or reused as specified. |

One fact does not imply the others. A stable region address is not automatically
a DMA address. CPU virtual addresses and device bus addresses can differ through
an I/O memory management unit, or IOMMU, which translates device addresses.
[Linux DMA guide](https://docs.kernel.org/core-api/dma-api-howto.html)

For a device read, such as sending bytes, the design may permit compatible CPU
reads while forbidding mutation. For a device write, such as receiving bytes,
the CPU must not read unfinished output. These are candidate access rules needing
specification. Current M1 `readonly` does not freeze other aliases, and
`readwrite` does not prove exclusivity. An I/O API cannot infer stronger rights
from those spellings.

| Component | Responsibility to investigate |
|---|---|
| Language/checker | Track outstanding access; reject invalid escape, reset, reuse, and conflicting aliases. |
| Compiler | Preserve the established lifetime and access facts; generate ordinary checked calls. |
| Runtime/library | Own finite request and buffer pools, explicit shutdown, capacity handling, and backpressure. |
| Backend | Define registration, submission, release events, memory visibility, errors, and cancellation. |

Do not silently wait in region reset or add reference counts to plain data.
External-resource draining is separate work with an explicit lifecycle. Likewise,
do not reject stack or region storage by its name alone: its actual lifetime and
the backend's requirements decide whether it is eligible.

## Modern paths worth exploring

**Reusable registered buffers.** Registration can amortize repeated setup for
eligible operations. `io_uring` registered buffers are one concrete mechanism;
their setup and pinned-memory budget must still be counted.
[Registered-buffer overview](https://www.man7.org/linux/man-pages/man7/io_uring_registered_buffers.7.html)

**Zero-copy receive.** Linux `io_uring` ZC Rx can receive payload into userspace
memory while the kernel handles TCP headers. It requires appropriate NIC/driver
support, header/data splitting, flow steering, and receive-queue configuration.
Its refill ring makes buffer return an explicit part of the protocol.
[Kernel ZC Rx documentation](https://docs.kernel.org/networking/iou-zcrx.html)

**NVMe queue pairs.** SPDK exposes asynchronous storage operations and explicit
completion processing. A real storage prototype needs isolated test storage and
its actual DMA and queue requirements. [SPDK driver](https://spdk.io/doc/nvme.html)

These are separate backends, not interchangeable guarantees. A capability request
should report an unsupported fast path clearly rather than quietly changing its
copying or lifetime contract.

## First prototype

Start with a deterministic fake device before connecting hardware.

A useful first model has two eight-byte buffers, one queue slot, and explicit
`accept`, `reject`, `cancel-request`, `result`, and `release` events. Fill each
buffer with an operation-specific byte pattern. Enumerate short event sequences
and check that a released operation cannot release a later reuse of the same
slot. The numbers make races easy to inspect; larger pool and batch sweeps follow.

1. Use a finite pool, a finite submission queue, and explicit operation IDs.
   Model accepted, rejected, partially accepted, and terminal operations.
2. Enumerate completion and cancellation orderings. Give each allocation a
   generation so delayed events cannot release a reused request slot.
3. Add one isolated checker experiment for the access protocol. Require user
   review before adopting a public rule; the experiment must preserve ordinary
   M1 aliasing elsewhere.
4. Connect one real backend, initially `io_uring` sends on a supported Linux
   system. Establish copied-send and batched-send controls before adding the
   zero-copy variant.
5. Investigate ZC Rx or NVMe separately after the first protocol works. Do not
   introduce several device mechanisms into one unexplained performance change.

Use real C and Rust controls with matched buffers, batch sizes, work, and safety
obligations. Use the intended pinned Goose checkout where it supports the case;
label missing APIs **unsupported**. Until fresh Holo actually executes the
protocol, label Holo-new **unavailable**, even if a Rust model passes.

## Correctness cases to keep

| Must work | Must reject or handle explicitly |
|---|---|
| Reuse after the exact release event. | Reuse after submission return or a nonterminal result. |
| Unaccepted buffers remain usable. | Losing ownership information on partial submission. |
| Other eligible work continues during I/O. | Reset, unregister, or free while device access remains possible. |
| Short receive initializes only reported bytes. | Reading the uninitialized remainder as received data. |
| Completion races with cancellation in either order. | Treating cancellation acknowledgement as a universal release. |
| Clean shutdown drains or transfers outstanding ownership. | Task drop silently frees an in-flight buffer. |

Also inject registration failure, pool exhaustion, queue-full responses, request
failure, and stale completion IDs. The fake backend checks protocol behavior;
real-device tests must additionally establish actual visibility and release rules.

Storage completion is not automatically durable persistence. A separate storage
experiment must specify cache, flush/FUA, and recovery assumptions.
[Writeback-cache control](https://www.kernel.org/doc/html/latest/block/writeback_cache_control.html)

## What to measure, and when to stop

Validate transferred bytes with an independent checker, or oracle, before
interpreting timing.
Measure setup/registration, submission, completion processing, explicit draining,
and deregistration separately. Record throughput, end-to-end latency distributions,
CPU time, actual batch sizes, outstanding requests, copied bytes, pool occupancy,
and pinned/retained memory. Use real network traffic, not loopback alone.

Sweep message sizes and load. Copy avoidance exchanges per-byte work for other
costs; Linux's `MSG_ZEROCOPY` documentation describes accounting and notification
overhead, so a small-message loss is plausible.
[Zero-copy tradeoffs](https://docs.kernel.org/networking/msg_zerocopy.html)

Run measured competitors sequentially with varied order. Record kernel, driver,
firmware, device, compiler/runtime, source identities, flags, inputs, and raw
samples using the [shared campaign rules](README.md#a-common-experiment-recipe).
Stop promotion if safety needs a hidden copy, cleanup wait, unbounded retention,
or unacceptable tail-latency regression. Preserve those results.

## Compatibility review

Apply the full [Goose checklist](../spec.md#goose-performance-compatibility-checklist)
when implementing; the relevant entries here remain research work.

| Mechanism | Restriction and acceptance evidence still needed |
|---|---|
| Flat data and destination construction | Construct eligible payloads in their eventual buffer; count staging, serialization, and copies. Device eligibility is additional work. |
| Stable growth and retained references | Do not move or invalidate published data to register it. Prove any in-flight subrange remains valid during allowed growth. |
| Fixed-marker bulk reset | Reject reset while access remains outstanding; external draining/deregistration must not become ordinary marker-reset work. |
| Static callbacks and errors | Inspect request-state, callback dispatch, failure cleanup, and allocations; no automatic `noalloc` or `noblock` claim. |
| Flat messages, images, and pools | Buffer reuse needs stale-identity rules. Flat memory alone does not prove wire-format eligibility, image validation, or durability. |

## My assessment and open questions

This is a strong future systems capability because Holo's lifetime checks could
remove a real barrier to efficient device APIs. I would prove one terminal
protocol before building a general async runtime. The largest risk is hiding
shutdown and cancellation work behind a convenient surface.

Questions to settle later: transferred ownership or checked borrowing; how a
borrow survives suspension; whether request IDs need generations; which failure
paths preserve ownership; and how backend capability requirements appear in an
API. See [locality](locality.md) for worker/queue placement and [ISA](isa.md) for
accelerator queues using the same lifetime principles.
