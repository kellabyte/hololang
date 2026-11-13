# Native runtime tests

These suites run compiled Holo code to check the native driver, generated guards,
resource lifetimes, and hosted I/O. They require LLVM and an OS linker. Optional
IPv4 loopback tests are opt-in. The explicit layouts are `Wide`, `Derived64`,
and `Compact32`. ASan means AddressSanitizer.

Run from the repository root.

```sh
cargo test --offline --test native --test runtime_bounds --test entry_exit \
  --test owner_lifecycle --test hosted_refill --test hosted_tcp
```

## Native driver and runtime bounds

[`native.rs`](native.rs) executes real LLVM binaries and validates native
driver/sanitizer paths.

[`runtime_bounds.rs`](runtime_bounds.rs) checks that dynamic invalid accesses terminate
before memory accesses in both optimization modes and under ASan for all three explicit
layouts. It requires exact target trap signals and linked BRK/UD2 instructions,
successful in-bounds reads/writes, and a separately selected guard witness. Its ASan
invalid index exceeds actual backing, so an incorrectly reached load would be
observable.

[`native.rs`](native.rs) also checks the Darwin arm64 driver's eager-binding link flag.
These are correctness/linkage checks, not latency or portable signal guarantees.

## Early entry completion

[`entry_exit.rs`](entry_exit.rs) compares early entry completion against the independent
model and uses test-only LLVM guards to verify reverse-order region releases before exit
at O0/O3 and under ASan for Wide, Derived64, and Compact32. Nested marker cleanup
remains generated compiler behavior; the guards observe backing ownership only and
perform no data-processing kernel.

The same harness verifies that `break` and `continue` release the regions
inside an iteration in reverse order while retaining its enclosing owner.

## Owner lifecycle and recovery

[`owner_lifecycle.rs`](owner_lifecycle.rs) checks ten actual parsed-source paths against
hand-derived outputs and the independent model, under all layouts at O0/O3/ASan.

<details>
<summary>Recovery paths, fault injection, and cleanup observations</summary>

- **Source paths.** These cover continued outer-alias/cursor use through inner
  failure/reset/reuse, three-owner early return after append rejection or capacity
  failure, and allocation failure after an earlier sibling owner has closed. Acquisition
  recovery also constructs and releases a replacement owner before resuming outer work;
  the identical source checks the successful acquisition tail without injection. Both
  outcomes of a conditionally terminating handler preserve exact cleanup and enclosing
  aliases. The recovery source fixture is included directly, and a nested retry-failure
  case observes two actual NULL returns while releasing only the outer owner.

- **Shared fixtures.** All ten paths run 90 native children. New recovery source
  factories live separately in
  [support/recovery_fixtures.rs](../support/recovery_fixtures.rs), sharing the established
  fixture type and producer definitions without duplicating model or native
  instrumentation.

- **Allocator instrumentation.** Test-only allocator wrappers check LIFO release and
  exact successful acquisition/free counts; a volatile, noinline fault wrapper feeds
  NULL into the real source acquisition-failure branch. Only the model's corresponding
  acquisition flag changes to mirror that external fault; source syntax gains no
  fault-injection feature.

- **Replay and interpretation.** Children have bounded execution and preserve native
  failure artifacts plus compiler/test/Cargo source snapshots. These are correctness
  checks, not timing evidence.

- **Rejection boundaries.** Frontend rejection cases also prohibit handler reset of
  referenced outer data and escape of a replacement owner's borrow; readonly aliases may
  still observe permitted outer-writer mutation during recovery.

</details>

## Hosted I/O

### Refill, send, and typed frames

[`hosted_refill.rs`](hosted_refill.rs) checks parsed source through the native ASan
host, including short-file reads and nonblocking rejection of a FIFO during setup.
Deterministic native witnesses cover one bounded read/receive or readiness call, status
mapping, socket admission, and traps on impossible results. Optimized generated code
also traps before publishing injected invalid adapter results. Source checker cases in
[`frontend.rs`](../compiler/frontend/frontend.rs) cover received-view scope and alias invalidation. Output
witnesses check exact binary suffixes on real nonblocking Unix streams, short progress
and error mapping through injected calls, and signal suppression on disconnect. The
checker rejects advancing a session with a live receive view. The optional two-client
TCP fixture requires permission to bind loopback.

<details>
<summary>Fresh prefixes, scoped retries, and incremental readers</summary>

- **Typed frames.** The hosted typed-frame fixture passes the exact fresh prefix to a
  nested producer and reuses the slot with a stale tail. Real file and nonblocking
  socket runs verify the typed truncation, preserved batch marker and retained record,
  owned payload bytes, and raw End and WouldBlock statuses across all three layouts at
  O0, O3, and under ASan.

- **Scoped frames.** The scoped-frame fixture requests six bytes and receives four while
  two stale bytes remain in the slot. It checks the exact count in a lifted truncation
  payload and retries a one-byte preview in the restored tail. A real file and
  nonblocking socket exercise payload ownership, address reuse, and success-arm release
  across the same layouts and modes.

- **Incremental readers.** The incremental reader fixture limits each read to two bytes
  or the next known frame boundary. Across the same layouts and native modes, files and
  connected sockets verify ordered publication, empty payloads, partial-header and
  partial-body EOF, and rejection of a bad header before a following valid frame.

</details>

### TCP ownership and concurrent clients

[`hosted_tcp.rs`](hosted_tcp.rs) checks lexical ownership and raw-IR restrictions. Its
[native fixture](../support/tcp_close.rs) injects rejected admission and terminal
interruption after a real close and forced fd reuse.

<details>
<summary>Loopback, descriptor reuse, backpressure, and parked workers</summary>

- **Sequential clients and descriptor reuse.** The opt-in IPv4 witness sends 8 MiB
  through checked short writes and readiness waits after observing input EOF. A second
  IPv4 witness reuses one process for 32 distinct client exchanges per backend. The
  deterministic fixture forces descriptor-number reuse, injects an actual prior Darwin
  event, and checks that mid-sequence admission or queue attachment failure releases the
  right resources. Use `cargo test --offline --test hosted_tcp` for the checker and
  lifecycle tests.

- **Concurrent owners.** The concurrent witness holds multiple owners open, verifies
  isolation and the live cap, then rotates through released slots. Its parked variants
  share one execution permit across four live owners.

- **Backpressure.** The backpressure fixture sends two simultaneous 8 MiB replies in the
  concurrent modes.

- **Parked writes.** The parked write witness completes a fast reply while a slow sender
  holds either a full 4 MiB reply or a 64 KiB replay window. It then verifies the exact
  4 MiB reply and terminal EOF from each worker.

- **Parked resets.** The parked reset witness rotates three live owners through one
  permit. It injects real TCP resets during an idle read and a backpressured write, then
  checks a distinct orderly echo and EOF for each wave.

</details>

[Hosted test commands](../../runtime/refill/README.md).
