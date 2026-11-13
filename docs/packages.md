# Packages and namespaces

**November 11, 2026.** The experimental CLI builds local packages with checked
`using` directives and a lockfile for resolved dependency versions and sources.
Package mode includes an experimental slice for public fixed-size records.

This document owns the focused package and namespace design discussion. The
[specification](spec.md#2-decisions-carried-forward) records approved decisions
and defines the status labels used here. The [implementation plan](implementation-plan.md)
controls executable scope; this document adds no new M1 acceptance gates.

## Selected public prefix

**Preferred — user-approved November 10, 2026:** the public namespace prefix for
Holo's standard library is `holo`.

The prefix gives officially supported library facilities a recognizable home.
The intended breadth includes text, parsing, time, files, networking and other
standard facilities, rather than limiting the name to compiler primitives.
The exact inventory and API boundaries remain proposals.

For example, a future HTTP module could have the public path `holo.net.http`,
and a future text module could have the path `holo.text`. Choosing the prefix
does not approve those individual modules, their APIs, a
package-manager design or a physical source layout. Whether `holo` may be
shadowed, and how its ownership is enforced, still need resolution rules.

`std`, `lib`, `core` and `lang` were considered as alternative standard-library
prefixes; the selected spelling is `holo`. A distinct low-level public `core`
surface has not been selected. Internal implementation layers can be designed
around actual dependencies without requiring a second everyday library root.

## Identifier naming

The [approved source naming conventions](spec.md#identifier-naming-conventions)
apply to package/module namespace components: lowercase names, with `snake_case`
for multiple words. The same policy uses `PascalCase` for named types and enum
cases, and `snake_case` for functions, methods, variables, parameters and fields.
Explicit named constants use `SCREAMING_SNAKE_CASE`. The specification owns the
full convention, acronym spelling rules and remaining open details. Compilation
applies naming warnings to the selected package's checked sources, with
`--strict-names` for enforcement. To check a dependency's declarations against
the same policy, select that dependency's manifest as the root.

Source-to-file mapping and published package identities have their own rules.
The namespace map below describes proposed modules.

## Names at different boundaries

**Working vocabulary; the relationships still need specification.**

| Concept | Purpose | Illustrative name |
|---|---|---|
| Published package identity | Identifies a distributed library independently of a consumer's shorthand. Registry, publisher and source rules remain open. | `northstar/json` |
| Resolved dependency | Identifies the actual selected package, source and version used by a build. Compatibility and instance identity remain open. | A particular selected release of `northstar/json` |
| Project dependency name | Gives a dependency a convenient name within a consuming project, under the alias proposal. | `json` |
| Public module path | Organizes the supported API presented to consumers. It need not describe the implementation files. | `holo.net.http` |
| Local binding | Provides the qualifier or selected name used within a source scope. Binding and collision rules remain open. | `http` |
| Physical source location | Places implementation files in the repository. General module-to-file and package-to-directory mappings remain open. | `lib/holo/bytes.holo` |

Separating these concepts would allow readable source names while preserving
inspectable provenance. It would also allow a library to reorganize private
files without automatically changing its public paths. This is a working
proposal, not an implemented resolution or compatibility guarantee.

The Rust compiler library also happens to export the name `holo`, and the CLI
executable is named `holo`. Those existing host-language and tool names do not
implement the Holo-language standard-library namespace.

## Candidate standard-library map

**Working proposal:** this map shows possible public modules. The experimental
`holo.bytes` and `holo.net` modules have source today. The map makes no
commitment to implement each facility or to map public names to source
directories.

```text
holo
├── memory                  Storage and region interfaces
├── bits                    Bit operations and representations
├── math                    Numeric functions
├── bytes                   Raw byte views, searching and comparison
├── text                    Ordinary text operations
│   ├── ascii               Explicit ASCII operations
│   ├── utf8                UTF-8 validation and traversal
│   └── unicode             Broader Unicode operations
├── parse                   Scanning and numeric parsing
├── format                  Formatting into chosen destinations
├── collections             Arrays, maps, sets and queues
├── algorithm               Sorting, searching and transformations
├── encoding
│   ├── json
│   ├── csv
│   ├── base64
│   └── hex
├── compress                Compression facilities
├── hash                    Noncryptographic hashing
├── random                  Explicit pseudorandom sources
├── crypto                  Cryptographic facilities
├── time                    Durations, monotonic instants and timestamps
│   ├── calendar            Dates and civil-time arithmetic
│   └── zone                Time-zone rules and conversions
├── io                      Streaming and buffered I/O
├── fs                      Files and directories
│   └── path                Path manipulation
├── process                 Arguments, environment and subprocesses
├── net                     Addresses and networking foundations
│   ├── tcp
│   ├── udp
│   ├── dns
│   ├── tls
│   └── http                HTTP messages, parsing, clients and servers
├── thread                  Native threads
├── sync                    Synchronization
├── os                      Platform-specific facilities
├── log                     Structured logging
└── testing                 Tests and supporting tools
```

Keep namespaces centered on coherent responsibilities. A module can expose
several related types; `holo.time` would not need a separate subnamespace for
every time type. Split modules when they have useful independent interfaces or
dependencies, rather than mirroring every type or implementation file.

The proposed text split distinguishes raw bytes, encoding validation and broader
Unicode work. String representation, indexing, view lifetimes and allocation
rules remain open in the [core-values agenda](spec.md#7-types-expressions-and-errors).
The proposed time split distinguishes elapsed time, wall-clock timestamps, civil
calendar operations and time-zone rules; exact types and policies remain open.
Threading and synchronization still require the language's broader sharing and
memory-model decisions. Listing a namespace does not settle those rules.

## Illustrative `using` directives and use sites

The experimental local slice binds the last path segment of a `using` directive
as the qualifier. Manifest packages support nested paths. Explicit `as` aliases
for collisions remain open.

```holo
// Design sketch for library modules beyond the implemented holo.bytes.
using holo.text
using holo.time
using holo.net.http
using holo.encoding.json
```

These sketches would let callers name `text.trim`, `time.Duration`,
`http.Request` and `json.decode`.
The alias proposal could provide a different local qualifier:

```holo
// Alternative design sketch for a file that needs a distinct JSON qualifier.
using holo.encoding.json as standard_json
```

That file could use `standard_json.decode` under the alias proposal.

Community libraries could use the same logical-path and qualification style.
One proposal would let project metadata bind short dependency names to published
identities, source locations and version requirements, with exact resolution in
a lockfile. Other options include publisher-qualified paths in source. No
registry scheme, GitHub-path requirement, manifest syntax or alias mechanism
has been selected.

Short names would need good provenance tooling: a reader should be able to
inspect the resolved provider, version and public export behind a binding.
Aliasing a dependency cannot by itself establish compatibility between its types
and another package or version. Public re-exports and dependency type identity
need explicit rules.

## Declaring a namespace

**Preferred.** Use `namespace` to declare a source namespace. In the local
library experiment, `lib/holo/bytes.holo` declares its full name before its
function declarations.

```holo
namespace holo.bytes
```

The caller selects that namespace with `using holo.bytes` and qualifies its
public functions with `bytes`. The loader maps the directive to `bytes.holo` in
the selected `--library-root`. If the file declares a namespace, its full name
must match the requested path. Files with no header take their namespace from
the directive and leaf mapping in this experiment.

Manifest packages use the same keyword with `namespace <package>.<path>`.
A library file declares its namespace before its file-scoped `using` directives.
A single-segment namespace can occupy `<leaf>.holo` at the source root.
Any namespace can occupy the immediate `.holo` files in the matching
directory. For example, `net.holo` declares `holo.net`, and
`net/tcp/*.holo` declares `holo.net.tcp`. Each contributor declares the
same namespace. A namespace's child directories are resolved independently.
The compiler rejects two source shapes for one namespace and duplicate names.

Plain `func` declarations are available across files in that namespace.
`public func` makes a function visible to files that select the namespace.
A `using` directive binds the final path segment as the file-local qualifier.
Each file can select namespaces from its own package and its package's direct
dependencies. The namespace dependency graph must be acyclic.

## Local manifests and locked builds

The hosted telemetry package selects its entry and depends on the local
standard-library package. [Example manifest](../tests/fixtures/telemetry_worker/holo.toml)

```toml
[package]
name = "telemetry"
version = "0.1.0"
source = "src"
entry = "main.holo"

[dependencies]
holo = { path = "../../lib/holo", version = "^0.1" }
```

The source directory is relative to the manifest, and the entry is relative
to the source directory. Library-only packages can omit the entry. A
dependency key matches its package's declared name. A dependency path selects
a local manifest directory, and an optional SemVer requirement constrains
the version declared there. The compiler resolves the graph deterministically
and rejects conflicts between different providers with the same package name.

`holo lock --package <holo.toml>` explicitly writes the
sibling `holo.lock`. The lock records the selected dependency locations,
versions, and SHA-256 fingerprints of their manifests and source snapshots.
`--locked` validates those selections and uses the same source bytes it
checked. The dependency fingerprint includes the manifest and every
`.holo` file in its source directory, including files outside the current
namespace dependency graph. Root source edits can continue under a locked
build. A dependency change requires a deliberate lock refresh. Registry
fetching, publishing, and dependency aliases remain open.
[Package implementation](../compiler/src/package/README.md)

### Checking a library package

A manifest with no `entry` can be checked directly.

```sh
cargo run --offline -p holo -- check --package lib/holo/holo.toml
```

The checker loads all of the root library's `.holo` files using the
source-root and nested-directory namespace rules. It verifies each namespace declaration
and checks public and private functions. The `using` directives in any of those
files can reach dependency namespaces through the ordinary package graph. When
a manifest has an entry, checking starts with that file and follows its reachable
namespaces.

Either check accepts `--locked` to verify the dependency selection against
`holo.lock` before checking the code. Checking uses no LLVM and emits no
executable. Commands that emit or run a native application require an entry.
[CLI package commands](../cli/README.md#local-packages)

## Public types across packages

**Working proposal.** Give each declared record, enum, or error type a nominal
identity. Within one build, the identity consists of its resolved package
instance, declaring namespace, and type name. A package instance is the
provider selected by the package resolver. The current local resolver admits
one provider per package name and checks every incoming version constraint.
In a locked build, the source fingerprint verifies the selected input. The
compiler resolves a declaration in that verified snapshot once and uses its
identity wherever it appears.

Several files can name the same type through the same namespace. Moving the
declaration between files in that namespace preserves its identity within the
selected source. Two declarations with identical fields or cases have distinct
identities. The checker accepts a typed argument, destination, or result only
when its identity matches the one required at that point. Diagnostics should
show the expected and supplied qualified names and locate the offending use.

### Visibility and storage

`public struct` exposes a record across namespaces in the experimental package
profile. A plain `struct` is visible across files of its declaring namespace.
A file names a public type through the qualifier bound by `using`. The compiler
rejects a public constructor that returns a private record, including
when the constructor is unused. A public record can name only public
types in its inline fields. Checking each such field also prevents a
private type from being exposed through a chain of public structs.

The experiment admits transparent records with `u64`, `bool`, owned `Bytes`,
and fields naming enums or acyclic inline structs in the same namespace.
`public enum` exposes a nominal enum. For example, `protocol.Command.Get`
names a case of `Command` from the namespace imported as `protocol`. The compiler
checks every field even if the record has no constructor or caller. Callers
can read public record fields through a checked readonly reference and
write through a checked readwrite reference. Public visibility grants a
name. Access to storage still comes from a live, initialized reference with
the applicable permissions. Private fields and opaque records need separate
rules before they can enforce invariants for types such as I/O handles.

Use the same checked record representation, layout planner, and destination
construction path as single-file source. Resolve type identities and
visibility at compile time. The native representation should carry no
package identifiers, lookup tables, or type tags for an ordinary record.
Use source-qualified names in diagnostics and layout inspection.
Construction into a selected batch keeps the existing capacity, rollback,
and marker rules. The shown pure initializer supports `noblock` in the
existing subset. Destination construction obtains fresh region storage and
counts as allocation under strict `noalloc`. A public declaration provides a
source interface. Native ABI, independent object files, and persistent or
wire representations need their own compatibility rules.

### Fixed-record experiment

The [package tests](../tests/compiler/packages/packages.rs) exercise this syntax with a locked
local dependency. The owner exposes a record and constructs it in
caller-selected storage.

```holo
namespace wire.frame

public struct Header {
    payload_length u64
    kind u64
}

public func make_header
    with noblock
    arg length u64 as readonly copy
    arg kind u64 as readonly copy
    returns Header
{
    return Header {
        payload_length: length,
        kind: kind
    }
}
```

The application manifest declares the `wire` dependency. In this experiment
the caller uses a qualified type as the schema argument to `core.batch`.

```holo
using wire.frame

func main
    returns u64
{
    let memory = core.region(256) else {
        return 0
    }
    let headers = core.batch(memory, frame.Header)
    let status = core.append(headers, frame.make_header(42, 0))
    core.assert(status == 0)
    let header = core.borrow_readonly(headers, 0)
    core.print(header.payload_length)
    return 0
}
```

Index the namespace's record declarations before resolving its constructors and
the caller's schema selection. Resolve both uses to the same checked schema
identity before ordinary lowering. This experiment uses record declarations in
manifest-driven namespaces, own-namespace public constructor result types, and
qualified schema selection at the caller. Constructors accept the existing
copied `u64` and readonly `Bytes` inputs. Ordinary helpers can borrow fixed
scalar state by writable reference. Use `qualifier.Type` in an argument to
name imported public state. A public helper's state parameter must expose a
public record. Qualified literals, imported field types, and general aggregate
values remain later steps. The existing direct leaf profile has a function
interface.

A provider can also export a forwarding constructor whose body is one
`return make_header(...)`. The checker requires the same nominal record and
the same result family where present. The exported wrapper receives the
destination chosen by the package consumer's append and delegates it to
the inner constructor. Its public signature follows the same visibility rules.

### Typed error families

**Implemented experimental ordinary result slice.** An ordinary call returning
`Result<u64, FrameError>` produces either a copied scalar or one named
failure. The header example below uses tag-only cases. An error case can also
carry one named `u64` field. Use `error`, `return error`,
`return ok value`, and `on … then` from the [error proposal](error-handling.md).
Use `Result<Unit, E>` with bare `return ok` and `on ok` for an operation
with no success value. General enums and additional ordinary result payload
types remain open.

```holo
namespace telemetry.frame

// Classify a complete header. Callers can use its length after admission.
public error FrameError {
    Oversized
    BadMagic
    UnknownKind
}

public func validate_header
    with noalloc, noblock
    arg magic u64 as readonly copy
    arg kind u64 as readonly copy
    arg length u64 as readonly copy
    returns Result<u64, FrameError>
{
    // Preserve the worker's precedence when several fields are invalid.
    if length > 4096 {
        return error FrameError.Oversized
    }
    if magic != 0xA5 {
        return error FrameError.BadMagic
    }
    if kind >= 3 {
        return error FrameError.UnknownKind
    }
    return ok length
}
```

A caller with `using telemetry.frame` can handle every outcome. This example
maps the typed result to the worker's existing numeric diagnostic codes. A
successful call yields code `0`. The success pattern binds the copied length.

```holo
let status = frame.validate_header(0xA5, 1, 42)
    on ok(length)   then 0
    on Oversized    then 3
    on BadMagic     then 1
    on UnknownKind  then 2

core.print(status)
```

The example starts after all four header octets are available. The hosted
worker keeps the decoded length even for rejected frames so it can drain
exactly that body and resume at the next header. A future integration must
preserve that length and the existing order of checks across short reads.
Premature end of stream belongs to the framing or input boundary.
[Hosted telemetry worker](../tests/fixtures/telemetry_worker/README.md)

#### Fixed-record typed append across packages

The manifest-driven experiment also accepts `Result<Header, FrameError>`
as a destination constructor's result. Declare the fixed record and error
family in the constructor's own namespace. Both types must be public when
the constructor is public. Each declaration may live in a separate file of
that namespace.

```holo
namespace wire.frame

public struct Header { length u64 }
public error FrameError { Oversized(length u64) }

public func admit with noblock arg length u64 as readonly copy returns Result<Header, FrameError> {
    if length > 4096 { return error FrameError.Oversized(length) }
    return ok Header:
        length = length
}
```

In a caller with `using wire.frame`, create the destination with
`core.batch(memory, frame.Header)`. The append consumes the constructor
call directly. The success arm observes publication and the rejection arm
receives the copied length.

```holo
let rejected_length = core.append(headers, frame.admit(length))
    on ok then 0
    on Rejected frame.FrameError.Oversized(actual) then actual
    on error then 1
```

The package binder resolves the result's two types before the shared checked
frontend sees the constructor. The record identity must match the receiving
batch. An explicitly qualified rejection case must belong to the same error
family. The package boundary adds no runtime dispatch, aggregate copy, or
storage allocation. Conformance checks compare emitted LLVM against the
same-file source in all three layouts and run the resulting modules at O0,
O3, and with ASan. They check a full-width payload, capacity failure after a
private field write, retry, and retained published references. [Package tests](../tests/compiler/packages/packages.rs)

#### Byte-bearing typed append across packages

The same interface admits the [byte-bearing frame decoder](../tests/fixtures/typed_frame.holo).
Put `Frame`, `Received`, and `FrameError` in the provider namespace and make
each type public. Export `receive` while keeping its `decode_frame` helper
namespace-private. The caller appends to `core.batch(memory, frame.Received)`
and handles the copied failure payload after rollback. On success the
published frame owns its payload independently of the caller's wire buffer.
The conformance fixture runs the same decoder in a locked package and as flat
source. It covers truncation after a parent write, malformed input, capacity
failure at the byte field, exact-slot retry, and a retained published
reference. [Byte-bearing package tests](../tests/compiler/packages/packages.rs)

The hosted conformance case passes a fresh `refill` view directly to the
locked package's typed constructor. The nested decoder sees the actual
read count when the input ends before the request bound. File and stream
runs also exercise capacity failure and exact-slot retry on the reused
receive buffer. Both failures preserve the published marker, count, address,
and payload. Package and flat source emit identical LLVM across Wide,
Derived64, and Compact32. [Hosted package test](../tests/compiler/packages/packages.rs)

Imported type names in record fields and function signatures and ordinary
aggregate values remain outside this experiment.

#### Proposed checking rules

- In this slice, package namespace sources declare error families and
  result-returning functions. The package entry can handle results from another namespace.
  A family declares distinct cases and can include another family with
  `from ChildFamily as Alias`. Its name must be unique among
  the namespace's types. Its identity comes from the resolved package
  provider, declaring namespace, and declaration name.
  `public error` exposes the family and all its cases. A plain `error` is
  namespace-private, including across contributing files. A function signature
  names a family from its own namespace. A union can name a public child in
  another namespace with `from qualifier.ChildFamily as Alias` and a
  file-local `using`. Public unions may include only public families.
  Public signatures require public
  families and public success records, even when the functions are unused.
  `on` resolves case names from
  the result family. Child leaves use `on Alias.Case`, and nested children
  extend the path. Explicit package qualifications use the qualifier bound
  by `using`, as in `on parser.ParseError.Input.BadHeader`.
  Ordinary result functions can take copied `u64` and `bool` arguments
  and use the existing borrowed argument modes.
- The function header declares the exact result type. A returning branch uses
  `return ok value` for a `u64`, `return ok` for Unit,
  `return error Family.Case` for a declared case, or `return call(...)` to
  forward an identical checked result. Forwarding
  a different family requires explicit handling and mapping. Two families
  with the same cases and physical tags remain distinct. Each reachable
  returning path must produce a result or take an existing nonreturning edge.
- An `on … then` expression evaluates its result-returning call once and yields
  the value of the single selected arm. Supply a success arm and cover each
  failure case by name or with the [error fallback](error-handling.md#check-success-with-an-error-fallback).
  Missing, duplicate, unknown, and wrong-family arms are compile errors.
  Adding a public case requires updating downstream handlers that name every
  failure explicitly. `on ok(value)` binds the immutable copied scalar in
  its arm. `on ok` and `on ok(_)` handle scalar success without binding it.
  Unit success uses bare `on ok`. Each arm's value must agree with the type
  of the handling expression.
- An ordinary result call can also be handled as a statement with an
  `on … then { ... }` block for every arm. The selected block may act on
  caller state and return from its enclosing function. Its payload binding
  stays within the arm. The same coverage and package visibility rules apply.
- Ordinary results support direct `on` handling, immutable `let` bindings,
  and exact-result forwarding. `let value = try call(...)` binds a `u64`
  success. `try call(...)` as a statement continues on Unit success. Both
  forms also accept a saved result and can propagate the same error family
  or a declared direct child. Independently constructed error values,
  general patterns, implicit propagation, and integer conversions remain
  open. User-defined generics and ABI or wire encodings need separate rules.

#### Effects and construction

A typed failure is an ordinary return. Existing lexical cleanup applies on
exit, and already executed writes to caller state remain visible. An `on`
arm's effects count on its own path. The result adds no required backing
allocation, blocking, unwind, formatting, logging, or trace. The
`noalloc` and `noblock` contracts still apply transitively to callees,
all reachable success and failure arms, and generated work.
[Effect contracts](spec.md#inference-and-explicit-promises)

The existing `core.append` boundary constructs into an unpublished
destination. On a recoverable failure it restores its marker and preserves the
published prefix. Its current statuses are success, capacity, size overflow,
and rejection. Keep those statuses, the ordering of checks, and the
separate fatal trap path unchanged. The compiler supports
[typed append](error-handling.md#typed-append-in-the-checked-compiler)
with named domain errors and the same final-slot construction for fixed
and byte-bearing records in packages and standalone source. Borrowed read
results still need their own rules.
[M1 memory rules](spec.md#m1-memory-rules) ·
[Synchronous refill](spec.md#synchronous-file-and-stream-refill)

#### Candidate lowering and verification

Every `u64` value, including zero and the maximum, is a legal success. A
general result with any failure cannot encode all of those outcomes in a
single `u64`. Retain the family and variant through checking, then lower an
ordinary result to scalar and tag SSA values. The initial native experiment
uses `{ i64, i8 }` across a direct internal call, with tag zero for success and
nonzero case ordinals. Initialize the inactive scalar to zero and expose it
only on the success edge. The compiler must establish valid alternatives
before using a tag as proof of payload availability. An eight-bit candidate
supports up to 255 error cases. Check widening at the 255/256 boundary before
enabling a larger representation. Source programs cannot observe tag
numbering, register assignment, or the size of an LLVM aggregate as an
in-memory result. At a separately designed wire or foreign boundary, decode
and validate that boundary's own explicit encoding.

Goose's fixed and variable enum forms make different tradeoffs for indexing,
payload size, and interior references. The scalar result slice needs no
decision about arrays of alternatives or their layout. Rust's nominal enum
and match rules illustrate family identity and explicit handling. Zig's
documented error sets and error unions illustrate compact checked returns.
Holo's proposed nominal identity belongs to the declaration. Its case
ordinals are private to a compiled build.
[Goose enum modes](https://aardappel.github.io/goose/docs/reference.html#two-modes) ·
[Rust enumerations](https://doc.rust-lang.org/reference/items/enumerations.html) ·
[Zig error sets and unions](https://ziglang.org/documentation/0.15.2/#Errors)

| Gate | Required evidence for a compiler experiment |
|---|---|
| Checked source and checked IR | Accept all declared alternatives and exact forwarding. Reject family mismatch despite equal names and tags, leaking private families in unused exports, missing or duplicate arms, error-arm payload use, numeric fabrication, and discarding an entire unhandled result. Check an entryless library with LLVM unavailable. |
| Ordinary effects and exits | Run success zero and maximum, every error, an early failure with simultaneous invalid conditions, and a forwarded failure. Observe exactly one call and one arm. Show that prior caller-visible writes survive and that unreachable arms do no work. Reject violating declared effects on both paths. |
| Native return path | Compare package and flat typed source at O0 and O3, including an outlined call and an inlined control. Compare against a matched numeric tag-and-scalar implementation with an independent oracle. Report register moves, stack traffic, branches, backing and helper calls, and copied bytes separately. Include ASan cases where relevant. |
| Existing destination path | Replay the fixed-record append and rollback controls for unchanged record layout, publication, capacity, traps, and retained references. Include existing scalar and worker controls to detect changes outside the result path. |

These forms are exercised through the checked frontend and native backend.
[Scalar result tests](../tests/compiler/frontend/frontend_evaluation.rs)

### Acceptance for the experiments

| Case | Required result |
|---|---|
| Provider declares `Header` and a caller appends the public constructor result to a batch of `frame.Header` | Compile through the real manifest and locked dependency path. The published field values match the same-file control. |
| `north.wire.Header` and `south.frame.Header` have identical fields | Reject a wrong-schema append at the call site. Repeat with two namespaces in one package to verify both parts of the identity. |
| Provider's private record used from a sibling file, from another namespace, or in a public constructor result | Accept the sibling use. Reject the outside access and the leaking signature with their own source locations. |
| Same package resolved along two dependency edges | Preserve the existing single-provider resolution. When signatures can name foreign public types, verify both edges use the same declaration identity. |
| Destination capacity failure, readonly field access, and reset with a live reference | Preserve publication and marker state on failure. Apply the existing access and lifetime checks. |
| Append with a retained reference to a previously published record | Keep the reference usable and its address stable. Inspect capacity and marker work through the same storage plan used by the flat control. |
| Package and flat control under Wide, Derived64, and Compact32 | Compare logical output, record size and offsets, emitted LLVM, and O0/O3 native behavior. Inspect for added metadata, staging, and allocation before attributing any cost to the source boundary. |
| Two error families use the same case spelling or tag | Reject a wrong-family value and access before alternative discrimination. Verify `on` dispatch and any storage or cleanup cost when typed results land. |
| Public package constructor declares `Result<Record, E>` | Resolve both local nominal types. Reject private leaks in unused exports and equal-shape mismatches. Compare package and same-file LLVM and observe typed rejection, partial capacity failure, retry, and stable published references in all layouts. |
| Byte-bearing public record exposes an inline child in a later contributor | Resolve both declarations. Reject private child exposure and unresolved foreign field names. Replay nested typed rejection and byte capacity failure after private writes, then verify owned bytes and exact-slot retry at O0, O3, and under ASan. |

Earlier design controls used app-local records that called a scalar helper in
another namespace. The package tests cover fixed-record type mismatches,
library visibility, rollback, and emitted-code parity. They also check typed
append with the same storage and failure boundaries. The diamond-dependency type
check needs foreign public types in signatures. [Package conformance tests](../tests/compiler/packages/packages.rs)

The [Goose performance checklist](spec.md#goose-performance-compatibility-checklist)
sets the cost boundaries for these experiments.

| Mechanism | Scope and evidence |
|---|---|
| Region backing, stable growth, and bulk reset | Reuses the bounded explicit region and batch policy. The prototype verifies a retained reference across success, rollback, retry, and capacity exhaustion. Reset follows the same live-reference check. Automatic data-stack assignment remains outside this experiment. |
| Destination construction and flat storage | Uses the existing final-slot producer path. Package and flat fixtures emit identical LLVM, including in the rollback fixture. The recorded native comparison produced identical executables. Nested and byte-bearing fields are deferred. |
| Compact metadata and tags | Existing Wide, Derived64, and Compact32 layouts give the prototype the same sizes and offsets as the flat record. Tagged alternative widths and fixed versus sequential layouts remain open. |
| Static calls and checked optimization | Namespace lookup resolves before checked lowering. The same direct calls and optimization path produce the identical code above. New generics, callbacks, and check-elimination rules are outside this experiment. |
| Error propagation | Typed fixed-record constructors use the existing private status and optional `u64` payload lanes. Equal LLVM from the packaged and flat fixtures verifies the package introduces no extra runtime transport. Native O0, O3, and ASan checks cover payload survival and rollback. General error payloads remain open. [Package tests](../tests/compiler/packages/packages.rs) |
| Queue or image interoperability, relative links, pools, and stable handles | Deferred. Each requires its own ownership and representation rules. |

## Socket library

**Implemented experimental module.** Manifest packages can select
[`holo.net`](../lib/holo/net.holo) with `using holo.net`. Its first functions
interpret the experimental hosted socket statuses as typed results.

```holo
let ready = core.prefix(reply.bytes, reply_length)
let raw = core.send(ready, sent, remaining)
let progress = net.send_result(raw)
    on ok(count) then count
    on net.SocketError.Interrupted then 0
    on net.SocketError.WouldBlock then 0
    on net.SocketError.Native(code) then report_error(code)
    on net.SocketError.InvalidStatus then report_invalid_status()
```

The example assumes the reporting functions return a scalar and shows
one possible outcome mapping. A caller that needs to distinguish waiting
from an interrupted retry can assign separate handler results. Send progress
is the accepted count, which can be shorter than the request. Receive
status zero means EOF for a positive-size request. `net.wait_result` reports
readiness with a Unit success, which permits another attempt. A native errno
preserves its positive numeric value on its own platform.
`net.next_session_result` decodes a host-owned session transition. Its Unit
success admits a client, `End` marks the end of a file or inherited stream,
and `Interrupted` permits another accept attempt. The
pure adapters acquire no descriptor, storage, or
event queue. The existing [hosted runtime](../runtime/refill/README.md) owns
descriptors and checks each actual byte range.

### Owned TCP capabilities

**Implemented experimental hosted slice.** `with core.tcp_stream() as stream`
is the sole executable scope of `main`. The host invocation
`accept-loopback PORT` accepts one IPv4 loopback connection, checks
nonblocking mode and disabled send and receive timeouts and linger, and
releases the listener. It hands sole close authority to the binding.
An admission failure exits with process status 2, with cleanup still
owned by the host. Port zero publishes the selected port before accepting.
Darwin also supports explicit `accept-loopback-kqueue PORT` qualification.
[Runnable example](../examples/tcp_echo/README.md)

`accept-loopback-many PORT COUNT` invokes the same checked entry for each
successive client. Darwin's `accept-loopback-kqueue-many PORT COUNT` retains
one host-owned queue across those calls. Every call receives one stream,
and its cleanup runs before the next accept. The listener and readiness
queue remain native resources. Application state and checked borrows are
local to each call. These modes accept a positive decimal count and stop
on the first setup or admission failure.
[Hosted profile and commands](../runtime/refill/README.md)

`TcpStream` is opaque and noncopyable. Safe source has no raw-descriptor
constructor, numeric cast, or duplication. The first slice keeps the owner
in its declaring entry scope. Passing it by ownership, returning it, storing
it in a record or batch, and transferring it across workers are separate
features. When an ownership transfer is eventually supported, it must
invalidate the source binding at the point the destination takes responsibility.
The source spelling for later moves remains open.

#### Concurrent hosted entries

The explicit `accept-loopback-concurrent PORT COUNT LIMIT` host admits
up to `LIMIT` live clients at once. Darwin also supports
`accept-loopback-kqueue-concurrent PORT COUNT LIMIT`. The host uses
bounded native workers. Each checked entry still owns exactly one stream
and its own local storage. One entry's normal return ends only its own
stream. The host keeps listener and readiness ownership across those
calls. Distinct entries share no Holo bindings, initialized ranges, or
authority to close each other's descriptors.

The shared readiness broker routes wait completions by the live
descriptor and wait arm. A wait keeps its owner alive and lends no
payload to the broker. Synchronous transfer and consuming close retain
their ordinary rules. The process-level concurrency of independent
entries introduces no source-level thread, shared reference, or ownership
transfer operation. Native worker stacks and broker synchronization are
costs of this opt-in host profile.
[Concurrent host and commands](../runtime/refill/README.md#concurrent-loopback)

#### Readiness parking experiment

`accept-loopback-parked PORT COUNT LIVE ACTIVE` and Darwin's
`accept-loopback-kqueue-parked PORT COUNT LIVE ACTIVE` separate two
host bounds. `LIVE` limits assigned slots and native worker stacks.
`ACTIVE` limits reservations for running checked entries. The host requires
`1 <= ACTIVE <= LIVE <= 64` and prestarts at most `min(COUNT, LIVE)` workers.

An entry publishes its explicit readiness arm, releases its execution
permit and waits on its existing native thread. Matching readiness makes
the slot eligible again. Admission and readiness publish into the same
FIFO under the broker mutex. A resumed owner reacquires a permit before
continuing checked code. The thread keeps its descriptor and stack through
the wait, including any live region backing and unsent suffix. The broker
retains only descriptor and wait metadata. Checked cleanup closes the
stream before the slot and permit can be reused for another entry.

The profile uses native synchronization and retains one stack per live
slot. It supports cooperative parking at explicit stream waits, with the
ordinary synchronous transfer, resource and effect rules. Queue order is
the order of publication under the broker lock.
[Parking host and commands](../runtime/refill/README.md#readiness-parking)

#### Synchronous attempts and close

Use `refill buffer.wire, limit from stream as received`,
`core.send(stream, buffer.wire, offset, count)`, and
`core.wait_input(stream)` or `core.wait_output(stream)`. Send and receive
borrow the live stream and a positive-size range of
initialized bytes for one attempt. Check the offset, length, and native
count limit before forming the pointer. A dynamic violation uses the
existing bounds trap. Send reads the range. Receive
can write only to the selected range and publishes only the fresh prefix
confirmed by its positive result. A refill view of that prefix follows the
existing view-liveness rules. Neither attempt retains the range or stream
after returning. A zero receive confirms EOF for the input half only.
A positive short send advances the caller's cursor by its exact count.
Interruption and would-block report no progress and leave the owner live.
Readiness permits another attempt, and waiting is explicit.
[Existing checked hosted range and view rules](spec.md#synchronous-stream-output-and-loopback-sessions)

The checker tracks the binding as live or consumed. This slice accepts
`core.close(stream)` as a direct child of the owner scope. An inner block
can contain region-backed I/O and release that backing before the explicit
close. A nested return instead follows automatic cleanup. Later uses of a
consumed binding are rejected. The compiler emits cleanup on each structured
exit without a runtime live flag or handle table. A future implementation of
conditional close must check owner state at each join and loop backedge.

Explicit close consumes the live binding before asking the host to release
it. Both success and a reported close error leave the binding consumed.
With `using holo.net.tcp`, `tcp.close_result(status)` returns
`Result<Unit, CloseError>`. Its native errno
includes interruption and leaves the binding consumed. On normal exit and
early `return 0`, implicit cleanup closes a still-live owner once.
A caller that needs the close status closes explicitly. Fatal traps follow the existing
immediate-trap rule. Resource cleanup is separate from plain-data marker
resets and may perform OS work. The initial slice admits no `noalloc` or
`noblock` promises for acquisition, close, or native transfers without a
separately checked backend effect contract.

The adapter must establish release on the supported target profile.
An interrupted or otherwise failed close must never trigger a generic retry
by descriptor number. The kernel may already have made that number available
for another stream. Linux releases the descriptor before a later close error.
The Darwin host admits one controlled socket with linger disabled and
uses the same single-attempt adapter. The interrupted-close fixture
injects failure after native release and forces descriptor reuse. Closing
does not acknowledge that the peer read the bytes.
[Linux close implementation](https://raw.githubusercontent.com/torvalds/linux/master/fs/open.c) ·
[Darwin descriptor implementation](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/kern_descrip.c)

#### In-flight I/O and connection setup

The first source slice is synchronous. A later submission API must keep the
stream and the buffer range live at a stable address until the matching
operation reaches its backend-defined terminal completion. A pending send
permits compatible reads and excludes writes to its range. A pending receive
excludes both reads and writes through every overlapping alias until it
completes. Reset, refill, relocation, and close are rejected while they
would invalidate a pending operation. Disjoint-range use needs a checked
proof. An event matches an operation identity and stream generation, since
the same descriptor number can refer to different streams over time.

Submission rejected before publication releases the unsubmitted borrow.
An interrupted wait or a cancellation request keeps a published borrow
live. The original request's terminal event ends it, even when cancellation
races with success. A partial batch submission must account for every
published request separately. The first asynchronous experiment should
require explicit resolution before a scope exits. It introduces no implicit
cancel-and-drain on an early return. Readiness alone does not publish data,
and registered buffers, multishot I/O, and zero-copy notifications require
separate lifetime rules. [General completion invariant](spec.md#completion-based-resource-lifetimes)

A future `TcpListener` owns a separate descriptor. Accept borrows it and
publishes an independently owned stream only after admission succeeds. A
future nonblocking connect keeps a pending owner distinct from
`TcpStream`. Immediate success publishes a stream. An in-progress
result retains the pending owner. After a writable, error, or hangup
notification for that live generation, the sole owner reads the pending
socket error and publishes a stream only when completion succeeded.
Terminal failure or an ambiguous interrupted connect closes the pending
socket. The next attempt uses a fresh socket.
[Linux connect contract](https://man7.org/linux/man-pages/man2/connect.2.html) ·
[Darwin socket option implementation](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/uipc_socket.c)

#### Acceptance for the first compiler slice

The implementation reuses checked hosted byte ranges. Only a successful
admission transfers close authority. Typed exits that propagate from an
owner-bearing function remain a future extension with their own cleanup gate.

| Case | Required witness |
|---|---|
| Copy, escape, use after close, or join with conflicting owner states | Static rejection. Valid structured exits still compile. |
| Successful admission, failed admission, and session handoff | Count actual host closes and show one close per owned descriptor across every outcome. |
| Checked IR fallthrough, entry early return, and explicit close with failure or interruption | Preserve the selected result and prove that no descriptor is closed twice, including under forced descriptor-number reuse. |
| Short send, empty receive, half-close, and native interruption | Preserve exact progress and the existing receive-view bounds. The output half remains usable after input EOF. |
| Pending request in a later asynchronous slice | Reject overlapping access and invalidation. Inject cancellation races, partial publication, and stale events and verify that only terminal ownership releases the range. |

### Native socket backend candidates

Linux `io_uring` submission and completion remain candidates. The historical
single-transfer probe held caller storage through a matching terminal
completion. A higher-throughput API could submit multiple checked operations
per worker. The queue, its entries, and the outstanding borrows must remain
explicit. A cancellation completion alone cannot release the original
operation's buffer. Match terminal completions against exact operation
identity, and establish descriptor and buffer liveness before admitting
batching, registered buffers, multishot receive, or zero-copy send.
[Linux io_uring interface](https://github.com/axboe/liburing/blob/master/man/io_uring.7) ·
[Cancellation manual](https://github.com/axboe/liburing/blob/master/man/io_uring_cancelation.7)

For plain TCP on macOS, compare nonblocking BSD sockets and persistent
`kqueue` registration against `poll` with matched work. `kevent` and
`kevent64` return readiness. The socket operation determines the actual
count or error, including queued bytes before EOF. A future scheduler must
associate a delivered event with the same live operation generation and arm
write readiness only when a writer has work. Network.framework has a
separate completion and buffer interface and remains a candidate when
qualifying platform-managed TLS.
[Apple kqueue manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/kqueue.2.html) ·
[Apple networking API guide](https://developer.apple.com/documentation/technotes/tn3151-choosing-the-right-networking-api)

In the earlier arm64 Linux guest experiment, batched `io_uring` was
1.33–1.47× as fast as direct socket calls at depths 32 and 64 and slower at
depth one. The standalone Linux probe source was removed. Physical Linux
qualification remains pending. In the [macOS readiness probes](../runtime/socket/README.md)
on arm64 loopback TCP, ordinary `kevent` was 3.18× as fast as `poll` with one
ready out of 128 and 2.17× on bursts with eight active streams. `kevent64`
performed similarly.
The ordinary interface uses the smaller event record on the measured
ABI and is the lead candidate for the checked host integration.
Backend setup failures stay visible. Owned-stream, close, and stale-event
checks now cover successive checked owners on a persistent Darwin queue.
The earlier checked benchmark's warm tail result reversed on repeat, so poll
remains the default while kqueue stays available for explicit use.
Throughput through the bounded concurrent host was within 1% of
parity with eight active clients and with 24 additional idle clients.
The maintained tests check TCP owner lifetimes and routing across successive
and concurrent clients. [Hosted TCP tests](../tests/runtime/hosted_tcp.rs)

## Dependency, effect and performance boundaries

The approved prefix selects a name. Existing language requirements continue to
govern each operation, including [effect inference and explicit promises](spec.md#inference-and-explicit-promises),
[memory rules](spec.md#m1-memory-rules) and [resource lifetimes](spec.md#11-concurrency-and-asynchronous-resources).
A namespace establishes no `noalloc`, `noblock`, freestanding, zero-copy or speed
guarantee.

**Working library-design proposals:**

- Keep foundational byte and parsing facilities usable independently of OS
  services. A shared public prefix should permit small dependency closures;
  target availability and the selection mechanism still need specification.
- Keep protocol parsing and serialization usable independently of sockets, TLS
  and scheduling. Runtime resources belong in ordinary function arguments under
  the already preferred dependency policy.
- Support eligible input views and output construction into chosen destinations.
  State lifetime, validation, copying, capacity and failure obligations. Escaped
  text may require transformation even when an unescaped token can be a view.
- Preserve the eligible flat-storage, stable-growth, destination-construction
  and bulk-reset requirements through library interfaces. Review storage work
  against the full [Goose mechanism checklist](spec.md#goose-performance-compatibility-checklist);
  this namespace map supplies no new mechanism acceptance evidence.
- Keep broad Unicode tables, time-zone data and platform services behind explicit
  dependency boundaries. Their inclusion and runtime acquisition policies remain
  open.
- Design `using` directives as static name and dependency declarations, with
  resource acquisition and initialization explicit in ordinary operations. Initialization semantics
  still require approval before implementation; this proposal authorizes no
  implicit scheduler, threads or allocation from selecting a namespace.

Library APIs can offer convenient operations while retaining controlled paths
through the same checked implementations. Costs must be documented and measured;
an appealing namespace or wrapper supplies no performance evidence.

Before stabilizing an API, specify the valid inputs, units, representation
limits, and behavior on failure. Document partial progress, ownership and
cleanup, and any allocation, copying, or platform dependency. Use executable
cases that witness both valid results and failure at the contract boundary.

### API precedents for bytes and input

Go's `io.Reader` makes a caller-owned buffer and its short-read contract
explicit. It requires callers to consume a positive count even when the call
also reports an error. The hosted Holo adapter has a narrower native contract:
it publishes the positive count as data and reports interruption, would-block,
EOF, or another failure on its own outcome. Each outcome should document the
next legal action and the validity of the buffer. [Go `io.Reader`](https://pkg.go.dev/io#Reader)

Elm's byte decoders make width and byte order visible, and the decode boundary
can report malformed or incomplete input. Holo's byte helper operates
on two copied integers and names its low-octet conversion explicitly. The
worker owns framing, length limits, and incomplete-input handling. Future
fallible reads from a view should carry a checked result that distinguishes
missing bytes from a valid zero value. [Elm `Bytes.Decode`](https://raw.githubusercontent.com/elm/bytes/1.0.8/src/Bytes/Decode.elm) · [Go `encoding/binary`](https://pkg.go.dev/encoding/binary#ByteOrder)

TypeScript's explicit exports make module scope visible at the source boundary.
Its handbook also distinguishes export syntax, resolution, and emitted code.
Keep those concerns separate when growing Holo's local namespace experiment into
published package support. The local compiler resolves `using` directives before
the ordinary checker and lowers resolved calls directly. [TypeScript modules](https://www.typescriptlang.org/docs/handbook/2/modules.html#modules-in-typescript)

### Date and time design

**Working proposal.** Give elapsed-time measurement, a wall-clock instant,
a civil date and time, a zoned value, and a duration distinct semantics. An
elapsed-time source needs a monotonic clock contract and a stated lifetime.
A civil date-time needs an explicit zone before it can identify an instant.
Zone conversion must specify how callers handle repeated or skipped local
times. Document precision, range, overflow, calendars, zone-data provenance,
and serialization before stabilizing those APIs. Go's monotonic-clock contract
and Temporal's distinct instant, civil, zoned, and duration types give concrete
cases to compare. [Go monotonic clocks](https://pkg.go.dev/time#hdr-Monotonic_Clocks) · [Temporal type guide](https://tc39.es/proposal-temporal/docs/)

## Implementation boundary and remaining decisions

The experimental `holo.bytes` module searches, compares, and moves borrowed
byte views. `bytes.find` returns the first matching offset as `Option<u64>`.
The packaged `holo.bytes.read` namespace reads unsigned integers from those
views and returns `Short(required)` if the field is incomplete. Its cursor
helpers advance the caller's position only after a complete field. The
telemetry worker uses `read.take_u16_le` for length fields and saves the low
octet when `Short` reports a field split by a refill. The gzip inspector
assembles the four timestamp bytes across reads and calls `read.u32_le`.
Each parser owns its format validation.
The `holo.bytes.write` namespace provides a cursor that appends complete
octets, byte views, and unsigned ASCII decimal fields. A short destination
preserves its bytes and cursor position.
[Byte operations](../lib/holo/bytes.holo) ·
[Integer readers](../lib/holo/bytes/read/integer.holo) ·
[Binary cursor](../lib/holo/bytes/read/cursor.holo) ·
[Byte writer](../lib/holo/bytes/write/cursor.holo) ·
[Telemetry worker](../tests/fixtures/telemetry_worker/README.md) ·
[Gzip header inspector](../tests/fixtures/gzip_header/README.md)

Pass `--library-root lib/holo` to select the repository's
directory of `holo` leaf modules. The `using` directive supplies the `holo`
prefix and the leaf name, and the compiler reads `<leaf>.holo` directly from that root.
The compiler checks any `namespace` declaration against the requested path.
The entry file can call `public func` declarations through the leaf qualifier.
A plain `func` remains private to its library file. The root and its contents
are trusted, stable build inputs. The compiler resolves calls before checked
lowering, with no generated lookup, initialization, or allocation.

Manifest-driven package mode uses the multi-file visibility and resolution
rules above. It admits scalar results, copied scalars, borrowed byte arguments,
and the [fixed-record experiment](#fixed-record-experiment). `Batch<Record>`
parameters resolve their element record by the same namespace and visibility
rules as other signature types. A public function can expose a batch of a
public record. [Batch borrowing](spec.md#synchronous-borrowed-batches)
Package declarations live in `holo.toml`. Package loading and namespace binding
end before the existing checked pipeline and native backend.

The experimental `core.*` operations still belong to the
[frontend](../compiler/src/frontend/README.md) and lower through checked IR.
Stable APIs for text, numeric parsing, formatting, time, network services,
and other modules need their own semantics and evidence.

[Q08](spec.md#stage-a-a-coherent-core) remains open. It needs decisions on:

1. Nested namespace paths, `using` aliases, and selective namespace bindings.
2. [Public type experiments](#public-types-across-packages), opaque fields,
   and re-exports.
3. Distribution-wide ownership of the `holo` package and naming rules.
4. Published identities, registry selection, remote source integrity, and distribution.
5. Separate compilation and reusable compiled interfaces.
6. Initialization, target availability and dependency selection.
7. The exact library inventory, API semantics and implementation sequence.

The November 10 prefix decision selects `holo`; the separately approved
[naming convention](spec.md#identifier-naming-conventions) governs source identifier
style. The package and namespace rules listed above remain open.
