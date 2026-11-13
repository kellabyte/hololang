# Compiler backend tests

These suites compare specialized and ordinary LLVM lowering on equivalent checked
programs. They check observable semantics and selected code shapes. The native
cases require LLVM and an OS linker. The explicit layouts are `Wide`, `Derived64`,
and `Compact32`. ASan means AddressSanitizer.

Run from the repository root.

```sh
cargo test --offline --test staging_copy --test uniform_batches --test byte_sum_native
```

## Staging copies

[`staging_copy.rs`](staging_copy.rs) checks the opt-in staging copy through the
independent scalar model and native O0/O3/ASan in all three layouts. It covers both
short and bulk disjoint copies, overlapping aliases in both directions, tiny and empty
loops, and identical emission on an unsupported target.

<details>
<summary>Proof boundaries, copy semantics, and outlined bulk calls</summary>

- **Stable slots and proof selection.** Stable-slot cases cover one-to-four-byte
  transfers with hoisted descriptors, zero-trip enclosing loops, and rejection of cursor
  shadows and rebindings. The proved-only candidate preserves ordinary LLVM for unproved
  loops. A mixed loop also verifies that an eligible proof leaves its other overlapping
  transfer scalar and preserves forward-copy behavior.

- **Trap order.** Test-only observations distinguish source and destination traps and
  verify the valid prefix writes before a late out-of-bounds index.

- **Constructed-slot cursor proof.** The ordinary Derived64 cursor case proves a
  successful constructed-slot copy and checks exact bytes against the independent model
  at O0, O3, and under ASan. Negatives decline the proof when a re-created slot can
  shrink under a carried cursor, the receiver boundary resets independently, an
  assertion capture is reassigned, a publication check is missing, or an update violates
  the extent. The unchanged reader gate also checks valid wire digests and
  malformed-input trap behavior.
  [Native qualification](../../../evidence/staging-cursor-proof-20261111/README.md)

- **Outlined bulk calls.** The outlined bulk staging tests compare every byte of a
  two-byte copy and an adjacent 257-byte copy against the model at O0, O3, and under
  ASan. They verify the one changed entry call and the shared private definition.
  Unproved sites, other layouts, and both Linux target triples reproduce ordinary LLVM
  exactly. The ordinary-policy boundary test admits proved maxima of 65 and 256 bytes
  and retains the intrinsic at 64 and 257 bytes, using an initial request bound inside a
  larger initialized slot.

</details>

## Uniform batches

[`uniform_batches.rs`](uniform_batches.rs) checks that the opt-in macOS arm64 Derived64
proof specializes captured cursors only when every append has the same exact extent.
Mixed lengths, computed lengths, other layouts, and Linux retain identical ordinary
LLVM.

<details>
<summary>Rollback, cursor aliases, and hosted reuse</summary>

The native cases exercise typed rejection and capacity rollback, snapshot traversal
during suffix append, reuse after reset, and an out-of-bounds trap. Zero-payload records
retain a positive physical stride, and a live mixed branch retains the variable hop.
Hosted file tests exercise successful refill and EOF with both uniform and mixed output
shapes, keeping earlier owned bytes after input-buffer reuse. A checked-IR case rebinds
the cursor within its batch and observes the original through an alias. Native checks
cover O0, O3, and ASan.

</details>

## Byte-sum lowering

[`byte_sum_native.rs`](byte_sum_native.rs) checks narrow reduction selection for the
same published or borrowed extent, rejects close nonmatches, and retains the
unsupported-target fallback. The native oracle covers all three layouts at O0/O3/ASan
with unaligned fields, aliases updated before the loop, poisoned prefix suffixes, zero
and all 16-byte tail widths, larger spans, and a wrapping u64 seed. These are
correctness checks and do not time the new lowering.

<details>
<summary>Call-site specialization, target controls, and differential oracles</summary>

- **Tiny literal prefixes.** The isolated static call-site trial additionally checks
  literal 0–2-byte prefixes against dynamic and larger calls, including eager bounds and
  unused scalar actuals in [`frontend_evaluation.rs`](../frontend/frontend_evaluation.rs).

- **Large literal prefixes.** The large-prefix cases check the 255/256-byte cutoff,
  dynamic and computed fallbacks, and eager failure on an invalid literal prefix. Their
  independent native oracle covers unaligned loads, exact 64-byte block boundaries,
  nonempty tails, and wrapping seeds in all layouts at O0/O3/ASan.

- **Three-block policy.** The macOS arm64 Derived64 default matches the qualified
  three-block emitter. The explicit policy on Wide verifies one changed literal call
  edge with identical eager argument and bounds LLVM. Neighboring widths, computed
  192-byte prefixes, and unsupported targets retain their prior call paths. The
  unaligned and wrapping oracle runs both policies. The Derived64 default traps when a
  192-byte prefix exceeds a 191-byte backing.

- **Deterministic differential test.** A separate deterministic differential test runs
  212 packed records through the outlined macOS arm64 lowering and ordinary checked LLVM
  lowering at O0/O3/ASan. The same checked source supplies both. The test selects the
  ordinary path with a nonmatching triple, then restores only the target header before
  verification and native linking. O0 exercises the scalar loop. LLVM can vectorize
  ordinary checked loops at O3. The independent u128 oracle verifies each complete and
  borrowed sum, and native offset observations cover all 16 alignment classes for
  vector-sized complete and borrowed spans in each layout. Other hosts check the
  outlined IR and run their native ordinary path. Set
  `HOLO_BYTE_SUM_DIFFERENTIAL_ARTIFACTS` to a new directory beneath `build/` to retain
  the generated input, emitted IR, binaries, tool commands, and native streams.

</details>
