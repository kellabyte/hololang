# Compiler storage tests

These suites exercise storage planning, finite reservations, byte initialization,
and borrowed input. The native tests independently check observable output and
require LLVM and an OS linker. The explicit layouts are `Wide`, `Derived64`,
and `Compact32`. ASan means AddressSanitizer.

Run from the repository root.

```sh
cargo test --offline --test layout_plan --test layout_native --test reservation \
  --test byte_initialization --test byte_input_native
```

## Layout planning and native conformance

[`layout_plan.rs`](layout_plan.rs) checks schema eligibility, positive fixed extents,
empty units, checked static overflow, retained complex headers, and useful inspection
offsets.

[`layout_native.rs`](layout_native.rs) independently calculates exact sizes/outputs for
unary tails (32/16/12-byte fixed parts), mixed retained/derived nested records, positive
empty units, unaligned scalar access, private rollback, stable aliases, captured-prefix
growth, and zero/u32-max/overflow byte counts at O0/O3/ASan. Huge logical lengths are
rejected against tiny backing without huge allocations.

## Reservations and rollback

[`reservation.rs`](reservation.rs) checks exact fits, empty/short capacities, retained
prefixes, ordered capacity/rejection/overflow outcomes, nested rollback and backing
reuse under all layouts at O0/O3/ASan. Expected outputs are hand-derived and checked by
the independent model. The rejected remaining-capacity candidate's two host-only
arithmetic exercises are preserved with its source in the generated experiment archive,
while these two native semantic fixtures remain active against the preserved production
lowering.

## Byte initialization

[`byte_initialization.rs`](byte_initialization.rs) checks all five supported
wrapping/bitwise initializer operations and mixed trees against both hand-derived
complete byte outputs and the independent full-u64 model.

<details>
<summary>Initializer operations, boundaries, and source fixtures</summary>

- **IR and parsed source.** Structured IR covers empty payloads, exact eight-byte
  prefix/tail boundaries (7/8/9 and 23/24), and physical indices through 511; parsed
  source covers helper expansion, high scalar/argument bits, aliases, growth, failed
  reservations, narrow-length rejection, and reset reuse.

- **Mixed lowering paths.** A mixed source fixture places a direct arithmetic
  initializer beside a nested-helper initializer with the same full-u64 actuals, so a
  later vector path and scalar fallback must preserve complete field bytes and partial
  failure.

- **Execution and limits.** All three fixtures execute under all three layouts at
  O0/O3/ASan. Native children have bounded execution and archive their inputs/output on
  failure; the tests make no timing or automatic-vectorization claim.

</details>

## Borrowed bytes and owned copies

[`byte_input_native.rs`](byte_input_native.rs) checks 15 complete source scenarios
against hand expectations and the independent model: owned copying, aliased inputs,
caller mutation, same-batch append, empty payloads, partial rejection/overflow/capacity
rollback and retry, and eager bounds failures. The typed cases add nested truncation and
malformed-header recovery, typed width/size and capacity outcomes, and a real target
trap after one valid copy read. All three layouts run at O0/O3/ASan (135 children).

<details>
<summary>Input scenarios, copy sweeps, and trap ordering</summary>

- **Eager arguments.** An identity helper's ignored out-of-backing actual must still
  trap for a nine-byte input spanning a possible eight-byte vector prefix and scalar
  tail, while empty initialization skips it.

- **Dynamic copy sweep.** A dynamic copy sweep checks lengths 0–34 and
  63/64/65/127/4,096, every output byte, separate receiving backing, same-prefix append,
  caller alias mutation and retained addresses.

- **Offset subranges.** The offset subrange fixtures cover every short length through
  64, longer lengths 65 and 127, zero-byte skipping, capacity failure before input
  evaluation and a late invalid read. The valid cases exercise separate backing and
  same-prefix append.

- **Trap observations.** Trap tests redirect only the specified byte-index guard to a
  distinctive exit, retain ordinary setup assertions, and require exact model errors. A
  finite out-of-backing index makes ASan relevant to load ordering.

- **Replay and limits.** Native children have deadlines and preserve full replay inputs
  on failure. These tests do not measure dynamic copy volume or speed; backing free
  counts remain separate evidence from the
  [owner lifecycle tests](../../runtime/README.md#owner-lifecycle-and-recovery).

</details>
