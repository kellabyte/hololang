# Independent test support

This directory contains reference values, an evaluator, and deterministic program
generation for the first checked region slice. It is test infrastructure, not a
second compiler backend or a runtime dependency.

The [reference interpreter](model/README.md) uses safe Rust values and independent size accounting.
`generator.rs` varies bounded nested-record workloads from seeds; `edges.rs`
provides structurally different explicit boundary/cursor fixtures. Native process
execution and failure-artifact capture live in [`native_vs_reference.rs`](../semantics/native_vs_reference.rs).
`owner_fixtures.rs` owns four established experimental Holo source cases and the
shared fixture type/producer text. `recovery_fixtures.rs` adds six acquisition
recovery cases, including the maintained recovery source fixture. Together they
provide ten hand-derived outputs/acquisition counts for [`owner_lifecycle.rs`](../runtime/owner_lifecycle.rs).
New recovery cases cover replacement-owner construction and cleanup before outer
continuation, the same source's successful acquisition tail, and both outcomes of
a conditionally terminating handler. A nested retry-failure case distinguishes
two injected NULL returns from the one successfully acquired outer owner; each
fixture states its exact fault count explicitly. These files keep source scenarios separate
from native process/allocator instrumentation; they neither implement record
operations nor derive expected values from compiler layout data.
`byte_input_fixtures.rs` combines the maintained byte-input source fixture with
focused alias, failure, empty, and bounds scenarios. It owns hand outcomes (including
explicit layout-dependent overflow statuses and exact expected trap identities)
for [`byte_input_native.rs`](../compiler/storage/byte_input_native.rs); it neither supplies a native algorithm nor uses
compiler schema planning to calculate expected data.
An identity helper's ignored invalid argument must execute for a nine-byte
initializer spanning a possible eight-byte vector prefix and scalar tail, while
an empty initializer skips it; returning the indexed byte does not authorize
erasing other eager observations. The selected out-of-backing guard remains the
trap oracle rather than an inferred vector or copy shape.
It must not import the compiler's checker, layout calculator, or LLVM emitter to
derive expected outcomes. The native tests provide the same input program to the
compiler and model, then compare deterministic observable results.

Generate and preserve complete inputs as well as seeds. Native address values are
not deterministic expected outputs; test address stability through comparisons
inside a run. Model identity and relative locations are only test witnesses.

Run these helpers through the integration tests documented in [tests](../README.md).

`refill_adapter.c` supplies deterministic read, recv and poll witnesses for
[`hosted_refill.rs`](../runtime/hosted_refill.rs). It checks the production adapter's native arguments
and outcomes, including one call per attempt and untouched spare bytes. Real
socket pairs exercise admission independently of the injected syscall outcomes.
`send_adapter.c` witnesses one send or writable wait with injected positive,
interrupted, would-block, and erroneous results. Its real Unix socket pair
checks exact binary bytes and per-socket disconnect signal suppression.
`hosted_stream.rs` queues real data on a Unix stream pair and hands the native
host a connected nonblocking descriptor. Hosted and locked-package tests share
the helper to observe half-close and slot reuse through the production adapter.
`tcp_close.rs` and `tcp_close_adapter.c` link checked source and IR to the
real TCP close adapter with test-only syscall substitutions. Real Unix
stream pairs witness EOF. Injected `EINTR` follows an actual close and
replacement at the exact same descriptor number. Synthetic listening
calls reach native admission rejection without requiring loopback bind.
The many-client case forces the first descriptor number on later entries.
On Darwin, the real queue observes each client while a test-only
`kevent` wrapper injects a saved earlier event or a failed second attachment.
The fixture verifies listener and queue release and stops before the third
admission on setup failure.
[`tcp_concurrent_adapter.c`](tcp_concurrent_adapter.c) injects a failed
worker startup and a broker error while admission is blocked. It checks
that the listener closes, only launched threads are joined, and the
host transfers no stream after either failure in both concurrent modes.
The same fixture exercises the fixed permit FIFO at capacity and across
index wrap, checking reservation order and full release.
Run the focused matrix with `cargo test --offline --test hosted_tcp tcp_close`.
[`tcp_parked_reset.rs`](tcp_parked_reset.rs) runs the checked
[`tcp_parked_reset.holo`](tcp_parked_reset.holo) worker on real loopback.
The Rust harness supplies the target's tagged reset status and applies
abortive linger only to client descriptors. The read and write reset
outcomes and the surviving peer's exact echo and EOF are separate gates.
Run it with `cargo test --offline --test hosted_tcp checked_parked_slots_recover_after_read_and_write_resets -- --ignored`.

The byte-input fixtures include a dynamic copy boundary sweep over 0–34
and 63/64/65/127/4,096, with complete byte assertions, both separate and same-prefix
receiving storage, writable source aliases, and retained reference addresses.

`byte_sum_differential.rs` generates packed-record byte sums with a fixed seed
and supplies per-record u128 oracle results for [`byte_sum_native.rs`](../compiler/backend/byte_sum_native.rs).
Its O0/O3/ASan runs compare the ordinary and outlined LLVM paths where the
host supports the outlined kernel. The native offset witnesses establish
coverage of all 16 alignment residues without importing the layout planner.
`static_byte_sum.rs` checks the isolated tiny call-site specialization against
ordinary execution, with literal and dynamic prefixes and a writable alias.
`large_byte_sum.rs` checks the 256-byte minimum for literal checked prefixes,
dynamic and computed fallbacks, eager bounds failure, and exact native tails.
