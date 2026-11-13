# Fresh checked construction core

This directory owns Holo's checked compiler implementation.
This Rust library parses the experimental Holo source subset, checks
construction IR, and emits real LLVM programs. The separate [CLI](../../cli/README.md)
uses the same library through one local dependency. Preferred headers now execute through
this path. The [finite M1 gate is accepted under its reviewed profile](../../docs/implementation-plan.md#m1-acceptance-decision--november-10);
the complete language and stable public APIs remain unfinished.

Follow the [compiler architecture](../../docs/compiler.md),
[approved M1 rules](../../docs/spec.md#m1-memory-rules), and
[implementation plan](../../docs/implementation-plan.md). Their full-language open
questions are not silently resolved by this internal compiler API.

## Components

| Component | Responsibility |
|---|---|
| [source.rs](source.rs), [lex.rs](lex.rs) | UTF-8 byte spans, caret diagnostics, checked literals, comments, and tokens. |
| [highlight.rs](highlight.rs), [format.rs](format.rs) | Borrowed lexical presentation categories and syntax-preserving formatting of standalone, entry, and local library source. |
| [frontend/](frontend/README.md) | Spanned parsing, restricted pure functions/constructors, and source-to-core lowering. |
| [package/](package/README.md) | Local manifests, dependency resolution, immutable source snapshots, and lockfile verification. |
| [ir.rs](ir.rs) | Logical schemas, destination constructors, structured scopes, and explicit observations. Producer actuals distinguish published fields, forwarded formals, and live refill prefixes. |
| [placement.rs](placement.rs) | Experimental programmatic selection of one private scoped construction destination before ordinary IR checking and lowering. |
| [check.rs](check.rs), [check/](check/README.md) | Private checked-program boundary, definition graphs, initialization, lexical roots, and access permissions. |
| [layout.rs](layout.rs) | Checked schema planning, explicit wide/derived/compact modes, and inspectable physical costs. |
| [backend/llvm/](backend/llvm/README.md) | Native lowering, packed projections, construction, publication, cursor traversal, and marker cleanup. |
| [driver.rs](driver.rs) | Matched tools, canonical target verification, code generation/linking, ASan, and compiler commands/timings. |
| [lib.rs](lib.rs) | Small public module boundary without archived implementation imports. |
| [CLI main.rs](../../cli/src/main.rs) | Check/inspect-layout/emit/build/run commands with explicit output ownership and observed phase timings. |

Keep new components in focused files and document each actual subdirectory. Source
locations/diagnostics, parsing, schema storage planning, lowering, and the driver
retain separate responsibilities. Shared types have an owning module; there is no
catch-all compiler state or utilities file.

## Implemented boundary

`placement::place_scoped_aggregate` inserts one private finite region around
an entry suffix and one typed construction at a supplied insertion path.
It verifies the candidate with the existing checker and reserves the
generated binding for its owner and designated site. The API returns ordinary
IR, which still goes through `check::check` for backend authorization.
The profile supplies capacity and recovery explicitly. See the
[placement experiment](../../docs/implementation-plan.md#automatic-placement-experiment).

Generated failed checks use LLVM's target trap before protected invalid access,
with no Holo logging/cleanup before host exception transfer. Darwin arm64 native
links request eager chained binding so a contracted first memory-helper call
cannot introduce lazy loader work. Source contracts still use the closed checked
graph; complete native acceptance belongs to the explicitly reviewed finite
target/runtime profile, not arbitrary libc versions or interposed environments.

- Acyclic schemas containing unsigned 64-bit fields, inline bytes, and nested records.
- Acyclic destination constructors with typed scalar/readonly-byte parameters,
  field-order initialization, byte comprehensions, nested calls, and recoverable rejection.
- Synchronous ordinary functions with copied scalars, readonly byte prefixes,
  writable fixed scalar-record state, and a scalar return.
- Entry-body `core.print` supports decimal scalars and double-quoted UTF-8
  literals. A text literal lowers to static bytes and a host output call.
- Explicit bounded regions and one active batch per region; backing is acquired
  once per region scope and never reallocated as records are appended.
- Explicit acquisition handlers may finish the failed owner's block and resume
  outside it, retry with a separately written owner, or terminate entry. No
  failed owner or success-only local is exposed; main still returns zero explicitly.
- Readonly/readwrite aliases, field/byte access, retained references during append,
  conservative lexical lifetime checking, and reset rejection with live references.
- Private construction, publication only on success, fixed-marker suffix rollback,
  reset/reuse, and structured cleanup.
- Linear captured-prefix cursor scans; indexed borrowing is O(index), so repeated
  indexed borrowing must not be described as a linear scan.
- Explicit IR wrapping arithmetic, branches, counted loops, and assertions. These
  do not choose default source arithmetic or collection API spellings.

Changed-root reference rebinding is explicitly **unsupported** in the first checker;
same-root rebinding works. It is not a new language prohibition. Producers cannot
express global/external mutation or arbitrary calls, making their effect boundary
structural. The frontend separates pure scalar helpers, ordinary borrowed
functions, and fresh record-returning constructors. Scalars use
`u64 as readonly copy`; constructors additionally admit `Bytes as readonly reference`
from complete live published fields or forwarded byte parameters. Typed byte inputs
retain their source root and extent without payload snapshots. Internal scalar
`Apply` frames preserve once-only eager helper arguments, including unused bounds
checks. Ordinary calls capture a live byte field or a forwarded byte formal,
optionally check a `core.prefix` extent, and invoke a closed helper graph without
region acquisition or external effects. The callee can mutate existing fixed
state and return a scalar. The adapter checks `noalloc`/`noblock` on those closed
forms, rejects `noalloc` on allocating constructors, and supports cleanup-aware
early `return 0` from main. General parameter modes, generics, and automatic
source-to-data-stack assignment remain unsupported.

## Representation and cost boundaries

The preserved wide format stores an eight-byte total-size header per record,
eight-byte unsigned fields, and an eight-byte length plus payload for bytes.
Nested records remain inline. Access is packed/unaligned (`align 1`); metadata
cannot be written as an ordinary field. An outer id plus nested byte record costs
32 metadata/scalar bytes plus its payload.

The source CLI defaults to `derived`, which removes total-size headers for fixed records and
records with one variable final field, retaining u64 byte lengths. `--layout
compact` uses the same schema policy with checked u32 byte lengths. The nested
benchmark shape occupies 16 or 12 bytes before payload respectively. Multiple
variable fields and variable non-tail fields retain total-size headers. Empty
elided records have one initialized unit byte for positive cursor progress.
`inspect-layout` reports widths, symbolic offsets, fixed/minimum extents, and
retained-header reasons without LLVM. Both modes share the existing checker and
one parameterized backend; there is no alternate ownership/runtime model.

Compact lengths above u32::MAX fail with overflow status 2 at their field's
construction point, before truncation. Other layout modes preserve u64 lengths.
Actual capacity outcomes can differ with representation size; failed appends
still restore the unpublished suffix and preserve old values/references.

`--layout wide` and the existing library `llvm::emit` preserve the original control.
Library clients select new modes with `llvm::emit_with_layout`. The source default
changed after correctness checks and a matched campaign; narrow lengths remain
explicit. These configurations are experimental rather than a stable binary ABI.

Capacity/size guards remain explicit. Allocation/free concern backing scope entry/
exit, while batch release resets one marker without visiting its records. A
zero-capacity logical region uses one physical byte to keep its owner non-null.
Reserved/committed/resident memory and allocator rounding are not inferred from
logical used bytes. There is no VM-reservation growth path or measured parity yet.

`Nanoseconds`, `OpaqueRegion`, and address observations are harness-facing IR
operations, exposed only as experimental `core` harness calls. The optional C support supplies only a
clock and optimizer boundary; all record algorithms are LLVM generated. Pure
constructors cannot invoke the clock or name arbitrary published references; they
read only their explicit scalar/readonly-byte inputs. Instrumented test
builds and required safety checks remain distinct.

## Validation

`lex::scan` losslessly partitions UTF-8 text into borrowed byte extents, including
whitespace, nested comments and unsupported scalars. Strict `lex::lex` adapts the
same scanner into owned parser tokens and validates u64 literals; tool scanning
does not change acceptance or numeric rules. `highlight::highlight` is a streaming
iterator with fixed context and no source-sized allocation. Function/type/field
categories use lexical hints rather than semantic resolution; malformed source
remains displayable. The CLI/editor owns colours and output.

Contextual source diagnostics escape terminal control characters in paths,
messages, and excerpts. Source tabs retain their four-column display. Reported
columns count original Unicode scalars, while carets account for escapes in the
displayed excerpt. The programmatic diagnostic still stores the original message.

`format::format` accepts parsed experimental syntax, retains exact non-whitespace
token/comment text and declaration order, and normalizes indentation, spacing,
vertical headers and parsed block/field boundaries. It reparses output and checks
AST structure without source spans before returning it. Significant newline
meaning, names, literal spellings and expression grouping remain protected. This
allocates token storage, syntax trees and output; boundary sorting is O(b log b).
It performs no line-width reflow or semantic/lifetime checking and is not an
allocation-free or fully general Holo formatter.

Run `cargo test --offline -p holo --test highlight --test format --test lex
--test parser --test source_tools`. The [highlight API benchmark](../examples/README.md)
measures host scanning/classification separately from terminal rendering and
native-program performance.

```sh
cargo fmt --check
cargo clippy --offline --all-targets -- -D warnings
cargo test --offline
cargo run --offline -- run tests/fixtures/region.holo --release
cargo run --offline -- inspect-layout --layout compact tests/fixtures/region.holo
cargo run --offline -p holo-compiler --example region -- build/examples/region
```

The [tests](../../tests/README.md) exercise valid/rejected IR, independent model/native
comparison, failed acquisition/construction, aliases, traversal, and ASan. Complete
replay artifacts are preserved on differential failures. Native tests require
installed LLVM; failures or missing tools are not silently skipped.

Broader source semantics/root analysis, target-specific alignment/relative links, general
source-level effects, and wider workload/architecture comparisons remain M1/follow-on
work. The [frontend guide](frontend/README.md) records supported syntax and
implementation limits. Do not label this increment a completed language milestone.
