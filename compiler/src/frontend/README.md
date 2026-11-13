# Experimental source frontend

The `parse_experimental` API and the CLI use the same experimental Holo source
path. Preferred vertical `func`/`with`/`arg`/`returns` headers and
brace bodies lower into the existing checked construction IR. Temporary `core.*`
calls are an experimental adapter, not approval of permanent collection APIs.

| Component | Responsibility |
|---|---|
| `mod.rs` | Immutable source-program facade and source-mapped checking. |
| `ast.rs` | Owned, spanned syntax with no implicit semantic approval. |
| `modules.rs` | Explicit local leaf imports, static call resolution, and original-file diagnostics. |
| [modules](modules/README.md) | Package namespace traversal and file-scoped import binding. |
| [parse](parse/README.md) | Separate declaration, expression, and statement grammar. |
| [functions](functions/README.md) | Schema resolution, ordinary signatures, closed scalar templates, and destination constructors. |
| [lower](lower/README.md) | Entry and ordinary bodies, typed actuals, structured control flow, and operation spans. |

The lexer and shared diagnostics live in `../lex.rs` and `../source.rs`. The
frontend produces one ordinary IR representation and invokes the existing checker;
LLVM still requires the checker's private authorization token. No archived code is
imported. Compiler trees, maps, spans, and inlined templates allocate host memory.

Compilation parsing collects naming diagnostics from declaration tokens before
namespace binding rewrites identities. `SourceProgram::naming_diagnostics`
borrows them in source order. An application collects advisories in its entry and
reachable root-package namespaces. Explicit library-root mode collects only
entry advisories. Entryless package checking can render advisories from all root
contributors after checking succeeds. Advisories stay in the frontend and have
no generated representation. The formatter uses parsing without collection.

`parse_experimental_project` adds an explicitly rooted local library experiment.
An entry starts with `using holo.<leaf>` and calls `leaf.function(...)`. The leaf
source is `<library-root>/<leaf>.holo` and exposes `public func` declarations. An
optional `namespace holo.<leaf>` header precedes its functions and must match the
requested namespace.
Other helpers are private to that file. Imported files currently contain only
functions with `u64` results and existing `u64` or readonly borrowed `Bytes`
argument modes. They can call local functions and checked `core.*` operations.
The resolver rejects duplicate `using` directives, bindings that shadow a
qualifier, and non-regular leaves including symlinks. It assumes a stable trusted library root
and its parent paths. Source resolution performs no ambient search or dependency
fetch. Diagnostics retain the originating file and span. The resolver rewrites
calls to private compiler identities before existing frontend lowering and
checking, so the generated program requires no namespace lookup or package state.

`parse_experimental_package` checks an entry and the namespaces reachable in its
manifest graph. Each namespace selects either one source file or a
directory of contributing files. The file form is available at the source
root. A nested path such as `holo.net.tcp` selects immediate files in
`net/tcp/` and uses `tcp` as its local qualifier. Contributors share
private function names and retain their own `using` directives.
Qualification reaches public functions from the same package or a direct
dependency. The frontend reports identity mismatches,
dependency cycles, qualifier collisions, and duplicate declarations in their
original source files.
It borrows the loader's snapshots and reuses the same call binding and lowering.

`check_experimental_library_package` checks a package without an entry. Every
root Holo file contributes its path-selected namespace, including functions with
no callers. Imported dependency namespaces retain normal reachability. Both
entry and library checks validate the same declaration IR. A library check
returns a diagnostic or success and never supplies an executable entry.

Both package and standalone source support experimental `Result<u64, E>` with
nominal families whose cases can carry one named `u64` field. A family can
include child families with `from Child as Alias`. Scalar `let value = try call()`
binds a success value and returns a failure under the enclosing result family.
`on Alias.Case(value) then …` selects a child case, including its payload when
present. `on … then` checks all cases or an explicit fallback and evaluates
just the selected arm. Public families can cross manifest package imports.
[Runnable dashboard union](../../../tests/fixtures/error_unions/README.md)

The typed append path also accepts a producer returning
`Result<Record, E>`. A fresh `return ok Record:` initializes the destination
in declaration order. Typed guards can reject with `return error E.Case(value)`;
a nested record field uses `try child(...)` to propagate a typed failure
in the same family or from a declared direct child.
A constructor can also forward its whole result with a single `return call(...)`
statement. The resolved record and error family must exactly match. The callee
constructs in the caller's existing destination.
An immediate append handler binds a scalar or boolean decision in entry. Its
arms see the batch after publication or rollback. Manifest library exports
also admit typed producers whose namespace-local record and error family are
both public. Byte-bearing and nested inline fields use the same checked
layout and construction as standalone records. Every child named by a public
record must be public in that namespace.

Entry can also construct a scoped aggregate local with
`core.construct(region, producer(...))` followed by block `on` arms. The
call resolves the region tail before evaluating its actuals. A readonly
record reference is available only in the success block. Failures retain
the typed constructor or storage outcome, and the region tail is available
for retry in a failure block.

One `with core.region(bytes) { … } else { … }` statement directly in
`main` supplies a private backing to one `core.construct(producer(...))`
site. The owner evaluates its byte budget once and the recovery runs if
acquisition fails. The site can be revisited inside a counted loop without
reacquiring the backing. Placement elaborates to the ordinary checked
region and scoped constructor operations. [Source rules](../../../docs/spec.md#supplying-a-stack-budget-in-source)

The experimental hosted TCP entry uses `with core.tcp_stream() as stream
{ … }`. The name resolves only in the owner body. Explicit refill uses
`from stream`, and explicit send, readiness, and close receive the
stream as their first operand. The existing checked byte ranges and
refill outcomes remain the transfer boundary. The checker handles owner
consumption and entry cleanup. [Hosted lowering](lower/README.md)

The current subset admits `u64`, inline `Bytes`, acyclic named records, readonly
copied scalar helper arguments, pure scalar returns, and fresh record returns into
append and scoped-local destinations. Record producers additionally admit `Bytes as readonly
reference` inputs from complete published byte-field projections, live refill
data bindings, or forwarded producer parameters. `core.len(input)` and
`input[index]` observe these bounded borrows. Explicit `core.bytes` construction
initializes owned output. Ordinary
borrowed functions return scalars and accept readonly byte inputs or writable
fixed scalar-record state. `core.prefix(field, count)` forms a checked logical
extent in a byte actual. Main has no arguments, declares `returns u64`, and uses
cleanup-aware `return 0`. Pure and ordinary helpers can satisfy `noalloc` and
`noblock`. Destination producers acquire storage and can satisfy only `noblock`.
Unknown calls and contracts reject. Main contracts remain outside this subset.

Every entry call argument is captured once in written order before pure scalar
template substitution. Producer byte descriptors retain rooted locations/extents,
without payload snapshots. Their metadata loads can occur at append because the
restricted argument effects cannot change those facts. Producers can read their
explicit initialized byte inputs but cannot write published data or invoke unknown
effects. Default infix arithmetic is rejected while
numeric policy remains open. Byte initialization and writes explicitly use
low-eight-bit narrowing. `let` prevents rebinding; `var` permits scalar or same-root
reference rebinding. Neither changes path permissions or makes a reference unique.
Same-batch published-prefix inputs are legal during suffix append. Standalone
byte views, writable byte parameters, reference returns, and user-owned input
handles remain outside this subset. Pure scalar templates still take copied
scalars only.
Scalar-helper calls in byte-bearing producers retain eager scalar actuals in a
checked scoped expression, so ignored/repeated parameters preserve input-read
bounds traps and once-only evaluation. This adds no runtime closure or payload
buffer; existing scalar-only helper substitution remains unchanged.

After this same checking boundary, the schema planner can select explicit `wide`,
`derived`, or `compact` physical modes. Source structs and calls are unchanged;
the CLI defaults to full-width `derived` and its `--layout` flag selects alternatives.
`inspect-layout` prints widths,
symbolic offsets, and retained-header reasons without LLVM. Compact uses checked
u32 payload lengths; derived preserves u64 lengths. These are experimental
compiler configurations, not newly selected source layout syntax or a stable ABI.

Implementation limits bound parser nesting (128), lowered owner nesting (64), and
definition/entry expansion (separate budgets of 100,000 charged units). These
budgets count admitted expressions/template expansion, with schema/construction
nodes additionally charged by definition lowering. They do not cap all AST, span,
temporary-binding, or operation allocations; those also scale with source input.
Limits produce diagnostics rather than fallback to another runtime. The checker
rejects changed-root reference assignment, even when a future analysis might
establish safety. `core.now`, `address`, and `opaque` are harness facilities.

From the repository root:

```sh
cargo run --offline -- check tests/fixtures/region.holo
cargo run --offline -- run tests/fixtures/region.holo --release
cargo run --offline -- inspect-layout --layout derived tests/fixtures/region.holo
cargo test --offline --test lex --test parser --test frontend --test frontend_native --test cli
cargo test --offline --test modules
```

Check-only uses no LLVM tools. Native tests/builds require matched LLVM and an OS
linker. See the [source specification boundary](../../../docs/spec.md#experimental-source-subset)
and [source fixture guide](../../../tests/fixtures/README.md).
