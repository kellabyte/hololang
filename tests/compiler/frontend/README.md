# Compiler frontend tests

These suites cover the path from Holo source text to checked IR. Lexing, parsing,
checking, formatting, and highlighting use compiler APIs without LLVM. The two
native suites also exercise LLVM lowering and execution.

Run from the repository root.

```sh
cargo test --offline --test lex --test parser --test frontend --test format --test highlight
cargo test --offline --test frontend_evaluation --test frontend_native
```

## Highlighting and formatting

| Suite | Coverage |
| --- | --- |
| [`highlight.rs`](highlight.rs) | Checks lossless borrowed UTF-8 partitions, incomplete/malformed source, nested comments, contextual categories including union and propagation keywords, and fixed selector names. |
| [`format.rs`](format.rs) | Checks preferred headers, comments/literal spelling, statement-newline meaning, grouping, control flow, scoped region outcomes, error union paths, malformed syntax, and idempotence of maintained fixtures. |

These tests need no LLVM and introduce no source-language acceptance rules.

## Lexing, parsing, and source checking

| Suite | Coverage |
| --- | --- |
| [`lex.rs`](lex.rs) | Checks exact tokens/spans, numeric failures, nested comments, and defensive UTF-8 diagnostics. |
| [`parser.rs`](parser.rs) | Checks preferred headers, child error declarations, multiline grammar, body braces, contextual scoped region acquisition and recovery, and bounded malformed nesting. |
| [`frontend.rs`](frontend.rs) | Checks real source acceptance, source-located access/lifetime rejection, contracts, and explicit unsupported cases. |

Definition unit tests under
[compiler/src/frontend/functions/tests.rs](../../../compiler/src/frontend/functions/tests.rs)
check closed-call graphs, explicit modes/contracts, schemas, and bounded specialization.

## Native source and evaluation order

[`frontend_native.rs`](frontend_native.rs) executes source fixtures with independent
expected/model results at O0/O3/ASan, covering nested construction/failure rollback,
aliases, growth, scan/reset/reuse, and owner cleanup before early completion.

[`frontend_evaluation.rs`](frontend_evaluation.rs) replaces emitted clock calls with
test-only monotonic ticks or a distinctive exit to check once-only, left-to-right
argument evaluation, unused arguments, comparison ordering, and byte-destination checks
before RHS evaluation at O0/O3/ASan. Borrowed-call fixtures also verify alias-visible
updates on a reused slot, empty and forwarded extents, and a prefix trap before later
arguments. The instrumentation supplies only clock observations.

<details>
<summary>Construction, typed results, rollback, and scoped placement</summary>

- **Typed results and error families.** The scalar typed-result witness also checks
  exact forwarding, full-width success, a named arm before fallback, and exactly-once
  subject and arm work. Payload and `try` witnesses exercise full-width error data,
  early failure, and retagging through two child families. The checked IR suite rejects
  forged leaves and propagation to an unrelated nominal family.

- **Fixed-record append.** The fixed-record typed append fixture also runs in Wide,
  Derived64, and Compact32. It observes a retained reference, full-width nested
  rejection after rollback, and successful retry. A Derived64 boundary distinguishes
  failure before the child from exhaustion at its field.

- **Multiple fixed children.** The multi-child fixed fixture checks that scalar
  reservations resume at the correct offsets after successful child calls, including the
  unaligned scalar after a headerless empty child. A later-child rejection and a
  last-word capacity failure both restore the marker and published prefix.

- **Mixed layouts and variable children.** The mixed-layout fixture wraps a fixed child
  in a variable parent with leading and trailing bytes. It checks resumed reservations
  at different child offsets, rollback after rejection or a trailing-field `Full`, and a
  retry at exact capacity. Fixed and variable parent fixtures repeat the rollback cases
  with a child family whose tag must be lifted. A variable child followed by a byte
  field witnesses the returned end cursor, a later width or size failure, and an
  exact-fit retry that preserves the retained prefix.

- **Terminal unions and bounded inlining.** The terminal-union fixture uses one child at
  six offsets and composes a lift through another wrapper. It checks both specialized
  and budget-limited propagation paths against the independent model at O0, O3 and under
  ASan. The bounded inlining tests verify atomic selection and rejection of shared uses
  or excessive expansion. The native witness checks ordered failure, exact-fit retry,
  and a retained prefix across Derived64 and Compact32.

- **Aggregate tail forwarding.** The aggregate tail-forwarding witness checks a
  whole-result chain with one retained header, a forwarded variable child before a
  trailing scalar, and an empty record's single header or unit. It exercises exact-fit
  retry after named and capacity failures, preserved input ownership, and a fixed
  terminal child with a lifted error tag and original storage status. The same source
  runs against the model and native O0/O3/ASan in every layout.

- **Scoped results.** Scoped result tests check a direct fixed and byte-bearing
  destination across all three layouts. They cover early return, exact fit, a one-byte
  shortage, size overflow, nested rollback, and ownership after input mutation. An
  effectful argument witness checks left-to-right evaluation before a capacity failure.
  Source and public IR tests reject an escaped reference, an inactive binding, and
  competing tail ownership. The locked package and formatter cases cover a qualified
  constructor with retry and nested arm indentation.

- **Programmatic placement.** The programmatic placement test compares byte-identical
  LLVM against explicit backing for four capacity and acquisition profiles across all
  three layouts. The independent model and native O0, O3, and ASan checks cover repeated
  reuse, failure order, separate live roots, and early release. Checker tests reject
  malformed site paths, escaping references, and private binding capture.

- **Source placement.** The source form compares byte-identical LLVM for fit, shortage,
  zero budget, early exit, and injected acquisition failure. An effectful budget
  verifies setup order and a native probe checks that the owner was released before the
  post-block continuation. The locked-package test covers qualified budget, recovery,
  and constructor calls.

</details>
