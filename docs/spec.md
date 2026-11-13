# Holo: a fresh systems language design

**Working specification 0.1 — November 8, 2026**

This is the starting specification for the new direction explored in the November
2026 language-design conversation. Holo is the working name. The existing
compiler and earlier design documents are experiments we can learn from; they do
not determine this design's syntax, semantics, runtime, or compiler architecture.

The goal is an exceptionally readable native systems language that makes peak
performance, deliberate memory design, and checked correctness accessible to
people and AI agents. Operating systems, databases, backends, infrastructure, and
AI workloads are all intended uses. Queues are useful building blocks rather
than the language's only concurrency or data-structure abstraction.

This is a living design document, not a finished standard. Its examples describe
the proposed language; they are not claims of support in the current compiler.
The research behind the design is collected in [research.md](research.md).
The [package and namespace design](packages.md) records the selected `holo`
standard-library prefix, the `namespace` and `using` source directives, and the
remaining package-system proposals.
The next executable milestone and comparison plan are in
[implementation-plan.md](implementation-plan.md).
The [compiler architecture](compiler.md) describes how checked language facts
should reach LLVM. Compiler source belongs in [compiler](../compiler/README.md),
Holo libraries in [lib](../lib/README.md), and application demos in
[examples](../examples/README.md). See the [evidence index](../evidence/README.md)
for preserved measurements and correctness records.

## Contents

1. [Goals and evidence](#1-goals-and-evidence)
2. [Decisions carried forward](#2-decisions-carried-forward)
3. [A small complete example](#3-a-small-complete-example)
4. [Functions and header grammar](#4-functions-and-header-grammar)
5. [Values, access, copying, and ownership](#5-values-access-copying-and-ownership)
6. [Checked execution guarantees](#6-checked-execution-guarantees)
7. [Types, expressions, and errors](#7-types-expressions-and-errors)
8. [Memory and resource lifetimes](#8-memory-and-resource-lifetimes)
9. [Layouts, encodings, and collections](#9-layouts-encodings-and-collections)
10. [Algorithms and execution schedules](#10-algorithms-and-execution-schedules)
11. [Concurrency and asynchronous resources](#11-concurrency-and-asynchronous-resources)
12. [Composition and extensibility](#12-composition-and-extensibility)
13. [Operating systems, native interfaces, and accelerators](#13-operating-systems-native-interfaces-and-accelerators)
14. [Human and AI tooling](#14-human-and-ai-tooling)
15. [Validation and performance evidence](#15-validation-and-performance-evidence)
16. [Design agenda and next decisions](#16-design-agenda-and-next-decisions)

## 1. Goals and evidence

### What success means

- **Readability:** ordinary code should reveal its inputs, results, access rights,
  ownership changes, memory work, and execution constraints in familiar words.
- **Peak performance:** enable implementations competitive with the best tuned
  native alternatives, with an ambition to lead on important systems workloads.
- **Memory efficiency:** make compact representations, locality, reuse, and
  predictable lifetimes practical without rebuilding their safety rules in every
  application.
- **Correctness:** enforce useful guarantees and make their boundaries inspectable.
  Distinguish what is statically checked, dynamically checked, tested, proved under
  a model, or merely measured.
- **AI comprehension:** provide consistent syntax, precise semantics, useful
  diagnostics, and structured tools for generation, repair, inspection, and tuning.
- **Systems reach:** allow freestanding code and direct hardware/OS interaction,
  while supporting productive application and infrastructure libraries.

“Fastest language” is a research ambition, not an achieved property or a universal
benchmark claim. Algorithm, representation, workload, compiler, target hardware,
and resource constraints determine results. A syntax change alone is not a speedup.

### Lessons to preserve from Goose

Goose separates native execution frames from compiler-managed data stacks and
uses representation and lifetime restrictions to reduce memory-management work.
Its documentation motivates flat data, relative links, destination construction,
and bulk reclamation. These mechanisms deserve explicit investigation here.
[Goose overview](https://aardappel.github.io/goose/)

Its queue restriction concerns communication between threads. Its data model also
supports arrays, trees, and reusable pools. Our design should retain efficient
queue paths while investigating additional sharing models.
[Goose reference](https://aardappel.github.io/goose/docs/reference.html)

Published Goose comparisons and the user's experiments are valuable evidence for
specific workloads. Before attributing a result to a language feature, record the
algorithm, representation, versions, hardware, and comparison conditions. This
draft does not claim to have reproduced those experiments.
[Goose benchmark design](https://aardappel.github.io/goose/bench/design.html)

## 2. Decisions carried forward

Status terms throughout this document:

| Status | Meaning |
|---|---|
| **Preferred** | A direction selected in the conversation; preserve it unless explicitly reopened. |
| **Design direction** | An intended capability whose detailed rules still need decisions. |
| **Working proposal** | A concrete rule to examine, not a completed or implemented guarantee. |
| **Open** | A decision is required before this part can be a complete specification. |
| **Research** | An approach worth investigating without committing to a language feature. |

| ID | Status | Direction |
|---|---|---|
| D01 | Preferred | One function declaration keyword: `func`. |
| D02 | Preferred | Braces delimit executable bodies. |
| D03 | Preferred | Function headers use explicit `arg` entries and a `returns` clause. |
| D04 | Preferred | A `with` clause belongs under `func`, above the arguments. |
| D05 | Preferred | Access and passing information follows the type: `arg name Type as ...`. |
| D06 | Preferred | Make the current examples explicit with `readonly copy` and `readwrite reference`. |
| D07 | Preferred | Keep runtime dependencies in ordinary function arguments; no baseline `uses` mechanism. |
| D08 | Preferred | Use `layout` for both built-in arrangement choices and user-defined layouts. |
| D09 | Preferred | Use `using` to attach a compatible encoding to a layout field/group. |
| D10 | Design direction | Keep algorithm meaning separate from checked execution schedules. |
| D11 | Design direction | Treat `noalloc` and `noblock` as checked, transitive contracts. |
| D12 | Design direction | Preserve ownership, compact data, and deliberate lifetime management as core tools. |
| D13 | Preferred requirement | Ordinary compiler-managed scoped data must support bulk reclamation whose work is independent of the number and total size of contained values. |
| D14 | Design direction | Prioritize destination construction, supported nonmoving growth, and flat nested storage as additional baseline guarantees; specify their eligible cases and restrictions. |
| D15 | Preferred (M1) | M1-R1: alias-permitting readonly/readwrite paths with left-to-right evaluation in the first single-threaded slice. |
| D16 | Preferred (M1) | M1-R2: private final-destination construction, publication on success, and record-suffix rollback on failure. |
| D17 | Preferred (M1) | M1-R3: bounded checked backing with recoverable construction errors in the initial slice; broader backing/exhaustion policies remain open. |
| D18 | Preferred | Infer provable allocation/blocking effects without required helper annotations; optional explicit `with` clauses preserve checked promises against future implementation changes. Detailed effect boundaries and general inference remain unfinished. |
| D19 | Preferred | Use `holo` as the public standard-library namespace prefix. The [package design](packages.md#selected-public-prefix) records this November 10 decision. Module APIs and broader package resolution remain open. |
| D20 | Preferred (M1) | Failed generated safety checks immediately transfer through a target trap; native helper contracts use an explicitly reviewed compiler/target/runtime profile. Host exception processing follows the defined boundary; arbitrary nonreturning calls are not exempt. |
| D21 | Preferred | Use `PascalCase` for named types and enum cases, `snake_case` for functions, methods, variables, parameters, fields and source namespace components, and `SCREAMING_SNAKE_CASE` for explicit named constants. Treat acronyms as words; naming checks warn by default with opt-in strict enforcement. See [identifier naming conventions](#identifier-naming-conventions) for scope, tooling policy and open details. |
| D22 | Preferred (experimental source libraries) | Spell a function visible to callers outside its namespace `public func`. The [local package rules](packages.md#declaring-a-namespace) define the supported function surface. Visibility for other declarations remains open. |
| D23 | Preferred | Use `namespace` as the source namespace declaration keyword. The [local library rules](packages.md#declaring-a-namespace) require a matching header for manifest-driven library files and admit an optional matching header in the legacy leaf profile. |
| D24 | Preferred (experimental local packages) | Allow a namespace to span source files. Plain functions are namespace-private, and `public func` is visible to files that select the namespace. Use `holo.toml` for package identity and dependency requirements and `holo.lock` for exact resolved dependency sources. See the [local package rules](packages.md#local-manifests-and-locked-builds). |
| D25 | Preferred (experimental source libraries) | Use `using <package>.<leaf>` to select a namespace in a source file. Its final path segment is the file-local qualifier. See the [namespace rules](packages.md#declaring-a-namespace). |

Earlier `view`/`mut` spellings, separate `kernel`/`routine` declarations, and implicit
`uses` examples are not the current baseline. Earlier `owns`, `take`, and `give`
ideas remain useful ownership proposals, but their complete integration is open.

These decisions select a readable direction. They do not settle every meaning of
copying, read-only access, references, errors, layout access, or scheduling.

### Identifier naming conventions

**Preferred — user-approved November 10, 2026.** Use the following
conventions for Holo source and public library APIs:

| Identifier kind | Convention | Examples |
|---|---|---|
| Source package/module namespace components | Lowercase; `snake_case` for multiple words | `holo.net.http`, `packet_tools` |
| Named types | `PascalCase` | `HttpRequest`, `RequestSummary` |
| Functions and methods | `snake_case` | `parse_request`, `read_content_length` |
| Variables and parameters | `snake_case` | `request_count`, `input_bytes` |
| Fields | `snake_case` | `content_length`, `request_headers` |
| Enum cases, error cases, and child-error aliases | `PascalCase` | `InvalidHeader`, `NotFound`, `Session` |
| Explicitly declared named constants | `SCREAMING_SNAKE_CASE` | `MAX_PACKET_SIZE`, `DEFAULT_TIMEOUT`, `PI` |

Single-word names such as `http`, `text` and `count` naturally remain lowercase
under the `snake_case` convention. Built-in type spellings such as `u64`,
`usize` and `bool` retain their documented forms.

These conventions apply across access levels. This decision itself gives no
declaration public or private visibility; export and visibility rules remain
open under [Q08](#stage-a-a-coherent-core). It does not decide identifier validity,
case sensitivity or declaration semantics. Naming methods, enum cases and
constants here does not establish support for those features or settle their
semantics.

#### Constants and immutable bindings

Use `SCREAMING_SNAKE_CASE` for explicitly declared named constants, regardless of
scope or visibility. Ordinary immutable bindings retain `snake_case`: for
example, `let packet_limit = 4096`. Compiler constant folding does not change a
binding's naming category. Enum cases retain `PascalCase`.

This selects constant naming, not a constant-declaration keyword, evaluation
rules or storage representation. Those semantics remain open.

#### Acronyms and digit-bearing terms

Treat an acronym or initialism as an ordinary word when applying the enclosing
identifier's case convention:

| Context | Examples |
|---|---|
| Named types | `HttpClient`, `JsonParser`, `Url`, `Uuid`, `IoError` |
| Functions and methods | `parse_http_header`, `decode_utf8` |
| Variables and fields | `request_id`, `source_url` |
| Constants | `MAX_HTTP_HEADERS`, `UTF8_BOM` |

Keep familiar letter-and-digit terms together: `Utf8Decoder`, `Ipv6Address`,
`utf8_decoder` and `ipv6_address`. Adjacent acronyms therefore produce names such
as `HttpJsonParser`. The same rule covers two-letter initialisms, as in
`IoError`. Built-in spellings such as `u64` remain unchanged.

These examples establish the acronym convention without requiring a complete
dictionary of acronyms or a general automatic word-splitting algorithm.

#### Naming diagnostics and renaming tools

Naming-policy violations in otherwise valid source produce warnings by default,
with suggested spellings where mechanically reliable. Projects may opt into
treating naming warnings as errors. Holo's workspace package suite requires
the maintained standard library to pass strict naming checks. Naming style itself
does not determine program validity or visibility; strict diagnostics can reject
a build under the project's selected policy.

Provide narrow, explicit exceptions for foreign bindings and generated code.
Respect required foreign symbol spellings and public API identities when applying
exceptions or refactorings. Automate mechanically checkable naming rules; broader
word-boundary judgments can use the documented examples and review without a
mandatory comprehensive acronym dictionary.

The formatter preserves identifiers. Identifier changes belong in explicit,
symbol-aware rename tools that update references and make public API changes
deliberate. Formatting must not silently rename a declaration or its uses.

**Experimental implementation:** compilation commands warn on declarations in
accepted source. `--strict-names` reports those same locations as errors and
fails the command before native code generation. For example, a declaration
named `requestCount` can suggest `request_count`. Diagnostics attach to the
declaration, including pattern bindings, and leave references and source text
intact. The checks cover currently implemented declaration forms. They leave
leading underscores and ambiguous acronym boundaries to review.

Standalone invocations apply the policy to the entry file. A package invocation
applies it to the selected package's checked sources. An application checks its
entry and reachable namespaces, while an entryless package check starts at all
its sources. Dependencies still undergo ordinary semantic checking. Run
`holo check --strict-names --package lib/holo/holo.toml` to apply
strict checks to the maintained standard-library package.

Exact lint names, configuration and suppression syntax, symbol-aware fixes and
foreign-binding syntax remain open.

**Open:** generic parameter names, special identifier prefixes, and casing for
other declaration kinds such as named layouts. Identifier validity, case
sensitivity and the detailed constant semantics also remain open as noted above.

Source namespace spelling also does not select registry/package identity syntax
or physical filenames. The [package design](packages.md) distinguishes those
boundaries and keeps broader namespace resolution open. This convention changes
no storage, effect or performance guarantee and supplies no runtime speed claim.

## 3. A small complete example

**Status: preferred header shape; working proposal for the supporting type, error,
layout, and schedule details.**

The example receives an existing batch, rejects invalid input before changing
anything, and clamps CPU readings. `server` and `time` remain grouped together;
`cpu` values form a separate column. The requested schedule targets that loop.

```holo
struct Sample {
    server u32
    time   u64
    cpu    f32
}

layout Telemetry for Sample {
    layout server, time as rows
    layout cpu as columns
}

type Samples = Array<Sample, Telemetry>

error NormalizeError {
    EmptyBatch
    InvalidCeiling
}

func normalize
    with noalloc, noblock
    arg samples Samples as readwrite reference
    arg ceiling f32 as readonly copy
    returns Result<usize, NormalizeError>
{
    if samples.empty {
        return error NormalizeError.EmptyBatch
    }

    if !is_finite(ceiling) || ceiling < 0 {
        return error NormalizeError.InvalidCeiling
    }

    for readwrite sample in samples {
        sample.cpu = clamp(sample.cpu, 0, ceiling)
    }

    return ok samples.len
}

schedule normalize.cpu {
    vectorize sample width native
}
```

Reading the header:

- `func normalize` names the function.
- `with` lists guarantees for the entire call.
- `samples` borrows the caller's collection; edits affect that collection.
  M1 permits compatible aliases; broader reference rules remain open. See
  [the M1 rules](#m1-memory-rules).
- `ceiling` is an immutable value copy.
- `returns` declares the function's result type. A body `return` supplies the result.
- The opening `{` ends the header and begins executable code.

The input already uses `Telemetry`; entering the function does not convert its
layout. The two error cases are intended to be inline result tags, with no
allocated error message or backtrace. The built-ins used here must satisfy the
declared contracts.

For this example, the intended `clamp` behavior preserves a NaN reading and
otherwise uses ordered comparisons to the bounds. Precise floating-point details,
including signed zero, signaling NaNs, exception flags, and target equivalence,
still need specification before a SIMD transformation can be approved.

`Array<T, Layout>`, `type`, the error declaration, `ok`, `return error`, and the
schedule selector are working notation. Their final grammar and representations
are tracked in the design agenda. `.cpu` names an implementation target; dispatch
and schedule selection are not yet specified.

## 4. Functions and header grammar

### Header shape

```holo
func function_name
    with guarantee, another_guarantee
    arg first Type as access passing_mode
    arg second OtherType as access passing_mode
    returns ResultType
{
    // Executable statements.
}
```

A small function using the same shape:

```holo
func add
    with noalloc, noblock
    arg left u64 as readonly copy
    arg right u64 as readonly copy
    returns u64
{
    return left + right
}
```

The arithmetic overflow rule for `+` remains open; this example selects the
header spelling, not an overflow policy.

Function calls use familiar argument lists in the current examples:

```holo
let total = add(20, 22)
```

### Working grammar fragment

The following describes only the header skeleton, not the full language grammar:

```ebnf
(* Current explicit copy/borrow subset; ownership transfer remains open. *)
function     = "func", identifier, header, body ;
header       = [ with_clause ], { argument }, returns_clause ;
with_clause  = "with", guarantee, { ",", guarantee } ;
argument     = "arg", identifier, type, "as", access, passing_mode ;
returns_clause = "returns", type ;
access       = "readonly" | "readwrite" ;
passing_mode = "copy" | "reference" ;
body         = "{", statements, "}" ;
```

This fragment intentionally leaves type grammar, tokenization, whitespace, clause
termination, empty returns, ownership transfer, and extensions unspecified.

**Working formatter rules:**

1. Put each header entry on its own line, indented once under `func`.
2. Put `with` before the arguments and `returns` after them.
3. Indent continuations inside a type one further level.
4. Put the opening body brace at the function's left edge.
5. Preserve long lines when presenting source on a phone; allow horizontal code
   scrolling rather than breaking identifiers or hiding indentation.

**Open grammar questions:** whether newlines terminate clauses, whether indentation
has any syntactic role, how defaults and anonymous functions end, and how a type
containing a function header is delimited. Prefer ordinary indentation for
formatting and an unambiguous explicit boundary at `{`, but prove the grammar can
handle the difficult cases before freezing it. Whitespace alignment must not
change ownership or access semantics.

Named arguments, default arguments, overload resolution, variadics, methods,
function values, callbacks, and asynchronous declarations are still open. Their
design must preserve the visibility of dependencies and effects.

## 5. Values, access, copying, and ownership

### Two independent decisions

Access answers **what may this function change?** Passing mode answers **does it
receive a value copy or borrow existing storage?**

| Declaration ending | Working meaning |
|---|---|
| `as readonly copy` | Receive a separate copy of the value; do not mutate it through this parameter. |
| `as readwrite copy` | Receive a separate copy of the value that may be changed locally. |
| `as readonly reference` | Borrow existing data for reading within its permitted lifetime. |
| `as readwrite reference` | Borrow existing data for reading and mutation; M1 does not require uniqueness. |

Access to objects reachable through a copied handle is governed by the read-only
and sharing rules below, which remain open.

For a scalar such as `f32`, `readonly copy` is straightforward: its local value
cannot be reassigned. A mutable copy could be changed without changing the
caller's scalar.

The semantics describe observable behavior. They do not demand a particular
calling convention, a physical copy instruction, a heap allocation, or one raw
pointer representation. The compiler may optimize representations while
preserving those semantics.

### Copy eligibility

**Working proposal:** `copy` is available only for types with a defined copy
operation that preserves the language's ownership rules. It is not an implicit
deep clone. Types owning unique resources cannot become copyable merely by
adding the word `copy` at a parameter.

Copying a handle copies the handle's value, not necessarily the object it names.
The specification must describe this explicitly for pointer-like, shared, and
device-backed values. A deliberate clone operation may allocate or run code and
must expose its effects.

Earlier unmarked scalar arguments were described as immutable value parameters.
That omission rule is now **open**. Canonical examples use explicit modes until
defaults are chosen. An omitted mode must never silently turn a resource into a
deep copy or an ownership transfer.

### References and read-only access

`reference` means a checked borrow; the caller retains ownership. **Earlier working
proposal, not a selected rule:** `readwrite` would require exclusive access to the
borrowed portion for the borrow's valid duration, with multiple compatible
read-only borrows permitted. The compatibility audit below identifies a conflict
with useful growth patterns. The [M1 rules](#m1-memory-rules) now approve
alias-permitting access for that first slice. The initial checked IR implements
that boundary; source syntax and broader reference-valued structures remain open.

Reborrowing, disjoint slices, fields, references stored in structs, escaping
returns, callbacks, globals, and asynchronous suspension all require lifetime and
alias rules. Exclusive access is a semantic guarantee to enforce, not an unchecked
annotation an optimizer simply assumes.

**Open:** distinguish binding reassignment, field mutation, and mutation of
reachable objects. A read-only handle must not be advertised as a deeply immutable
object without a rule that actually establishes that property. For layout-backed
collections, borrowing the logical collection need not mean a reference to one
contiguous native `Sample` object for every element.

### Owning and transferring resources

Earlier proposed vocabulary:

```holo
struct Upload {
    name Text
    body owns Bytes
}
```

- `owns` on a stored field expresses responsibility for the resource's lifetime.
- `give` at a call transfers ownership and invalidates the previous owning use.
- `take` was proposed for the receiving parameter.

**Open:** how these integrate with `arg ... as ...`, whether transfer is a third
passing mode, how access to a received owner is specified, and what failure does
to transferred inputs. Do not add a new final spelling by implication. Keep an
explicit distinction between owning an object, mutating it, and moving ownership.

## 6. Checked execution guarantees

### Inference and explicit promises

**User-approved policy, November 10, 2026.** Ordinary functions need no `with`
clause for effects the compiler can establish. Infer allocation/blocking effects
when provable, through supported calls; absence of a clause imposes no default
ban on allocation or blocking. In particular, cheap dynamic region construction
must not become unavailable because an omitted clause implicitly means `noalloc`.

An explicit `with noalloc, noblock` is an optional checked requirement. A constrained
caller may use unannotated helpers if their transitive effects satisfy it. A later
helper change that violates the written promise must reject the constrained caller
with an explanation of the offending call chain. The annotation cannot substitute
for proof, silently trust an unknown callee, or introduce a runtime fallback.

For example, a helper can remain uncluttered while its caller pins a requirement:

```holo
// Ordinary copied arithmetic; no dynamic storage is requested.
func add_words
    arg left u64 as readonly copy
    arg right u64 as readonly copy
    returns u64
{
    return core.add_wrapping(left, right)
}

// Preserve the transitive promise when this body or a helper changes.
func packet_checksum
    with noalloc, noblock
    arg header u64 as readonly copy
    arg payload u64 as readonly copy
    returns u64
{
    return add_words(header, payload)
}
```

The current adapter already admits this pattern through its closed pure call
graph. That does not establish a general effect-summary system or complete native
verification. Unknown external/callback/backend effects remain unestablished
unless verified or covered by an explicit trusted boundary. How inferred summaries
are exported, cached and represented in broader callable interfaces remains open;
an inferred implementation property is not automatically a permanent public API
promise. Explicit promises retain their force across implementation changes.

This policy changes annotation ergonomics, not the detailed working proposals
below. Strict `noalloc` still counts fresh bump-region storage. `noblock` does not
imply `noalloc`, and neither contract establishes uniqueness, SIMD independence or
a latency bound. Equivalent checked code can generate identical native code with
or without an explicit clause. No speedup is established by removing annotations.

```holo
func normalize
    with noalloc, noblock
    arg samples Samples as readwrite reference
    arg ceiling f32 as readonly copy
    returns Result<usize, NormalizeError>
{
    // Body and everything it invokes must satisfy these contracts.
}
```

### `noalloc`

**Working proposal:** no dynamic allocation may occur during the function's
execution, including its callees, callbacks, error handling, cleanup, and
compiler-generated work.

- Fixed compiler-managed local storage and operations on existing buffers are
  permitted under a specified frame-storage rule.
- Requesting fresh storage from a heap, growing an allocation, or advancing a
  dynamic arena to obtain a new object counts as allocation.
- Using an already reserved slot is permitted only under a defined existing-storage
  operation; “preallocated” cannot conceal arbitrary allocation effects.
- Hidden closure allocation, task frames, formatting, logging, and error objects
  must participate in checking.

**Open:** the exact boundary between fixed frame storage and dynamic storage,
growable compiler data-stack backing, recursion, pool reuse, virtual-memory commit,
page preparation, and allocator metadata. A language can avoid `malloc` while
still allocating dynamically; that is a different contract from this proposal.

### `noblock`

**Working proposal:** no operation may synchronously park the executing worker
while waiting for a lock, timer, I/O operation, or another task.

Asynchronous task suspension may be allowed because it releases the worker. Its
creation, suspension, scheduling, resumption, and cleanup must independently obey
any `noalloc` contract. A guarantee that a function never suspends would be a
separate concept; its spelling and need remain open.

`noblock` alone establishes no completion-time bound, termination guarantee,
lock-free or wait-free progress property, or immunity to OS preemption and page
faults. Define busy-waiting, scheduler interactions, and backend operations before
making stronger claims.

### Composition and checking

Contracts follow indirect calls as well as direct ones. Separately compiled APIs
and callback types must expose compatible guarantees. Unknown native operations
need an explicit trusted boundary rather than silently satisfying a promise.

A diagnostic should explain the chain that violates a guarantee:

```text
normalize requires noalloc
  -> record_metrics calls format_message
  -> format_message may allocate
```

The grammar may eventually allow compile-time arguments such as `align(64)` or
`inline(prefer)`. Keep distinct meanings for:

1. Checked requirements such as `noalloc`.
2. Compiler configuration or hints such as alignment and preferred inlining.
3. Runtime behavior such as tracing, retrying, or caching.

Runtime wrappers contribute their own effects and have observable order. A
reusable contract bundle may combine understood requirements; it cannot turn an
unchecked assertion into a proof. Bundle syntax, namespaces, conflicting clauses,
and parameterized guarantees are open.

### Fatal checks and the M1 native boundary

**Approved for M1, November 10, 2026.** A failed generated assertion or bounds
check transfers immediately through the target's trap operation. Holo emits no
formatting, libc abort, resource cleanup or callback before that transfer. The
guard must still precede the protected invalid access. Fatal checks do not become
recoverable append errors, and trapping execution promises no cleanup or recovery.
Ordinary success, recoverable statuses1/2/3 and lexical cleanup retain their rules.

Target trap signal and debugger behavior can differ from libc abort; the exact
host signal is not portable Holo syntax or a recovery API. OS exception/signal/
debugger processing after the trap transfers control is outside these execution
contracts. This boundary does not exempt arbitrary nonreturning calls, user
error handlers, logging or compiler-generated cleanup from effect checking.

Compiler-introduced memory helpers participate in the transitive contracts. M1
uses an explicitly reviewed compiler/target/runtime profile with documented
binding, interposition, hardware and valid-memory obligations. Unknown helpers,
unreviewed runtime identities and mismatched profile assumptions cannot silently
qualify. A reviewed finite implementation profile is a trusted boundary, not a
universal effect proof for every native build, platform, sanitizer or future
compiler. General effect exports and a universal post-link verifier remain open.
The [implementation plan](implementation-plan.md#concrete-decision-package-for-m1)
records the selected scope and acceptance status; no extra source annotation or
per-call runtime profile check is part of this rule.

## 7. Types, expressions, and errors

The language needs a small coherent core before advanced scheduling or hardware
policies can become usable. Most choices below remain open.

| Area | Requirement or question |
|---|---|
| Numeric types | Fixed-width integers and floats; define literals, conversion, overflow, shifts, division, and SIMD correspondence. |
| Floating point | Define NaNs, signed zero, rounding, fusion, exception behavior, reproducibility, and explicit relaxation permissions. |
| Structs | Named fields, initialization, alignment, representation, and evolution boundaries. The [public-type proposal](packages.md#public-types-across-packages) includes an executable experimental slice for fixed `u64` records and destination constructors across local packages. |
| Sum and optional types | Tagged alternatives and absence without magic sentinel rules; final vocabulary and layout rules open. |
| References and slices | Bounds, provenance, lifetimes, access rights, and escape rules. |
| Text and bytes | Encoding, indexing, ownership, allocation, borrowed views, and raw byte interfaces. |
| Generic code | Type/value parameters, constraints, specialization, separate compilation, code size, and diagnostics. |
| Interfaces | Static versus dynamic dispatch, effect signatures, object representation, and implementation coherence. |
| Local bindings | `let` and `var` are working notation; define binding mutability separately from object access. |
| Control flow | Conditions, loops, matching, early returns, loop labels, and cleanup on each exit. |
| Modules | Local packages have checked namespaces, file-scoped `using` directives, `public func` interfaces, and locked source dependencies. The [package design](packages.md) tracks the remaining namespace, visibility, type, and distribution decisions. |
| Compile-time code | Evaluation phases, resource limits, hygiene, determinism, and diagnostics. |

**Working error direction:** an ordinary typed `Result` can hold a success value
or an error tag without requiring a heap allocation. The full language must
specify payloads, representation, propagation, and cleanup. An error with owned
text or a backtrace has additional costs that the contracts must describe.
The [error-handling proposal](error-handling.md) specifies `return ok value`,
`return error Family.Case`, visible propagation with `try`, and exhaustive
`on … then` clauses. The experimental compiler implements copied `u64`
successes, [closed nominal error families](packages.md#typed-error-families)
with optional `u64` payloads, exact forwarding, scalar `let` propagation with
`try`, and explicit `on` handling. A family may include another through
`from ChildFamily as Alias`. `try` propagates the same family or a direct
child, and `on Alias.Case` selects an included leaf. The destination slice
implements `return ok Record:` and the same propagation rule in nested fields.
Typed `core.append` finalizes publication or rollback before its `on` arm runs.
Byte-bearing destinations reuse the checked byte-range initialization and
reservation rules. The experimental package binder supports
`Result<Record, E>` for destination constructors with fixed or byte-bearing
namespace-local schemas. An exported constructor uses a public record and
error family from its own namespace. A public error union exposes only
public child families. Inline fields of public schemas can
expose other public records in that namespace. A constructor whose body
is one direct `return` call can forward the exact record and result family
using its caller's existing destination. Independently stored aggregate
values remain follow-on work.
[Typed append](error-handling.md#typed-append-in-the-checked-compiler)

General patterns and owned or borrowed payload transfers need further rules.
Existing traps, capacity exhaustion, and I/O outcomes retain their own policies.

Safe code requires defined behavior for invalid accesses. The bounds/provenance
model and the division between static rejection, checked runtime failure, and
explicit unsafe operations must be specified. Optimizers may remove checks when
their conditions are proved; the toolchain should explain which checks remain.

## 8. Memory and resource lifetimes

### How closely this draft follows Goose

**Status: compatibility audit and working architecture proposal.** This draft is
Goose-inspired, not yet a strict adoption of its memory model. Listing a mechanism
as a goal does not establish the restrictions that make its guarantees possible.

| Area | Goose baseline | Current draft |
|---|---|---|
| Dynamic storage | Compiler-assigned data stacks with constrained top-of-stack growth | Checked IR has explicit bounded regions; automatic source-to-stack assignment is pending. |
| Growth | Supported growth preserves existing addresses | Approved M1 stable-extension rule, implemented with disclosed capacity guards. |
| Nested representation | Variable-sized parts can remain flat and contiguous | Checked IR/source share inline packed wide, header-derived u64 and explicit compact u32 modes; general programmable layouts remain future work. |
| Construction | Results are constructed at their destination | M1 constructor calls receive final destinations; general source returns remain open. |
| References | Root/lifetime checks deliberately allow aliases | M1 alias permissions approved; broader borrow/sharing rules remain open. |
| Threads | Separate mutable memory with typed queue communication | Additional sharing and transfer models are research proposals. |

Sources: [Goose overview](https://aardappel.github.io/goose/),
[Goose reference](https://aardappel.github.io/goose/docs/reference.html).

**Compatibility case to preserve and test:** hold a reference to an existing
element, grow its owning array, then use that reference. Goose permits this for
supported stable-growth arrays. The earlier blanket `readwrite reference` proposal
could reject the growth while that element reference exists.

M1 distinguishes access to existing elements from safe extension into a new suffix.
The checker/lowering must establish that extension preserves those elements'
addresses and permitted access. Broader Q02/Q09 questions remain open. This is not
a reason to emit an unchecked non-aliasing promise for the entire collection.

**Working architecture proposal:** establish a faithful Goose-style storage and
lifetime baseline first, then evaluate extensions with explicit representation,
permission, and cost boundaries. Regions, pools, shared data, and device-backed
storage need not all promise the same growth or reclamation behavior. This is an
implementation/type-system distinction to investigate, not a proposed new keyword.

### M1 memory rules

**Approved for M1 by the user on November 9, 2026.** This is the contract for the
[first checked region subset](implementation-plan.md#m1--implement-the-checked-region-subset),
not a complete language memory model. The implementation covers structured IR
and the [experimental source subset](#experimental-source-subset).
Its supported operations, limitations, and validation
are documented in [the source guide](../compiler/src/README.md). D13/D14 remain the full
performance goals; approving this slice does not establish performance parity.

The subset is single-threaded plain data: unsigned fields, inline nested records,
byte payloads, sequential traversal, ordinary calls, and one appendable batch per
backing region. Reference-valued record fields, user-defined reference returns,
shared mutation across threads, dynamic callbacks, external-resource ownership,
interior resizing, and general relocation/transfer are outside this first slice.
M1 calls are statically resolved and nonrecursive; recursive production/types wait
for their storage, initialization, and exhaustion rules.
Compiler-recognized element access may yield a checked reference; rejecting all
user-defined reference returns is not proof of a general reference-return checker.

The examples use **operation notation**, not new Holo keywords or finalized array,
constructor, error, or region APIs. Scope ownership is sufficient for this first
slice; no explicit public `region` or `reset` keyword is being selected here.

#### A. One construction tail, stable published contents

1. A backing region has a start, finite limit, and a marker for its next unused
   byte. These are conceptual storage facts, not a required runtime struct.
2. Its appendable batch owns the construction tail. A nested field or producer
   temporarily constructs into that same tail as part of the current record;
   it is not a second independently growing owner. Finish variable-sized parts
   in field order before constructing the following part.
3. Once a record is published, its location, inline part sizes, and field types
   remain fixed. Appending extends a new suffix without moving or rewriting the
   existing records. Fixed-width fields and already-existing payload bytes may
   still be changed through a permitted writable reference. Shape metadata such
   as lengths and offsets is compiler-managed; it cannot be overwritten as an
   ordinary writable field to change a payload's size, initialized extent, or
   interpretation. Future tag/layout-changing operations need separate rules.
4. M1 rejects growing/replacing a completed variable-sized field, shrinking or
   replacing the batch, and recursively appending to the same batch while one of
   its records is under construction. Independent growable owners need separately
   assigned backing; they cannot silently compete for one tail.
5. Records and eligible nested payloads stay inline, without per-child heap
   allocations. Variable-sized records do not automatically promise constant-time
   random indexing. Pin offsets, metadata widths, alignment, and padding in the
   layout fixture before comparison; an auxiliary index would have explicit costs.

This preserves the distinction between growing a container and resizing something
already inside it. Goose also restricts growth of an array while an element is
being constructed in place; our failure/publication rules below are a Holo
proposal, not an assertion of identical Goose error behavior.
[Goose construction rules](https://aardappel.github.io/goose/docs/reference.html#construction-in-progress)

#### B. Access permission, aliases, and roots

**Approved M1-R1:** permit compatible typed aliases in this
single-threaded subset. `readwrite reference` permits reading and writing through
that reference; it does not promise that no other reference exists. `readonly
reference` prevents writes through that reference and its derived paths into M1's
inline data. It does not freeze the underlying value against another writable
alias, take a snapshot, or establish thread-safe sharing.

```text
batch contains record 0 with id = 1
reader = readonly reference to record 0
writer = readwrite reference to record 0
write writer.id = 7
read reader.id -> 7
write reader.id = 8 -> reject
```

The caller retains ownership. Every reference has a checked root and projection
into initialized storage; its destination binding must not outlive that root.
Track these as compiler facts, without mandatory per-reference reference counts
or a runtime scan to find outstanding references. M1 may conservatively use
lexical scopes; it need not infer the last use of every alias. Rebinding an owner
or changing an object's size/type cannot invalidate a still-usable reference.
Reference-valued fields and their deeper access rules remain outside M1.

M1 uses left-to-right evaluation for statements, operands, call arguments, and
record fields. A value argument captures its value when evaluated; a reference
keeps its location rather than a snapshot of later contents. Resolve a construction
destination before running its producer. This sequencing is part of the approved
contract, not permission to reorder aliasing reads/writes during optimization.

An existing-element reference remains usable across a permitted append because
that operation leaves its initialized storage and type intact. Reject an escape,
an out-of-range projection, or later access to discarded storage. `readonly`,
`readwrite`, and stable addresses alone do not authorize LLVM `noalias`, invariant
loads, or loop-independence assumptions; establish those facts separately.

#### C. Destination construction and record publication

**Approved M1-R2:** construct an appended record privately in its
final slot, and publish it only after successful initialization. The compiler
passes the destination through nested producer calls; it must not require a
temporary aggregate followed by a copy into the batch. Branches may construct
fresh alternatives at that destination. Returning an arbitrary previously built
local or transferring a finished aggregate between regions remains unsupported
until separately specified.

The experimental compiler supports whole-result forwarding between two
destination constructors with the same nominal result. The wrapper consists
of a single direct return call. The callee receives the original private
destination and owns its initial reservation and initialization. The caller's
publication boundary keeps the original rollback behavior. This form uses the
existing evaluation and lifetime rules without materializing a local value.

An M1 record producer may compute with fixed local values, read initialized inputs,
and write its unpublished destination. It may call producers with the same checked
restrictions. It cannot mutate previously published records, publish a reference
into its unfinished output, reenter the owner's append operation, or perform
external effects. This is a checked subset restriction, not a new function keyword
or a blanket restriction on every ordinary Holo function.

Enforce those restrictions transitively through callees, aliases, and global
access. A `readonly` input alone does not prove that a helper cannot write the same
published data through another path. Unknown/indirect callee effects do not satisfy
the restriction by default; reject them in an M1 producer.

```text
begin append at current batch end
  save the entry marker and published count/end
  outer producer constructs the record in its receiving slot
    inner producer writes the nested byte payload in that slot
  initialize every readable field and payload byte
publish the completed record and advance the visible count/end
```

The physical slot is already the final slot while private; publication does not
move it. Fields are not readable before initialization, and unfinished bytes are
not visible as a batch element. Padding is not a readable source-language value;
raw-image/serialization validity needs separate rules. Copying existing input
bytes into an owned payload is real work to account for, even when no intermediate
record copy occurs. Explicit scalar `copy` semantics remain unchanged.

On a recoverable construction error, restore the saved marker and published
metadata. The previous batch's values, length, addresses, and existing references
remain unchanged from immediately before that append. Unpublished bytes may stay
physically present but become unavailable; no per-field zeroing, object walk, or
destructor is required for M1 plain data. Later publication must initialize its
own readable contents before exposing reused storage.

A failure from any nested producer aborts the owning append. M1 does not catch an
inner construction failure and resume writing that partially initialized
destination. Unwind the producer calls and discard their private state; the caller
may handle the append's error and begin a new attempt. Restore markers and
published metadata once at the append boundary. That rollback has fixed storage
bookkeeping, but propagating an error and leaving call frames may still require
work per frame; this is not a constant-time total failure-handling guarantee.

This is record-level publication and suffix rollback, not a transaction around
arbitrary functions. Writes performed by the caller before starting the append
are not undone. A producer that writes through an alias into older records is
rejected under this narrow proposal, rather than silently adding an undo log.

##### Fresh aggregate locals

**Implemented experimental M1 slice.** A scoped local constructs into one
explicitly selected bounded region in entry code. A typed destination
constructor supplies the record schema. The experimental `core.construct`
statement handles the outcome in lexical blocks.

```holo
let memory = core.region(512) else { return 0 }

core.construct(memory, decode_frame(input))
    on ok(frame) then {
        core.print(frame.sequence)
    }
    on Rejected Truncated(required) then {
        core.print(required)
    }
    on error then {
        core.print(0)
    }
```

- **Destination.** A caller-supplied live region lends its construction tail
  to one scoped local. The checker requires that this region has no competing
  live tail owner. The destination is resolved before arguments are evaluated
  once in written order. The producer writes directly into that slot using its
  existing fixed or variable reservations and checked layout. The region's
  finite backing supplies the bytes. There is no second record or backing
  acquisition per attempt.
- **Lifetime.** Success introduces `frame` as a readonly reference whose root
  is the scoped local. Projections and aliases share that lexical lifetime.
  The success name cannot be reassigned. A bare `on ok` or `on ok(_)`
  discards the name and holds the tail through the end of the arm.
  A copied scalar may leave the arm. A reference, a captured byte view, or the
  aggregate itself may not escape. Readonly limits this access path. It grants
  no optimizer alias promise.
- **Failure and release.** A producer's named rejection, `Full`, and
  `SizeOverflow` remain distinct outcomes. Failure restores the checkpoint
  and releases the attempted tail before the selected handler runs. Only the
  success arm can see the record. Cursor-based producers leave the marker
  unchanged on failure. A failure arm can explicitly retry in the
  same region. Success-arm exit restores the checkpoint exactly once, including
  early exit. It takes no walk through fields or payloads. Ordinary caller
  effects, including evaluation of actual arguments, are outside rollback.
- **Costs and propagation.** A fresh bump reservation counts toward `noalloc`
  even with pre-acquired backing. Ordinary storage checks and their existing
  failure order stay in the producer. A catch-all `on error` can cover remaining
  cases without a payload binding. A future `try` must represent both storage
  failures and the nominal error family, or require a checked proof that the
  storage failures cannot occur.

Plain `let frame = decode(...)` needs a rule that selects the backing and
specifies the result on failure. Choosing a surrounding data stack implicitly
or assigning fixed records to native frames would require a separate storage
and budget decision. The explicit-region slice allows both fixed and
byte-bearing records to use the same semantics. Moving a previously published
local to a different lifetime remains a separate ownership decision.

The acceptance cases exercise exact fit, one-byte shortage, size overflow,
nested rejection after a private write, and retry at the same marker. They check
both early exits, escaped aliases and byte views, competing ownership, and packed
byte ownership after input mutation across all three layouts. The fixed-record
LLVM witness checks the single destination and the absence of a batch or indexed
lookup. [Scoped result conformance](../evidence/scoped-result-20261111/README.md)

##### Compiler-selected backing for one scoped aggregate

**Implemented experimental slice.** Give one fresh typed aggregate construction site in
`main` an automatically selected backing. A test profile supplies one
compiler-owned finite data stack, its logical capacity, its lexical acquisition
boundary, and its acquisition failure handler. The stack holds payload bytes
separately from native call frames. The compiler selects this stack by its
checked identity. The same site can run repeatedly while the backing stays
live. Both the programmatic API and the
[experimental source form](#supplying-a-stack-budget-in-source) use the
same checked region and construction operations.

1. Acquire the bounded backing at the supplied boundary, which must enclose
   and dominate every visit to the site. An acquisition failure runs the
   supplied recovery without evaluating constructor arguments. Once acquired,
   the existing reservation checks determine whether a given record fits. A
   budget of zero still has zero usable bytes.
2. Save the free marker and resolve the final receiving address. Evaluate
   actuals once, left to right, and run the existing producer. Nested producers
   receive their final inline field destinations. Keep the producer's guard
   order, named error and payload, `Full`, and `SizeOverflow`. Any writes
   performed while evaluating actuals remain visible.
3. On success, lend one readonly reference through the selected lexical arm.
   All aliases and byte projections expire with that arm. Discarding the
   reference still holds the tail through the arm. Restore the saved marker
   when the arm finishes or exits normally through an early return. On a
   recoverable producer failure, restore the marker before the handler, or keep
   it untouched if the producer never advanced it. The handler receives the
   selected outcome and any bound scalar payload.
4. Release the backing when its owner scope ends. Each attempted construction
   uses the same backing while it remains live. Marker restoration makes the
   suffix available for a later visit. The target-trap policy continues to
   apply to failed safety checks.

The experiment requires one private root and one static site. A second
automatic site, a competing tail owner, or a reference that escapes the
success arm makes the case ineligible. Earlier live records and borrowed
inputs may use their own checked roots. The first slice assigns no stacks
across calls, recursion, suspension, or threads. General growth, virtual
reservation, native-frame promotion, aggregate transfer, and propagation of
storage failures through a bare `try` remain separate decisions.

The proposed lowering uses the same finite capacity and existing allocation
and failure semantics as its explicit-region control. Each attempt uses an
inline destination and fixed marker bookkeeping. Acquiring backing and
reserving fresh record storage both count toward `noalloc`. Logical release
and return of backing to the host occur at different boundaries.
[Compiler plan](compiler.md#placing-one-scoped-aggregate) ·
[Explicit-region control and acceptance plan](implementation-plan.md#automatic-placement-experiment)

##### Supplying a stack budget in source

**Implemented experimental source form.** Use
`with core.region(bytes) { … } else { … }` to give a block a bounded
backing. Within the success block, `core.construct(producer(...))` selects
that backing automatically. The source names the budget and the lifetime.
The compiler owns the region binding.

The snippet assumes an `open_ticket` producer that returns a ticket or rejects
an invalid ID. The [native placement tests](../tests/compiler/frontend/frontend_native.rs)
exercise the same source form against explicit region acquisition.

```holo
func main returns u64 {
    var opened = 0
    with core.region(32) {
        for id in core.range(3) {
            core.construct(open_ticket(id))
                on ok(ticket) then {
                    core.print(ticket.id)
                    opened = core.add_wrapping(opened, 1)
                }
                on Rejected InvalidId(invalid_id) then { core.print(invalid_id) }
                on Full then { core.print(1000) }
                on SizeOverflow then { core.print(1001) }
        }
    } else {
        core.print(1002)
    }
    core.print(opened)
    return 0
}
```

- **Budget and acquisition.** Check `bytes` as an ordinary `u64` expression in
  the enclosing scope. Evaluate it once at the `with` statement, then
  attempt acquisition once. The budget covers the record's representation
  in the selected layout, including its metadata. Zero grants zero usable
  bytes. The current M1 runtime requests one byte for a non-null base in
  that case. Setup takes place even when the success block would visit the
  site zero times.
- **Recovery.** Both bodies are required. Enter the success block only
  after acquiring backing. An acquisition failure runs the `else` block
  with the enclosing bindings and skips every success statement and
  constructor argument. Effects of evaluating the budget are already
  visible. Either body may fall through to the statement after `else` or
  return through normal entry cleanup. Their local bindings do not reach
  that continuation.
- **Construction.** The one-operand form takes a statically resolved typed
  destination constructor. It keeps the existing exhaustive block
  handlers, destination-before-arguments order, and lexical success
  reference. `Full` means a reservation exceeded the live backing's
  capacity. `SizeOverflow` means size arithmetic or layout width failed.
  Named rejections keep their family and payload. A final `on error`
  handles remaining construction outcomes. Acquisition failure reaches
  the enclosing `else` alone. The selected layout retains its existing
  reservation order. A retained header can hit `Full` before a later
  rejection guard. [Failure order](#borrow-sequencing-and-failure-rules)
- **Reuse and release.** The loop visits one static site. Each visit
  releases its private suffix on rejection or when the success arm ends.
  The next visit can reuse that address. Leaving the `with` block
  releases the backing through the existing region cleanup. Copied
  scalars can leave the success arm through ordinary assignment. Aliases
  and byte views retain the arm's lifetime.

The first source increment admits one `with core.region` statement
directly in `main` and exactly one one-operand construction in its acquired
body. The path to the site may pass through lexical scopes, counted and
batch iteration, explicit region success bodies, batch bodies, and
`refill` data bodies. Conditional arms, unbounded loops, recovery bodies,
and construction handlers are outside that path. Other statements and
explicit two-operand constructions retain their current rules. Imported
producers use the existing package visibility and result checks.

`with core.region(` is a contextual statement introducer. Function-header
`with` clauses retain their current grammar.
Acquisition and fresh bump reservations retain their `noalloc` effects.
[Source lowering](compiler.md#lowering-the-source-budget) ·
[Source acceptance cases](implementation-plan.md#source-budget-and-acquisition-recovery)

#### D. Scope release, finite backing, and failure

A scope may release its owned suffix only when no usable reference outside it can
subsequently access that suffix. Restore the fixed set of associated markers;
do not enumerate records or references. The backing owner stays usable for the
next scope. Ending an inner reference scope can establish that references are no
longer usable; merely choosing not to read an otherwise usable binding is not an
implicit reset permission in a conservative lexical checker.

Backing is finite. Check size/address arithmetic before forming an invalid access;
never expose uninitialized storage, wrap an offset, relocate published contents,
or spill to another allocator to make growth appear supported. Acquiring backing
and returning it to the OS remain separate from logical scope release.

**Approved M1-R3:** use a bounded, checked backing configuration
for the first executable harness, with small recoverable construction errors for
size overflow and capacity exhaustion. Error names and propagation syntax remain
open. A failed acquisition before a scope begins exposes no partial owner; a
failed append follows the publication rule in subsection C. Test faults must
exercise real failure boundaries rather than a separate implementation of
construction.

This configuration is an initial implementation restriction, not a requirement
that every target reserve/commit all memory upfront or check capacity on every
append forever. Capacity guards, commitment policy, and reserved/committed/resident
memory must be disclosed. Goose's documented growth path avoids ordinary capacity
checks; a checked bounded prototype cannot claim the same growth cost from address
stability alone. Evaluate proof-based check removal and other backing paths before
claiming parity. The full-language exhaustion policy remains a separate decision.
[Goose data stacks](https://aardappel.github.io/goose/docs/reference.html#data-stacks)

No construction/growth example automatically satisfies `noalloc`: obtaining fresh
dynamic region storage counts under the intended strict contract. Neither this
proposal nor a `noblock` label promises freedom from OS page faults or a latency
bound. Omit unestablished contracts from examples and test effect rejection.

#### Allowed and rejected cases

```text
allowed: stable reference during growth
  append record(id=1, bytes=[10, 20])
  first = readwrite reference to record 0
  append record(id=2, bytes=[30])
  write first.id = 9
  read record 0.id -> 9; first still names its original address

allowed: failure and reuse
  start with the two published records above
  begin a third record and fail during its payload construction
  observe the same two records, count, and existing-reference addresses
  end the scope containing element references
  end the batch scope; restore its markers
  construct another batch using the backing owner

rejected: escape
  create a batch inside an inner scope
  store its element reference in a binding that outlives that scope

rejected: invalid reset/access
  retain an element reference outside the scope being released
  release that scope and then read through the retained reference

rejected: interior resizing
  append record(id=1, bytes=[10, 20])
  grow that completed record's byte payload

rejected: construction interference
  begin constructing a record in the batch's tail
  call a helper that appends to that same batch, or writes an older record
```

These scenarios guide the implementation's acceptance checks. The initial
[checker tests](../tests/compiler/check/check.rs), [native tests](../tests/runtime/native.rs), and
[model/native comparisons](../tests/semantics/native_vs_reference.rs) exercise structured IR forms;
the [source checks](../tests/compiler/frontend/frontend.rs) and [source/native comparisons](../tests/compiler/frontend/frontend_native.rs)
now cover the experimental source subset as well. Current injected producer failures
occur at field/construction boundaries, not arbitrarily inside a byte initializer.
Require direct checks of addresses and initialized/publication state, independent
value oracles, compile-fail cases, injected failures, and generated release paths.
The [staged test harness](compiler.md#the-first-region-harness) must preserve full
inputs and toolchain identities. Constant marker work and absence of record
relocation/copies require their own code/memory evidence.

#### Approved choices and deferred alternatives

| Choice | Approved for M1 | Alternative and consequence |
|---|---|---|
| M1-R1: access and sequencing | Access permission without blanket uniqueness; readonly limits that path; explicit left-to-right evaluation. | Exclusive or frozen-data borrows require different conflict rules and may reject useful aliases. Finer-grained exclusivity remains a later option. |
| M1-R2: construction failure | Private final-destination construction, read-only inputs, publication on success, suffix rollback on error. | Allow producer writes elsewhere and preserve those effects on error, or introduce explicit transactional machinery; neither is implicit in this proposal. |
| M1-R3: first backing configuration | Checked bounded backing and recoverable construction errors for the initial slice, with costs measured openly. | A guarded/fatal-exhaustion path or stronger static budget proofs may reduce hot-path checking but have different failure/portability obligations. |

The user approved M1-R1 through M1-R3 as summarized above. Alternatives are not
implicitly selected, and source APIs/defaults outside this slice remain open.
Future changes that weaken the selected guarantees still require review.

### Experimental source subset

**Implementation status.** `frontend::parse_experimental` and the CLI expose source
parsing, lowering, checking, and native execution for this experimental subset.
The preferred vertical headers and brace bodies are preserved.
The temporary `core.*` calls let us exercise approved M1 storage rules while
permanent collection/error APIs, numeric defaults, and broader functions remain open.
See the [maintained source example](../tests/fixtures/region.holo) and
[frontend components](../compiler/src/frontend/README.md).

```holo
struct Entry {
    id u64
    data Bytes
}

func make_entry
    with noblock
    arg id u64 as readonly copy
    arg length u64 as readonly copy
    returns Entry
{
    return Entry {
        id: id,
        data: core.bytes(length, core.index())
    }
}

func main
    returns u64
{
    let memory = core.region(4096) else {
        core.print(1)
        return 0
    }
    {
        let entries = core.batch(memory, Entry)
        let status = core.append(entries, make_entry(7, 4))
        core.assert(status == 0)
        for readonly entry in entries {
            core.print(entry.id)
        }
        core.reset(entries)
    }
    core.assert(core.used(memory) == 0)
    return 0
}
```

Experimental grammar/semantics and implementation boundaries:

- Struct fields use `name Type`; literals use `name: value` in declaration order.
  Types are `u64`, `Bytes`, and acyclic record names. Decimal/hex literals are
  checked u64 values. Newlines/semicolons separate complete statements; lists may
  continue across lines. Indentation is formatting. Comments use `//` or nested
  `/* ... */`; identifiers are ASCII in this initial adapter.
- Copied scalar arguments require `u64 as readonly copy`. Record constructors
  also admit `Bytes as readonly reference` from complete live published byte fields
  or forwarded byte parameters. The [ordinary borrowed subset](#synchronous-borrowed-buffer-functions)
  supports scalar returns, readonly bytes, and writable fixed scalar record state.
  Pure scalar templates have
  one pure return, with statically resolved nonrecursive calls; they are boundedly
  inlined. Every entry call argument is captured once left to right, including
  unused parameters. Captures are scalar compiler IR bindings, not heap closures.
- Record-returning helpers have optional `core.reject_if(comparison)` prefixes and
  one fresh record literal. Nested named calls/literals construct directly into
  their receiving field. Aggregate locals, forwarding an existing aggregate,
  and arbitrary effects in producers are unsupported. `core.bytes(length, value)`
  initializes bytes using low-eight-bit narrowing; `core.index()` is scoped to
  its element expression. Completed record/byte shapes cannot be reassigned.
- Scalar helpers can declare `with noalloc, noblock`; all calls must resolve to
  the supported closed pure graph. Record constructors can declare `noblock`,
  but strict `noalloc` is rejected because they acquire dynamic destination
  storage. Unknown/duplicate contracts are rejected. Entry contracts and a
  general effect inference system remain unsupported; `noblock` supplies no
  page-fault, wait-freedom, or latency guarantee. The initial native audit exposed
  optimized memory helpers and a bounds-to-abort edge. The [accepted M1 follow-up](implementation-plan.md#m1-acceptance-decision--november-10)
  replaces abort with target traps and admits memory helpers under the named
  reviewed compiler/target/runtime profile. Labels still establish the restricted
  source rule; this is not universal linked-runtime certification or a complete
  effect-summary/export implementation.
- `let` disallows rebinding; `var` permits scalar/reference rebinding, separately
  from path access. `core.borrow_readonly/readwrite(batch, index)` and
  `core.alias_readonly/readwrite(reference)` make permissions explicit. The
  existing checker rejects readonly upgrades/writes, escape, changed-root rebinding,
  competing tail owners, and reset with usable references. Compatible aliases
  may observe one another's writes and survive supported append. No `noalias`
  promise is inferred. A bare reference-name initializer aliases with the same
  access; owners/aggregate data are never copied implicitly.
- A region/batch declaration owns the remainder of its lexical brace block.
  `core.region(capacity)` requires explicit acquisition recovery and exposes
  no owner on failure. Recovery may terminate entry or finish the owner's block,
  resuming outside it; it never enters the success-only tail. `core.batch(region, Record)` starts a tail scope;
  `core.append(batch, producer(...))` must bind its status: 0 success, 1 capacity,
  2 size overflow, 3 rejection. `core.reset(batch)` restores reusable marker/count
  state. These numeric statuses/API spellings are experimental, not final errors.
- `for index in core.range(count)` evaluates count once. Batch loops require
  `for readonly/readwrite entry in batch` and visit a captured published prefix;
  suffix appends do not extend that pass. `core.len` observes batch count or byte
  length. Scalar/byte fields support reads and authorized fixed-shape writes.
  Byte stores check the destination index before evaluating the replacement.
- Infix comparisons have unsigned semantics. Infix scalar arithmetic is rejected
  while defaults remain open; explicit `core.add_wrapping`, `sub_wrapping`,
  `mul_wrapping`, `and`, and `xor` name implemented policies. `core.assert` traps
  on false. In the entry body, `core.print` prints a decimal scalar or a
  double-quoted UTF-8 literal, followed by a newline. A literal occupies one
  source line and accepts `\"`, `\\`, `\n`, `\r`, and `\t` escapes.
  Embedded NUL bytes are rejected. Literal printing uses static bytes and host
  output. The literal syntax is limited to this operation.
  `core.used`, `address`, `now`,
  and `opaque` are temporary harness observations, not approved platform APIs.
- Main takes no inputs and declares `returns u64`. Only literal `return 0` is
  implemented, including early exits through branches/loops. Successful completion
  restores active markers and releases acquired backing in reverse lexical order.
  Other return values are rejected, never discarded. Runtime assertion/bounds
  traps terminate the process; they are not recoverable construction failures.
- Parser nesting is capped at 128 and lowering/definition nesting at 64.
  Definition and entry lowering each have a separate 100,000-unit expansion
  allowance. These limit charged expression/template/construction work, not all
  compiler allocations or accepted file bytes. ASTs, maps, operation spans, and
  scalar captures also consume compiler memory proportional to admitted input.

Check-only does not discover LLVM. Native builds retain all existing size/bounds
checks, packed wide metadata, explicit backing assignments, and same-root limits.
Neither this adapter nor its tests establish full-language support, universal compactness,
Goose performance parity, or a completed M1 comparison campaign.

### Acquisition recovery continuation

**Implemented experimental source behavior, November 9.** This removes an adapter
restriction using the existing approved M1-R3 failure boundary and checked Region
operation. It does not select general exception syntax or a new ownership model.

The handler is lowered before the owner's name is bound. It may read/write
permitted enclosing bindings, acquire an explicitly written replacement owner,
or use `return 0`. Normal handler completion skips the failed owner's entire
success tail and resumes after its containing brace block. The failed owner,
its batch, and its success-only locals are unavailable to both the handler and
the outer continuation. Handler-local owners/bindings also end before that
continuation. No implicit allocation, retry, fallback or aggregate copy is added.

Main still requires explicit `return 0` on every path. A return in a region's
success tail cannot establish termination of a nonterminating failure handler;
place a continuing acquisition inside an inner block and return afterward.
The [runnable example](../tests/fixtures/recovery.holo) shows that structure. Native
NULL injection and independent model expectations cover retry, outer aliases,
both outcomes of a conditional handler exit, and exact acquired-owner cleanup.

### Readonly byte-input construction proposal

**Implemented experimental extension, November 9, 2026.** The user authorized
implementation of this bounded path after reviewing its working proposal. It
applies approved M1-R1 access/sequencing and M1-R2 initialized-input/destination
rules; the surface API remains experimental. It does not select a general slice
API, reference-return model, external I/O API, or stable ABI. The complete
[maintained example](../tests/fixtures/byte_input.holo) now executes through the
same checked core; acceptance and remaining evidence are recorded in the
[implementation plan](implementation-plan.md#readonly-byte-input-construction--next-m1-extension).

The new experimental parameter form is:

```holo
arg input Bytes as readonly reference
```

Initially, only a record-returning destination producer may declare it. Its caller
supplies a complete `Bytes` field reached through a live record reference, such as
`source.payload.data`, or forwards an existing readonly byte parameter. Scalars
keep `u64 as readonly copy`; scalar-returning helpers remain scalar-only.
No additional keyword or automatic aggregate-copy operation is needed:

```holo
data: core.bytes(core.len(input), input[core.index()])
```

This explicitly initializes a new owned payload from the input. It performs real
source-byte reads and destination writes. Direct `data: input`, returning a borrowed
payload, and forwarding an already-built aggregate remain unsupported; the compiler
must not silently interpret those forms as a clone or ownership transfer.

#### Borrow, sequencing, and failure rules

1. A byte input names initialized published payload storage, its captured length,
   and its originating checked root/projection. Capturing an argument records its
   location/extent once in written left-to-right order, including unused arguments;
   it does not copy payload bytes or create a snapshot. Destination resolution
   still precedes producer argument evaluation. Native descriptor representation
   is an implementation choice, not a source-level address or an owned buffer.
2. The borrow is synchronous and cannot escape the producer call graph. Nested
   producers may forward it without extending its root's lifetime. Inputs cannot
   be stored in records, returned as references, rebound, reset, resized, converted
   to numeric addresses, or made to name an unfinished destination. Readonly input
   paths and their forwarded forms cannot write source bytes or metadata.
3. A caller-held writable alias is legal. Readonly limits this access path; it
   does not freeze other aliases. The checked producer graph itself has no
   capability to mutate published storage, invoke unknown effects, or reenter
   append. Single-threaded synchronous execution therefore preserves the input
   while the producer runs. Caller writes before or after the call remain visible
   through other aliases under M1-R1; no lock or reference count is introduced.
4. Inputs may come from a different region or the same batch's published prefix.
   Different regions have separate backing. For same-batch inputs, published
   payload extents are fixed and precede the new private suffix, so valid append
   leaves their storage intact. Root/projection and initialized-range
   checking must establish that fact; the spelling `readonly` does not prove it.
   A zero-length input can have a boundary address equal to the new suffix while
   containing no bytes. Do not infer blanket LLVM `noalias` from this permission,
   address stability, or that empty-range case.
5. `core.len(input)` reads the captured extent. `input[index]` evaluates its index
   once, checks it against that extent, then reads one byte as a `u64`. Existing
   wrapping/bitwise expressions and low-eight-bit destination initialization keep
   their semantics. No input bounds check may be omitted without a valid proof.
6. Field order remains unchanged. At a byte field, evaluate its destination length
   once, check representation width and size arithmetic, reserve the destination,
   then evaluate element expressions in index order. Capacity or size failure at
   that field must not evaluate its byte element expression. Earlier scalar/field
   reads and private writes may already have occurred. Length zero performs no
   element-expression evaluation or payload loads, even if its initializer would
   trap when evaluated. Preserve existing producer header/unit reservation before
   rejection prefixes, and scalar initializer evaluation before its scalar field's
   reservation; rejection does not universally take precedence over exhaustion.
7. A byte-read bounds violation remains a process-terminating bounds trap, not
   recoverable rejection. Capacity (1), size overflow (2), and explicit rejection
   (3) retain their experimental append statuses and suffix rollback. A nested
   producer failure aborts the owning append; recovery cannot expose an unfinished
   record or resume its construction. Recoverable failure leaves source bytes,
   published destination contents/count/end, and retained reference addresses
   intact. Trapping execution makes no recovery or cleanup promise.
8. The allowed producer graph is still closed, statically resolved, acyclic, and
   free of source-expressible external effects or writes outside its private output. The checker must
   validate typed arguments, read permissions, roots, input bounds operations, and
   these restrictions through every nested call, including unused definitions.
   `noblock` remains available under that graph check. Generated/native helper
   effects have [finite target/runtime acceptance](implementation-plan.md#m1-acceptance-decision--november-10):
   immediate target traps and explicitly trusted copy/fill/zeroing helpers under
   the reviewed profile. Source graph checking alone does not establish arbitrary
   native effects, and nonreturning calls are not silently exempt.
   Constructors still consume dynamic destination storage and cannot claim strict
   `noalloc`. Neither contract promises freedom from page faults or bounded execution time.

Arbitrary subrange views, standalone byte-view bindings, writable byte parameters,
byte parameters on scalar helpers, reference-valued fields/returns, foreign raw
pointers, asynchronous retention, and external input acquisition are outside this
increment. These are implementation-scope boundaries, not full-language bans.
They must reject clearly rather than fall back to a heap buffer or another runtime.

#### Synchronous borrowed-buffer functions

**Implemented experimental subset, November 10.** The user authorized the
synchronous borrowed-call implementation after reviewing this design. The
[reusable chunk-buffer decoder](../benchmarks/telemetry/chunked/README.md)
now calls an ordinary function. This slice extends ordinary calls beyond pure
scalar templates and keeps the separate destination-producer restrictions above.

The existing header modes express mutable fixed state and borrowed bytes.
The [runnable buffer example](../tests/fixtures/borrowed_buffer.holo) exercises the
same call behavior with two prefixes, an empty chunk, and an observing alias.

```holo
// Fixed caller-owned counters, initialized before the first call.
struct ByteTotals {
    // Number of bytes consumed across calls, modulo 2^64.
    seen u64
    // Sum of consumed bytes across calls, modulo 2^64.
    checksum u64
}

// Read the logical input extent and update the caller's existing counters.
// The call borrows storage synchronously and returns the consumed byte count.
func accumulate_chunk
    arg state ByteTotals as readwrite reference
    arg input Bytes as readonly reference
    returns u64
{
    // Each checked byte contributes once. An empty input executes no reads.
    for index in core.range(core.len(input)) {
        state.checksum = core.add_wrapping(state.checksum, input[index])
    }
    state.seen = core.add_wrapping(state.seen, core.len(input))
    return core.len(input)
}
```

`core.prefix(slot.wire, received)` is the experimental call argument form for
a logical prefix of initialized byte storage. It checks
`received <= core.len(slot.wire)`, then supplies a
borrow with exactly that extent. Reserved capacity and initialized bytes after
the prefix remain outside the callee's input. The empty prefix reads no bytes.
Creating the prefix evaluates its byte-field operand once to capture location,
initialized extent, and root, then evaluates its requested length once. It copies
no payload bytes. One possible call, with `totals` already a writable record borrow,
is the following.

```holo
let consumed = accumulate_chunk(totals, core.prefix(slot.wire, received))
core.assert(consumed == received)
// The synchronous callee's input borrow has ended at this point.
```

The tested `decode_chunk(state, input)` function uses a fixed `Decoder` record
containing its phase, saved header scalars, remaining body length, provisional
checksum, and running totals. Each call consumes the supplied logical extent.
An empty chunk preserves framing state. The caller observes end-of-input after
the last chunk and reports any incomplete frame separately.

The experimental subset has these rules and scope boundaries.

1. Admit fixed scalar-only record state by `readwrite reference` and initialized
   bytes by `readonly reference`. The state is already owned by the caller.
   Calls update that storage directly. Plain copied scalars keep their current
   capture rules. General aggregate locals and aggregate returns from this new
   mutable-call category, reference-containing state, recursive calls, callbacks,
   and foreign effects remain later design work. Existing destination-producing
   record returns retain their implemented rules.
2. Capture actual arguments once in written left-to-right order. A prefix extent
   failure traps before entering this callee and before its state writes. Nested
   helpers receive only the captured extent and checked originating root. A wider
   outer initialized field must not be substituted for a narrower forwarded prefix.
3. The callee's parameter borrows and local reborrows end when the synchronous
   call returns. A caller's existing reference can remain usable. An input cannot
   be returned, stored in the state, exported, or retained by a suspended task.
   The initial admitted state cannot contain reference fields. Caller reuse after
   return is checked against the caller's own remaining borrows.
4. Preserve M1-R1 alias-permitting access. Readonly limits one access path and
   does not freeze another alias. Mutations through explicit writable parameters
   are immediately observable through compatible aliases. Argument metadata is
   captured, while payload contents retain ordinary alias semantics. No `noalias`
   promise follows from the modes. The chunk example's separate roots can establish
   a narrower disjointness fact where the checker proves it.
5. Passing borrowed input and existing fixed state introduces no payload copy,
   region acquisition, reference count, lock, or hidden staging operation. Body
   and callee effects are accounted for separately. A pointer and captured logical
   extent are a candidate internal byte-argument representation. Stable ABI and
   register placement remain implementation decisions. Fixed function-frame
   storage and actual helper effects must be accounted for by contract checking.
6. Infer effects through the admitted closed synchronous helper graph. Explicit
   `with noalloc, noblock` remains optional and must reject any violating callee,
   generated operation, or failure path. Mutable ordinary functions are checked
   separately from the private-write-only destination producers. Their admission
   must not widen the producer graph's effects or change its failure ordering.
7. Calls add no transactional rollback. A decoder may update provisional parser
   state while waiting for more bytes, and publish a record tally only at a
   complete-frame boundary. General error returns and EOF result types need their
   own design decision. Existing bounds violations retain the accepted target-trap
   behavior and supply no recoverable-state or cleanup promise.

Acceptance cases for a first implementation must cover the following.

| Case | Required outcome |
|---|---|
| Two consecutive calls with the same initialized slot and different logical prefixes | Only each prefix contributes. The caller can overwrite after the previous call ends. |
| Zero-byte prefix with an initialized poisoned tail | Zero payload reads and no parser reset or EOF transition. |
| Dynamic prefix beyond initialized length | Bounds trap before entering the callee or writing its state. |
| Prefix forwarded through two ordinary helpers | Preserve the original logical bound and root, including a trapping out-of-prefix read. |
| Caller holds a compatible writable alias | Accept under M1-R1 and observe writes in source order. |
| Function tries to retain, return, or export the borrowed input | Reject at the offending escape. |
| Calling function ends, resets, or invalidates a root still borrowed by its callee | Reject the invalidation. |
| Stateful decoder split after each byte, rejected body drain, partial EOF, and immediate slot poisoning | Match the independent framing oracle and existing reusable fixtures. |
| Unknown callee reachable from a constrained function | Leave effects unestablished and reject the unsupported contract. |

Source functions enter this subset when a scalar-returning header declares a
supported reference parameter. Its checked bodies admit scalar locals and
returns, fixed state writes, branches, counted ranges, assertions, and direct
closed calls. Compiler-level declarations use a checked separate call graph.
Stable slice spelling, exported lifetime and effect summaries, ABI, asynchronous
retention, and reference-return syntax remain open for the full language.

#### Synchronous file and stream refill

**Working proposal, November 10.** Refill connects an already open regular file
or connected stream socket to the checked borrowed-buffer decoder. The first
slice uses an existing initialized `Bytes` slot and keeps its physical extent
fixed. Only a positive native result publishes a logical prefix.

The [hosted telemetry worker](../tests/fixtures/telemetry_worker/README.md) exercises
the active experimental source slice. A native host admits and owns one input
descriptor per process. Entry code uses `refill slot.wire, limit as chunk` with
an `else status` branch. Entry code may explicitly call `core.wait_input()` for
a would-block outcome by binding the result with `let`. The prefix can be
passed directly to a checked readonly `Bytes` parameter of an ordinary helper
or a destination producer in `core.append`. The view expires at the end of the
data branch. A producer can forward that same extent through readonly `Bytes`
formals. This source experiment accepts `loop {` on one line in the
entry function. The [earlier checked-IR experiment](../evidence/checked-refill-20261110/README.md)
preserves the prototype evidence. General Holo-owned handles and the public
outcome and branch syntax are open.

The proposed operation takes a source handle, a live writable projection of the
slot, and a positive request bound. The following is a sketch of the outcomes
and their scopes. The spelling of the operation, handles, and branch syntax is
open.

```text
match refill(reader, slot.wire, limit) {
    Data(chunk)      => decode_chunk(state, chunk)
    End              => finish_decoder(state)
    Interrupted      => handle_interruption()
    WouldBlock       => wait_for_input()
    Error(code)      => report_input_error(code)
}
```

The driver evaluates the handle, slot, and limit once in written order. It
requires `0 < limit <= initialized_extent` and checks representability under the
target's native request bound before entering the adapter. A dynamic violation
uses the existing bounds trap. The positive bound ensures a successful zero
result cannot come from a zero-byte request. Darwin's XNU implementation checks
file and socket counts against `INT_MAX`. A native acceptance profile must verify
the actual target's request and result conversions. [XNU file read](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/sys_generic.c#L283-L355) · [XNU socket receive](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/uipc_syscalls.c#L2101-L2124)

The adapter makes one synchronous native call per attempt, using `read` for the
file or `recv` with flags zero for the stream. Native signal restart behavior
may apply beneath that call. The adapter passes only the writable pointer and
bounded count, plus the handle. It captures the native error immediately when
the call reports failure, before performing any operation that can replace it.
It checks a successful count before constructing a byte view. An impossible
result outside the adapter's contract traps before any prefix is published.
[Apple read(2)](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/read.2.html) · [Apple recv(2)](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/recv.2.html)

| Native outcome | Driver outcome and decoder action |
|---|---|
| `0 < n <= limit` | `Data` binds exactly `[0, n)` in the slot. A short result is valid data and may split a frame anywhere. |
| Zero on a positive request | `End` exposes no bytes. It observes EOF at the current file position or the peer's orderly end of input after queued stream bytes. The caller finalizes this decoding session once. |
| Failure with `EINTR` | `Interrupted` exposes no bytes. The caller chooses whether to retry. A positive native result goes through `Data`, even if shorter than requested. |
| Failure with `EAGAIN` or `EWOULDBLOCK` on a supported nonblocking stream | `WouldBlock` exposes no bytes. The caller retains parser state and chooses when to try again. Admitted sockets have receive timeouts disabled, since Darwin can use `EAGAIN` for receive timeout too. |
| Any other reported failure | `Error(code)` exposes no bytes. Socket reset is an error. Parser totals for earlier complete frames stay available. |

The operation derives the borrow from its own positive native result. The
`Data` branch is the only scope that can receive that borrow. The prefix extent
is fixed for that scope, and forwarding it preserves the same checked root and
extent. A saved count from an earlier attempt cannot create a received view for
a later attempt, even if the counts match. Ordinary `core.prefix` can still view
initialized storage. The slot can be refilled again after the scope ends. The
checker rejects refill, reset, or owner release while a received view is usable,
including through another projection of the same storage. It also rejects
attempts to return or store the view. Ordinary initialized-byte aliases retain
M1-R1 access semantics. The `Data` binding establishes extent and leaves bytes
subject to writes through compatible aliases.

Each attempt supplies at most one new prefix. The caller controls retries,
waits, and input boundaries. Caller-owned parser state lives outside the refill
operation. Only `Data` supplies input to the decoder, and only `End` carries EOF
meaning. The native call determines the file position and slot contents after
each attempt. A file can grow after an EOF observation. Finalization after a
partial frame reports truncation. Resource closure belongs to the handle's
owner, separately from the region's marker reset.

The trusted adapter may write only within the validated extent, and a positive
count requires it to have produced this attempt's input bytes in exactly the
published prefix. The adapter retains neither the pointer nor the borrowed
handle. The handle, its admitted kind and mode, and disabled receive timeout
must stay valid throughout the call, including across any foreign aliases able
to close, reuse, or reconfigure the resource. A stream remains open on the local
receive side for the decoding session. Count checks establish bounds for the
view. The adapter and its native linkage must establish bounded writes, fresh
bytes, and handle validity.
Synchronous refill is an external effect in the driver. Its direct write into
existing backing requires no Holo payload staging, per-attempt region acquisition,
or tail clearing. Contracts such as `noalloc` and `noblock` require their own
admitted native effect boundary. Descriptor acquisition and closure APIs,
uninitialized backing, datagrams, and deferred completion have separate design
requirements.

| Acceptance case | Required witness |
|---|---|
| Regular file with a known short final read and an empty file | Exact byte count and frame totals, with EOF after the last positive result. |
| Local connected stream pair with fragmented frames and a half-close | Every queued byte is decoded before `End`. A partial final frame reports truncation. |
| Empty nonblocking stream, then data | `WouldBlock` supplies no input, followed by the exact `Data` prefix. Parser state survives between attempts. |
| Injected interruption, connection error, and invalid adapter counts | Error mapping and absence of a published view. Impossible native counts trap before decoder entry. |
| Zero, oversized, and unrepresentable request bounds | Trap before the adapter runs. |
| Poisoned tail, shorter next read, and immediate refill | Only the fresh prefix is visible. The same fixed backing is reused. |
| Attempt to save a received view, or refill/reset its live slot through the original or an aliasing projection | Reject the escape or invalidation at the source operation. |
| Refill reached from a checked constructor or `noblock` decoder | Reject the unsupported effect at its call boundary. |

#### Synchronous stream output and loopback sessions

The experimental hosted entry admits `let sent = core.send(record.bytes, offset, count)`
and `let ready = core.wait_output()`. Send captures the address and initialized
extent of a live `Bytes` field before evaluating offset and count once in
written order. A readonly reference suffices. It checks that offset is within
the extent and that count is positive, fits the remaining suffix, and fits the
native signed-int request cap. A dynamic range violation traps before forming
the subrange pointer. The adapter makes one native send from that range and
retains no pointer or descriptor.

The immutable send status reports the exact positive number accepted by the
socket, including a short send. The caller retains the source for any later
attempt and advances only by the reported progress. Interruption is
`u64::MAX`, would-block is `u64::MAX - 1`, and other errors are bit 63 OR-ed
with positive native errno. The adapter rejects a zero result on a positive
send. `core.wait_output()` performs an explicit writable wait that can block.
Its zero result permits another attempt, including after a hangup or an error
event. Either output operation reports tagged `ENOTSOCK` in file mode.

A session-aware entry uses `let next = core.next_session()`. When launched
with `listen-loopback PORT`, its native host owns one listener bound to IPv4
loopback and admits a connected nonblocking stream before entering Holo.
An explicit session transition closes the current client and attempts one
blocking accept. Zero selects the newly admitted client, interruption and
other errors leave no active client, and the Holo caller chooses whether to
retry. Another I/O attempt requires a live client. File and inherited-stream
invocations report 1 for the terminal transition and retain ordinary descriptor
cleanup on entry return. A transition is rejected while a refill view is live.
The host owns descriptor closure. Scoped Holo storage and application
ownership remain checked by the existing rules.

These operations are entry-only and add no public handle or effect contract.
The optional output and listener setup remain outside any send or refill's
byte-range checks. [Native adapter boundary](../runtime/refill/README.md) ·
[Hosted source witnesses](../tests/runtime/hosted_refill.rs)

The experimental [socket library](packages.md#socket-library) supplies
typed adapters for these status domains. The
[owned TCP profile](packages.md#owned-tcp-capabilities) implements a
host handoff and checked lexical close. Future in-flight byte loans and
multi-stream ownership have separate qualification gates.

#### Illustrative nested input construction

This complete experimental source passes a borrowed payload through an ordinary nested
producer call. The seed creates initialized source bytes for the example; it is
not an implementation of file/network ingress or evidence of external-input support.

```holo
// Payload bytes remain inline in their owning record.
struct Payload {
    // Frozen length and initialized data; borrowing does not transfer ownership.
    data Bytes
}

// A record owns its scalar and nested variable-sized payload.
struct Entry {
    // Application identity, separate from representation metadata.
    id u64
    // Destination-constructed inline child, with no per-child heap allocation.
    payload Payload
}

// Produce length bytes from id+index using copied scalars; append may fail to fit.
func seed
    with noblock
    arg id u64 as readonly copy
    arg length u64 as readonly copy
    returns Entry
{
    return Entry {
        id: id
        payload: Payload {
            data: core.bytes(length, core.add_wrapping(id, core.index()))
        }
    }
}

// Read input for this call and initialize a fresh owned Payload in its final slot.
func copy_payload
    with noblock
    arg input Bytes as readonly reference
    returns Payload
{
    return Payload {
        data: core.bytes(core.len(input), input[core.index()])
    }
}

// Copy the scalar id; forward the byte borrow without an intermediate record.
func from_bytes
    with noblock
    arg id u64 as readonly copy
    arg input Bytes as readonly reference
    returns Entry
{
    return Entry {
        id: id
        payload: copy_payload(input)
    }
}

// Bounded example: observe success, rollback, source preservation, and same-root append.
func main
    returns u64
{
    // Backing acquisition failure ends this demonstration before its expected output.
    let source_memory = core.region(256) else { return 0 }
    let sources = core.batch(source_memory, Entry)
    let seeded = core.append(sources, seed(10, 64))
    core.assert(seeded == 0)
    let source = core.borrow_readonly(sources, 0)
    {
        let output_memory = core.region(128) else { return 0 }
        let entries = core.batch(output_memory, Entry)
        let first = core.append(entries, from_bytes(7, source.payload.data))
        core.print(first)
        // One copy fits every layout; two exceed this region's finite capacity.
        let failed = core.append(entries, from_bytes(9, source.payload.data))
        core.print(failed)
        core.print(core.len(entries))
        let output = core.borrow_readonly(entries, 0)
        var sum = output.id
        // Validate every owned output byte, independently of metadata widths.
        for index in core.range(core.len(output.payload.data)) {
            sum = core.add_wrapping(sum, output.payload.data[index])
        }
        core.print(sum)
        core.print(source.payload.data[63])
    }
    // A published prefix can supply input while this same batch grows a new suffix.
    let appended = core.append(sources, from_bytes(9, source.payload.data))
    core.print(appended)
    core.print(core.len(sources))
    return 0
}
```

With successful backing acquisition, hand-derived expected output is
`0, 1, 1, 2663, 73, 0, 2`, one value per line. The 64 input bytes are 10 through
73; their sum is 2656, plus output id 7. The nested record occupies 96 bytes in
Wide, 80 in Derived64, and 76 in Compact32. One fits output capacity 128; two do
not. Two fit source capacity 256 in every mode. The [source/native checks](../tests/compiler/storage/byte_input_native.rs)
confirm the expected values against an independent model under all layouts at
O0/O3/ASan. They do not establish dynamic copy volume or physical memory traffic.

Scalar-helper actuals in byte-bearing producers use an internal eager scalar frame
before evaluating the helper body. This preserves bounds failures even when the
helper ignores its parameter, and avoids duplicating the captured read when it
uses a parameter more than once. There is no new source keyword, runtime closure,
indirect call, or payload buffer. Pure scalar-only template substitution remains
valid for the previous nontrapping subset. Optimizations may remove work only when
they preserve the specified values, bounds failures, and observable sequencing.

### Experimental physical layouts

**Implementation and evaluation authorized by the user on November 9, 2026.**
These configurations share the checked core and do not select permanent source
layout syntax or a stable ABI. [Schema planning](../compiler/src/layout.rs) runs after
logical checking; all modes use the same access, lifetime, construction, and
publication rules and one parameterized LLVM backend.

| CLI selection | Record-size policy | Byte length | Benchmark fixed portion |
|---|---|---|---:|
| `--layout wide` | Preserve each eight-byte total-size header. | u64 | 32 bytes |
| `--layout derived` (source default) | Elide eligible headers using checked schema shape. | u64 | 16 bytes |
| `--layout compact` | Same eligibility; explicitly bounded narrow lengths. | u32 | 12 bytes |

The fixed portions above apply to `Entry { id u64, payload Payload }` with
`Payload { data Bytes }`. The application id remains eight bytes in every mode.
The original 32-byte format includes two eight-byte total sizes and an eight-byte
length; derived removes the two sizes; compact also narrows the length to four
bytes. Other schemas have their own explicit plans.

The initial planner elides a record header when all its fields have fixed sizes,
or when exactly one variable-sized field is last. A nested final field may retain
its own header; the parent can derive its total from that child's stored size.
Records with several variable fields or a variable non-tail field keep a total-size
header, avoiding a new general metadata-walk cost. This is a conservative policy,
not a claim that every other header is mathematically necessary. Derived stepping
reads frozen length/size metadata and fixed schema facts, never payload bytes.
Traversal remains linear in records for a fixed schema; indexed borrowing still
walks preceding records. Representation helpers introduce no runtime callbacks.

An otherwise empty headerless record occupies one initialized unit byte, so
batch cursor/index progress stays positive. Fixed nested fields include that unit
in their offsets. All accesses remain packed/alignment 1. Lengths, sizes, and the
unit byte are compiler-managed representation, not writable source fields.
Region markers/offsets remain u64 in every mode; compact changes byte-length
metadata, not application `u64` scalars or whole-region address width.

Compact admits at most `2^32-1` bytes in one payload. An over-width length returns
construction overflow status 2 at that field's normal evaluation point, before
truncation or payload reservation. Earlier field work may have occurred privately;
the owning append rolls back its suffix once and preserves published data,
addresses/count/end, and usable aliases. `--layout derived` is the explicit
full-width alternative. There is no hidden widening, relocation, reallocation, or
dynamic format switch. Width eligibility does not promise that a payload fits its
finite backing. Static derived extents exceeding u64 are rejected before native
emission; the preserved wide control remains available.

Removing a representation header naturally changes the bytes that must fit
before subsequent construction operations. Tight-capacity failure outcomes can
therefore differ by explicit physical format. Field/argument order, private
construction, narrow-overflow handling, and suffix rollback remain specified;
bit-identical status outcomes across differently sized formats are not promised.

`holo inspect-layout --layout compact source.holo` requires no LLVM
and reports size widths, fixed/minimum extents, symbolic offsets, and each retained
header's reason. After conformance and the corrected matched campaign, full-width
`derived` is the experimental source CLI default. `--layout wide` and the existing
library `llvm::emit` preserve the original control; library callers choose other
formats explicitly. Compact stays opt-in; automatic narrow-width inference is not implemented.
Successful conformance or a smaller image alone does not establish a speedup,
Goose-equivalent backing, or a completed M1 mechanism checklist.

### Performance additions and their compatibility

- Checked tiling, fusion, specialization, and interleaved independent traversals
  can be investigated without first introducing a general heap or shared mutation.
- Aligned, blocked, and columnar representations may improve selected workloads,
  but can change packing, element-reference behavior, or the single-flat-message
  property. Preserve those guarantees only where the chosen representation permits.
- Shared mutation, cross-thread ownership transfer, and independently freed
  objects require additional lifetime/synchronization/reclamation rules. Their
  costs must not silently become requirements of the baseline.
- Goose already supplies SIMD multiversioning and checked bounds-check elimination.
  The additional research opportunity is richer composable scheduling and
  representation control, not the mere existence of SIMD or check removal.
  [Goose function qualifiers](https://aardappel.github.io/goose/docs/reference.html#function-qualifiers)

### Mechanisms to support or evaluate

| Mechanism | Intended use | Required design work |
|---|---|---|
| Fixed local values | Cheap scalar and small aggregate work | Frame size, recursion, escape, and target ABI. |
| Scoped regions/data stacks | Nested lifetimes and required object-count-independent bulk memory reuse | Growth, reset, partial initialization, escape, and exhaustion. |
| Destination construction | Build large or variable-sized values in their eventual home | Branching returns, insertion, failure, cleanup, and guaranteed copy avoidance. |
| Relative links | Compact structures and relocatable images | Base identity, offset width, overflow, bounds, alignment, and validation. |
| Pools and stable handles | Graphs, indexes, caches, and nonnested lifetimes | Slot reuse, stale identity, generations, deletion, and permissions. |
| General owned storage | Independent and long-lived resources | Allocator interfaces, fallibility, fragmentation, and cleanup. |
| Shared storage | Read-mostly data and selected concurrent structures | Permission model, reclamation, synchronization, and memory ordering. |

These are design objectives, not a requirement that every object use every
mechanism. Neither mandatory per-object heap allocation nor mandatory tracing GC
should be necessary for core native programs. Optional libraries must make their
own allocation and reclamation costs visible.

### Required baseline: bulk memory reclamation

**Status: preferred performance requirement; implementation and proof rules remain
open.** Preserve Goose-style reclamation for ordinary compiler-managed scoped
data. At the end of an eligible scope, the memory becomes reusable by resetting
the associated region/data-stack markers.

For a fixed program and storage shape, reclamation work must be bounded by the
fixed set of markers being reset, independent of the number of stored objects,
their nesting depth, or their total payload size. It must not traverse those
objects, issue per-object frees, run a garbage collector, or adjust per-object
reference counts. Creating a separate region per object would not satisfy this
requirement merely because each individual reset is cheap.

Conceptually, one region's logical release can be:

```text
region.top = saved_scope_top
```

This is a target semantic/cost guarantee, not a statement that the new compiler
already emits that instruction or has matched Goose's measured teardown time.
Multiple statically selected backing regions may require several marker resets;
the requirement is constant work per backing region and object-count-independent
work for the eligible scope.

Keep the following operations distinct:

| Operation | Required treatment |
|---|---|
| Logical region reset | Makes existing backing storage reusable with the bounded work above. |
| Returning backing memory to the OS | Separate decommit/unmap policy, potentially dependent on memory size and the OS. |
| Releasing external resources | Explicit lifecycle work for files, requests, registrations, and other resources; its cost is not hidden inside the bulk-reclamation claim. |

The checker must establish that no usable reference, captured callback, or
asynchronous operation can still access a region when it is reset. Values needing
additional resource cleanup require a distinct, specified lifecycle; their work
must not silently become mandatory for plain nested data. No implicit copying to
another allocator or runtime sweep may be used to make an invalid lifetime appear
compatible with this baseline.

Validation must inspect generated release paths and compare increasing object
counts and nesting at a fixed storage shape. Verify the absence of object walks,
per-object frees, and reference-count updates. Benchmark release time separately
from construction, external-resource cleanup, and OS backing-memory release, and
report retained resident/committed memory as well as timing.

### Lifetime and representation obligations

1. A reference cannot remain usable after the storage or object it names becomes
   invalid. Logical identity under slot reuse needs its own rule.
2. In-place growth needs sufficient reserved storage and compatible live objects;
   address stability is not a promise that every allocation can grow forever.
3. Region reset must account for outstanding references and asynchronous users.
4. Reclaiming memory and releasing external resources are separate obligations.
   Files, locks, callbacks, and requests may require work even when memory can be
   reclaimed in bulk.
5. Partially initialized values must not be read or leaked. Failure must clean up
   exactly the initialized resources under a specified order.
6. A relocatable image needs bounds/alignment/type validation and versioned format
   rules. Structural validity alone does not establish durability or trust.

Virtual-memory reservation is one implementation technique, not a portable
semantic requirement. Kernels, embedded targets, bounded environments, and devices
need alternatives. Inspect reserved address space, committed memory, resident
memory, fragmentation, and actual bytes moved separately.

**Open:** region and allocator vocabulary, destination-construction guarantees,
growth restrictions, stable versus movable values, cleanup ordering, graph access,
sharing, and stale-handle detection. These are central specification work, not
details to postpone until after syntax is implemented.

## 9. Layouts, encodings, and collections

### Separate logical values from their representation

```holo
layout Telemetry for Sample {
    layout server, time as rows
    layout cpu as columns
}
```

`rows` keeps selected fields of a record together. `columns` keeps values of the
same field together across records. A blocked column arrangement is another
candidate:

```holo
layout BlockedTelemetry for Sample {
    layout server, time, cpu as columns(block: 128)
}
```

Rows and columns are a starting vocabulary, not an exhaustive list. The design
should accommodate tiled arrays, hot/cold field splits, relative-link structures,
inline variable-sized records, paged/block-based arrangements, and isolated
alignment-sensitive counters where appropriate. Their names and contracts remain
open. LLAMA and Kokkos provide useful precedents for programmable mappings.
[LLAMA mappings](https://llama-doc.readthedocs.io/en/latest/pages/mappings.html),
[Kokkos views](https://kokkos.org/kokkos-core-wiki/ProgrammingGuide/View.html)

### Encodings use `using`

```holo
layout Archive for Sample {
    layout server as columns using Dictionary
    layout time   as columns using Delta
    layout cpu    as columns
}
```

An encoding implements a representation of logical values. Dictionary codes,
compact deltas, bit packing, and runs are candidates with different access and
mutation costs. Delta encoding must preserve input order and represent the
allowed differences; choosing it does not authorize sorting records. Smaller
representations are not always faster for every operation.
[Parquet encodings](https://parquet.apache.org/docs/file-format/data-pages/encodings/)

Precision-changing encodings require explicit permission. `Quantize(step: 0.1)`
is illustrative notation for a lossy policy, not an interchangeable implementation
of exact `f32` storage.

### Layouts expose capabilities and costs

The collection implements operations such as append, lookup, update, and delete.
The codec implements encoding/decoding. The layout connects fields to physical
arrangements and compatible access implementations. Selecting a layout does not
invent a collection algorithm.

An encoded field may yield a decoded value or an accessor rather than a native
reference. Mutating it may rewrite a block or require new capacity. Such an
operation must expose its effects and failure behavior; it cannot automatically
satisfy `readwrite reference` at the field level or `noalloc` at the operation level.
Arrow is useful evidence for the access/mutation tradeoffs of columnar formats.
[Arrow format](https://arrow.apache.org/docs/format/Columnar.html)

**Working proposal:** provide a small trusted mapping/access foundation and allow
user-defined layouts to compose or implement it. Preserve enough compile-time
information to specialize common mappings. Arbitrary user code is not automatically
safe, transparent to optimization, or free of runtime dispatch.

Changing an existing collection's layout is an explicit, potentially fallible
conversion. Earlier notation was:

```holo
let archive = try samples.rebuild(layout: Archive)
```

`rebuild`, `try`, preservation versus consumption of the source, allocation,
rollback, and the selected layout's place in the type system are all open. No
example should make a costly conversion look like a free annotation change.

## 10. Algorithms and execution schedules

The function describes the operation. A schedule describes a checked execution
strategy for an implementation of it.

```holo
schedule normalize.cpu {
    vectorize sample width native
}
```

Future schedules may express tiling, fusion, batching, unrolling, parallel work,
and target-specific operations. Each transformation has preconditions. A compiler
must establish them, require an explicit supported proof/permission, or reject the
request with a useful explanation. It must not silently assume independence.

Schedules preserve defined numeric behavior, memory effects, bounds, error order,
and observable behavior unless the source grants a specific relaxation. Neither
`noalloc` nor `noblock` proves loop iterations independent.

The language needs stable ways to identify loops and regions. The tour's unique
`sample` loop binding is illustrative; repeated names, nesting, code edits, and
composed transformations require a stronger reference model.

User-extensible scheduling libraries are a research priority. Exo 2 and MLIR
Transform suggest keeping transformation programs separate from the operation
being transformed. Their use does not make arbitrary transformations correct.
[Exo 2](https://arxiv.org/abs/2411.07211),
[MLIR Transform](https://mlir.llvm.org/docs/Dialects/Transform/)

Keep compile-time computation schedules distinct from runtime placement policies
for workers, cores, queues, and memory. The final terminology should make that
difference obvious. Explicit schedules are requests with checked legality, not
proofs of faster execution.

## 11. Concurrency and asynchronous resources

### More than one useful execution model

Preserve efficient single-owner work and typed queue communication. Also investigate
shared immutable data, ownership transfer between tasks, and carefully controlled
shared mutable structures. Specify synchronization and reclamation before claiming
any model safe. A systems language also needs atomics and an explicit memory model.

Thread-per-core, work stealing, polling, interrupts, and dedicated I/O workers
should be evaluable policies rather than an accidental consequence of syntax.
Worker, queue, and memory placement can be coordinated for locality; ownership
requirements and device capabilities still need checking.
[DPDK PMD guide](https://doc.dpdk.org/guides/prog_guide/ethdev/ethdev.html)

Queue APIs must state capacity, full/empty behavior, batch behavior, ordering,
ownership on failure, and concurrency permissions. “Queue” alone says none of
these. Avoid exposing a blocking operation as a harmless-looking bounded enqueue.

### Completion-based resource lifetimes

**Required design invariant:** an asynchronous operation retains valid storage
and permissions for as long as its backend may access them. Submission returning
does not establish completion. Buffer lifetime, address stability, registration,
alignment, device visibility, and backend restrictions are separate requirements.
[SPDK NVMe submission](https://spdk.io/doc/nvme_spec.html)

A useful conceptual protocol is:

```text
Available -> InFlight -> Completed -> Available
```

Requesting cancellation does not itself move the original operation's resources
back to `Available`. Races, separate completions, and multishot operations need
backend-defined terminal events before reuse or reclamation.
[liburing cancellation manual](https://raw.githubusercontent.com/axboe/liburing/master/man/io_uring_cancelation.7)

This invariant may be implemented with ownership transfer or a statically valid
borrow; neither spelling is selected yet. Do not ban a storage origin by name
when its actual lifetime and backend requirements are what determine validity.

### Persistence and distributed behavior

Submission, I/O completion, and durable persistence are different states.
Database APIs must express the required flush/FUA and recovery protocol rather
than treating ordinary completion as a durable commit.
[Linux writeback-cache documentation](https://www.kernel.org/doc/html/latest/block/writeback_cache_control.html)

Network delivery, timeout, cancellation, retries, and crash recovery likewise need
explicit library protocols. A memory-safe type system does not automatically prove
deadlock freedom, exactly-once effects, or distributed correctness.

## 12. Composition and extensibility

The design should make useful forms of composition explicit without putting every
behavior into one universal attribute mechanism.

| Kind | Composition rule to investigate |
|---|---|
| Checked contracts | Combine requirements; reject contradictions; expose expansion. |
| Function dependencies | Pass typed arguments; retain effect and lifetime information. |
| Runtime wrappers | Preserve visible order, failure, cancellation, and ownership behavior. |
| Layouts and codecs | Compose compatible representations and access capabilities. |
| Schedules | Compose ordered transformations with checked preconditions. |
| Resource protocols | Permit legal transitions and track who may access each resource. |

Research avenues from the conversation include capability/capture tracking,
linear or one-shot handlers, permission tokens for shared structures,
resource-coordinated concurrency, and independently checked optimization evidence.
These do not require adopting implicit dependencies in ordinary function headers.
See [the research map](research.md#composition-permissions-and-checked-optimization).

A unified typed execution plan combining contracts, backends, protocols,
representations, and transformations remains an integration hypothesis. It needs
a small prototype and measured costs before it becomes a language commitment.
General continuation capture/replay, arbitrary compiler plugins, and runtime
metaprogramming must not be assumed allocation-free or compatible with exclusive
resources.

## 13. Operating systems, native interfaces, and accelerators

The design agenda includes:

- A freestanding subset with no mandatory OS, tracing collector, or scheduler.
- Native ABI interfaces, explicit external representations, and library interop.
- Raw memory, volatile/device access, atomics, alignment, and explicit unsafe
  boundaries with documented obligations.
- CPU feature selection, portable fallbacks, vector types, and specialization.
- Device memory spaces, addressability, transfers, stream/event ordering, and
  host/device ownership when accelerator support is added.
- Shape, stride, alignment, and lifetime information that survives library calls
  and compiler lowering where possible.

An unsafe boundary does not suspend all contracts automatically; imported code
needs a declared and auditable effect/lifetime model. The initial target list,
backend, executable/object formats, ABI guarantees, and kernel integration remain
open.

AI hardware motivates minimizing data movement as well as arithmetic cost.
FlashAttention and PagedAttention are reasons to support tiling and flexible
block-based storage, not reasons to bake one model architecture into the language.
[FlashAttention](https://arxiv.org/abs/2205.14135),
[PagedAttention](https://arxiv.org/abs/2309.06180)

## 14. Human and AI tooling

### Readable source, inspectable meaning

The toolchain should expose:

1. Parsed declarations and resolved types.
2. Copy, borrow, ownership, lifetime, and inferred effect information.
3. Layout sizes, alignment, offsets, encoding/access capabilities, and conversions.
4. Transformation legality, selected implementations, generated code, and missed
   optimization explanations.
5. Structured diagnostics with source locations, violated requirements, and the
   relevant chain of calls or dependencies.

Provide human-readable and machine-readable forms. AI tools should be able to
query and edit compiler-checked structures rather than infer everything from
free-form prose. A formatter and a small canonical grammar are part of this work.

**Experimental tooling status, November 10:** the Rust compiler library now
provides lossless source ranges, lexical highlight categories and syntax-preserving
formatting. `holo highlight` adds optional terminal headers, line numbers/frame and
CSS-like style overrides; `holo fmt` shares its engine with the VS Code formatting
provider. Formatting preserves identifiers and exact non-whitespace token/comment
spelling, reparses output, and checks syntax structure without spans. It applies
four-space indentation and the existing vertical-header working layout to the
current experimental grammar. Formatting has no line-width reflow and
validates syntax alone. Naming advisories run on the compilation path.
See [architecture](compiler.md#host-source-tools)
and [editor integration](../editors/vscode/README.md).

The current header may improve comprehension, but that is a hypothesis to test.
More explicit text also has token and familiarity costs. Grammar-aware generation
has research support; no cited work establishes that this exact syntax beats a
conventional function header.
[Synchromesh](https://arxiv.org/abs/2201.11227),
[Grammar Prompting](https://arxiv.org/abs/2305.19234)

AI-generated optimization proposals should pass the same legality checks and
correctness tests as human proposals. Tools should make a weakened contract or
numeric relaxation visible during review. Successful compilation is not a proof
of application correctness or a guarantee of improved performance.

## 15. Validation and performance evidence

### Goose performance compatibility checklist

**Status: proposed cost objectives and acceptance cases, not achieved compiler
behavior.** Bulk reclamation is required by D13. The following mechanisms need
their own specification and validation; cheap freeing alone does not establish
equivalent construction, traversal, or execution performance.

| Objective | Working target | Acceptance case |
|---|---|---|
| Destination construction | Build eligible results in their final variable, field, or appended slot through calls. | Parse a nested record directly into a collection; inspect temporary storage and copies. |
| Stable, cheap growth | Supported extension preserves existing addresses with bounded marker bookkeeping. | Retain an element reference while growing its array; inspect relocation and capacity-management work. |
| Flat nested data | Inline eligible strings, arrays, and variable-sized fields without mandatory per-child pointer/header allocations. | Inspect and traverse a batch of nested records; count allocations, metadata, and bytes touched. |
| Compact representation choices | Support narrow metadata/links and explicit alignment alternatives. | Compare compact and full-width/aligned layouts with the same logical workload. |
| Tagged alternatives | Evaluate fixed/indexable and compact/sequential representations with tag-based dispatch. | Inspect variant size, access cost, dispatch, and representation-change restrictions. |
| Static abstractions | Provide specialized paths for generics and compile-time-known callbacks. | A filter/comparator has no required closure allocation or indirect call on its static path. |
| Checks and target specialization | Prove removable checks and support target-specific implementations under defined numeric rules. | Inspect remaining checks and generated variants; reject an unsupported checked guarantee. |
| Error propagation | Avoid mandatory allocation/unwinding and unnecessary payload copies on eligible paths. | Inspect and measure normal and error paths, including payload construction and cleanup. |
| Bulk interoperability | Define eligible flat messages and validated relocatable byte images. | Account for copied bytes and validation; establish when pointer fixups are unnecessary. |

The first three are the next baseline priorities. Representation choices and
static execution paths must also remain expressible, with their costs visible.
Sources: [Goose overview](https://aardappel.github.io/goose/),
[Goose reference](https://aardappel.github.io/goose/docs/reference.html),
[Goose benchmark design](https://aardappel.github.io/goose/bench/design.html).

Do not turn this checklist into universal packing, universal static dispatch, or
a claim that every object can grow. Compact sequential variants, relative links,
alignment, dynamic callbacks, and external representations have different
requirements. Define the eligible path and measure the alternative.

Destination construction also differs from copying an existing value. Keep the
explicit `copy` semantics in function headers while eliminating unnecessary
intermediate materialization. Branches returning different existing locals,
ownership transfer between regions, and source preservation need explicit rules;
do not promise all of them are free.

The current Goose reference distinguishes returning locals and destination
placement cases. Treat older benchmark TODO text about moves as historical context,
not a blanket assertion that every Goose return copies. Similarly, messages must
meet the queue's eligibility rules; copying a serialized byte image is distinct
from directly sending an arbitrary relative-reference structure.

#### Initial checked IR coverage

This ledger describes the first implementation increment, not completion of the
compatibility checklist. The experimental source adapter shares this core;
evidence and restrictions
are linked from [the source guide](../compiler/src/README.md) and [M1 plan](implementation-plan.md#m1--implement-the-checked-region-subset).

| Mechanism | Current evidence or remaining gap |
|---|---|
| Data-stack assignment / backing | Explicit IR/source regions and one live batch per region. The [bounded-policy acceptance](implementation-plan.md#bounded-backing-acceptance--november-9) validates many-cycle reuse, finite exhaustion, allocation-failure cleanup and live-page retention under bounded competing demand. [One-site placement](#compiler-selected-backing-for-one-scoped-aggregate) selects finite backing through programmatic input or experimental source. General assignment, VM backing, genuine pressure, generic commitment and Holo trim/recommit remain pending or unavailable. |
| Destination construction | Checked acyclic producer calls, including fresh source record returns and synchronous readonly byte inputs, write directly into final slots. Typed nested input forwarding passes location/extent without an aggregate temporary; general source functions/returns remain pending. |
| Stable, cheap growth | Retained-reference native cases pass; the matched campaign's assembly retains component overflow/capacity guards. Stable growth is implemented; check-free cost parity is unestablished. |
| Flat nested storage | Packed inline nested records/payloads. Preserved wide Holo uses 47.5 MiB in the largest fixture; derived/compact modes pass independent 16/12-byte-prefix conformance. Fresh-construction and complete-field readonly-input diagnostics separately combine provenance with exact receiving coverage across 672 traced generations each. The latter observes 58,848 required input reads/final writes without accepted staging. General aggregate transfer, owned-error payloads and physical traffic remain unestablished. |
| Bulk release | Scope/reset paths restore fixed markers without record walks. The ten-shape one-marker matrix validates release/reuse at O0/O2; the [cleanup-path audit](implementation-plan.md#generated-cleanup-path-acceptance--november-9) adds original/optimized/native explicit reset, rollback, multi-owner exits and acquisition recovery, with 252 native and 45 independent model outcomes. Native checkpoint/spill work, eliminated private stores, construction-inclusive failure windows and allocator frees are distinguished. The retained-backing diagnostic observes unchanged resident pages across all 384 full/reset pairs. Physical latency, helper-effect contracts, pressure/trim policies and general commitment remain separate limits. |
| Compact metadata / relative links | Explicit header-derived u64 and compact u32 byte-length modes are implemented through checked schema planning and one backend. Native/model tests cover zero/max/over-width, rollback, complex retained headers, and empty units. Relative links, general narrow scalars, alignment alternatives, and automatic width inference remain deferred. |
| Tagged alternatives | Deferred beyond this plain-data slice. |
| Static abstractions / optimized checks | Statically resolved constructor calls and LLVM O3. The [macOS arm64 byte-sum lowering](../evidence/outlined-byte-sum-20261110/README.md) proves exact bounds for a full-span wrapping reduction and replaces per-byte checks with checked-width loads in a separate native function. Eligible [tiny literal prefixes](../evidence/static-tiny-byte-sum-adoption-20261110/README.md) select exact-width reductions after argument capture and validation. The [macOS arm64 Derived64 default](../evidence/exact-192-default-20261112/README.md) admits the checked exact 192-byte prefix to its existing wide helper. General generics, source callbacks, checkfree contracts, and target multiversioning are pending. |
| Failure paths | Small construction statuses and append rollback tested at field boundaries. Experimental acquisition handlers can now finish their failed owner block and resume an enclosing continuation, with explicit retry and exact cleanup checked at O0/O3/ASan. The [ordinary sequence campaign](implementation-plan.md#ordinary-construction-failure-and-retry-costs--november-9) covers actual attempt/conditional-retry/consumption without output barriers, all layouts and nested calls. Its costs include caller accounting and successful construction; isolated rollback/error bounds, general errors/unwinding, owned-error payloads and streaming-byte failures remain outside the evidence. |
| Messages / images / pools | Deferred; internal flatness does not establish queue eligibility, validated serialization, or stable-handle semantics. |

The [first matched lifecycle campaign](../evidence/lifecycle-comparison-20261109/report.md)
and [generated-code/stability review](../evidence/lifecycle-comparison-20261109/generated-review.md)
support these scoped observations. All 270 native samples validate, but working-set
timing drift, different trust/check boundaries and backing policies, and Rust's
different LLVM version limit conclusions. Holo's matched-Rust gap is inconclusive;
Goose is consistently faster in this local workload. Neither outcome settles
overall language performance, automatic storage assignment, or unimplemented
mechanisms. The later [finite M1 acceptance](implementation-plan.md#m1-acceptance-decision--november-10)
distinguishes demonstrated cases and the reviewed native profile from measurement
limits and deferred features. It does not update these historical timings or
establish overall language performance.

The later [phase campaign](../evidence/lifecycle-phases-20261109/report.md) adds900
validated measurements with unchanged combined controls and separate diagnostic
clocks. These diagnostics point to construction for the remaining local Goose gap;
scan costs are close. Its scope-exit windows remain unresolved: clocks can dominate and private
marker updates can move or disappear. This neither changes the baseline guarantees
nor completes isolated reset scaling or dynamic memory-accounting acceptance.

The later [populated scoped-release matrix](implementation-plan.md#populated-scoped-release-scaling--november-9)
adds real count/payload/inline-depth variation and actual reuse under one owner,
with 70 native profiles and 5,600 validated releases including preflights. Original
checked Holo code has one lexical marker restoration for every tested shape;
O2 removes it from the observed gap. Inline wrappers introduce no independent
owners. Timer-floor zeros do not establish physical reset latency, and this does
not establish cost independence from the number of separate marker owners, OS
decommit, external cleanup, or general Goose compatibility. Compiler semantics
and preferred rules are unchanged.

The [retained-backing diagnostic](implementation-plan.md#retained-backing-and-resident-pages--november-9)
adds 48 observations of unchanged Holo/C/Rust/Goose O2 controls at 1,152 clock
checkpoints. All 384 full/reset pairs preserve selected backing residency; exact
page/mapping accounting distinguishes this from logical used bytes and whole-task
totals. Goose's virtual reservation is not resident RAM. Final allocator release,
OS process teardown and explicit trim remain separate; no baseline guarantee or
preferred backing policy is changed and generic committed bytes stay unavailable.

The [fresh-construction copy diagnostic](implementation-plan.md#fresh-construction-and-copy-accounting--november-9)
adds 84 uninstrumented native controls and 168 traced observations across fourteen
source cases and three layouts at O0/O2. Its payload writes match semantic
initialization exactly; original destination provenance separately excludes
aggregate staging in the admitted constructor subset. Known scratch copies and
overwrites calibrate detection, including copies erased or scalarized by O2.
Observable barriers can retain failed private writes, and optimized-IR events are
not physical traffic. No general existing-input-copy, owned-error, copy-elision or
language-performance guarantee is selected by this evidence.

The [readonly-input diagnostic and matched comparison](implementation-plan.md#readonly-input-accounting-and-comparison--november-9)
extend evidence only for the implemented synchronous complete-byte-field borrow.
All three layouts pass source/destination range checks, aliased same-prefix input,
partial failure, retry and reset/reuse. Source payload reads and final writes match
the 58,848-byte initialization work; independent original and optimized audits
reject unexplained temporary or spare-backing staging. Uninstrumented comparisons
retain both strong known-span and record-backed input controls with their actual
optimizer/checking/backing differences. These observations add no alias promise,
new storage policy, public source rule, physical-traffic bound or language ranking.

The [ordinary failure/retry study](implementation-plan.md#ordinary-construction-failure-and-retry-costs--november-9)
separately observes dynamic valid/failing requests through the same checked hot
call path. No output opacity or producer callbacks force failed private work to
remain. Complete published-data/status guards precede the end clock, and exact
untimed checks validate every output/source byte and retained prefix. Late checks
still follow payload work in inspected optimized producers; this is generated-code
evidence, not a rule requiring failed writes to occur. The finite measurements do
not change M1 failure order, publication, aliasing, backing or cleanup semantics.

A later [remaining-capacity trial](implementation-plan.md#remaining-capacity-guard-experiment--november-9)
preserved the M1 rules while reducing native success-path guards. Its small default
case regressed and larger default gains were inconclusive, so the original lowering
was restored. This rejected experiment adds failure-order coverage, not a new growth
guarantee, compiler policy, or proof of check-free parity.

A separate [byte-initializer width trial](implementation-plan.md#byte-initializer-width-experiment--november-9)
proved low-byte projection for the existing closed wrapping-expression subset,
but its measured construction path regressed. The original lowering remains
active. Tests protect unchanged byte/scalar semantics; no numeric default,
encoding, lifetime rule or new performance guarantee was adopted.

The later [construction branch-layout trial](implementation-plan.md#construction-success-branch-layout-trial--november-10)
preserves the same checked semantics while testing success-biased LLVM metadata.
Its 5,210 measured observations and 337 preflights include nine favorable
lifecycle comparisons and a repeated wide 4,096-byte copy regression. It is not
adopted: all 52 captured compiler inputs were restored after that trial, with no
broad branch-weight flag or format-specific fallback. Metadata-only original-LLVM equality across
115 actual module pairs and finite native checks do not establish a new language
rule or a universal performance guarantee. M1 access, ordering, stable growth,
publication, rollback, compact representation and fixed-marker release remain
unchanged; later optimizations still require broader evidence.

The [narrow/vector follow-up](implementation-plan.md#narrow-construction-and-vector-follow-up--november-10)
accepts a private success-layout heuristic only for all-U64 producer signatures and
appends to those producers. The checked forwarding rules exclude transitive byte
borrows from that scope; even unused Bytes formals remain ineligible. This is a
compiler choice with unchanged checks, failure order, publication and rollback,
not a public annotation, uniqueness claim, layout rule or new performance contract.
Its default lifecycle paired costs improve about 3% across the three tested sizes,
while full-copy intervals cross zero and ten native copy pairs are unchanged.
The vector-only alternative is not adopted: original scalar source and exact
prefix stores do not guarantee identical or faster native tails. Both independent
full matrices and screenings retain 11,512 measurements and 752 preflights in the
[record](../evidence/construction-followup-20261110/README.md), with final rebuilt
validation separately established. The Goose mechanism ledger and deferred
sharing, queues/images, pools, backing and asynchronous features are unchanged.

The subsequent [retained-binary profile](implementation-plan.md#retained-binary-cpu-attribution--november-9)
adds exact-PC sampling of the production and rejected byte-width outputs. Its 36
captures preserve unknown/external observations and show a consistent redistribution
toward scalar/test/transition code. This is local optimization evidence, not a new
numeric rule, check-elimination guarantee, isolated latency result or completed
Goose-compatibility gate. Production lowering and the mechanism ledger above remain
unchanged; the next candidate must preserve their semantics and pass new timing
acceptance.

The [allocation/transfer API campaign](implementation-plan.md#allocation-and-transfer-api-diagnostics--november-9)
adds independently validated observations of unchanged native controls. No covered
API entry occurs in their workload windows; outer requests, nested delegation,
runtime transfer aliases and setup/output activity remain separate. This is not
strict `noalloc`, zero semantic copying, constant-time reset or OS-memory evidence.
The mechanism ledger above remains scoped: inline copy-volume and remaining
reclamation/backing gates are still open, with no language rule or lowering change.

### A fair comparison matrix

Use both an approachable baseline and an expert-tuned representation where they
answer different questions. Report the same algorithm, data, allowed numeric
behavior, safety requirements, build settings, and resource limits. Include
construction, steady-state work, conversion, failure, and teardown where relevant.

| Family | What to measure |
|---|---|
| Flat scalar loops | Baseline overhead, SIMD, generated instructions, and bandwidth. |
| Nested/variable-sized records | Allocation count, construction copies, traversal, and teardown. |
| Trees, graphs, and pools | Link width, cache behavior, slot reuse, deletion, and nonnested lifetimes. |
| Columnar and compressed data | Scan, lookup, mutation, conversion, memory footprint, and decode work. |
| Multicore services | Throughput, tail latency, queueing, backpressure, placement, and contention. |
| Network/NVMe paths | Batch sizes, outstanding work, completion handling, CPU use, cancellation, and durability costs. |
| Accelerator kernels/serving | Transfers, working sets, layouts, tiling/fusion, and allocation/reuse costs. |
| AI programming tasks | Correctness, repairs, diagnostics, tokens, time, and semantic regressions. |

Benchmark multiple working-set sizes, distributions, and target architectures.
Record versions, compiler flags, CPU features, topology, storage/device details,
warmup, repetitions, uncertainty, and raw results. Separate virtual reservation,
commit, resident memory, and useful payload. Use hardware counters and assembly
where they help attribute a result, rather than treating them as scoreboards.

### Correctness checks

- Independent reference models and differential tests for collection operations.
- Compile-fail cases for invalid borrows, copies, effects, lifetime escapes, and
  unsupported layout access.
- Allocation/capacity-failure cases, partial initialization, and cleanup accounting.
- Schedule legality tests, numeric edge cases, bounds, and error-order preservation.
- Deterministic concurrency/cancellation tests and small model-checked protocols
  where practical.
- Crash/recovery tests for persistence APIs, separate from ordinary I/O completion.
- Verification of selected compiler rewrites within a clearly stated model.

Label conclusions accurately: **measured**, **tested**, **statically checked**, or
**proved under stated assumptions**. No single label implies all the others.

## 16. Design agenda and next decisions

Work through these areas in dependency order. A checked box requires a concrete
decision or evidence linked from this document; mentioning a feature is not enough.
The stages organize the full design agenda. The immediate executable subset
crosses those stages; use [the implementation plan](implementation-plan.md) to
keep that subset small and testable.

### Stage A: a coherent core

- [x] Record the fresh design's purpose, preferred syntax, and research agenda.
- [ ] **Q01 — Header grammar:** clause termination, multiline types, defaults,
  anonymous functions, empty returns, and the role of whitespace.
- [ ] **Q02 — Access and passing:** binding versus object mutability, deep access,
  copy eligibility, references, omitted modes, and compatibility between retained
  element references and structural growth.
- [ ] **Q03 — Ownership and lifetimes:** transfer spelling, reborrowing, escaping
  references, destruction, partial initialization, and failure ownership.
- [ ] **Q04 — Core values and operations:** literals, arithmetic, numeric edges,
  structs, alternatives, options, text/bytes, expressions, and control flow.
- [ ] **Q05 — Functions and abstraction:** methods, callbacks, generics, interfaces,
  effect signatures, static/dynamic dispatch, separate compilation, and a static
  callback path without mandatory closure allocation or indirect dispatch.
- [ ] **Q06 — Errors:** representation, propagation, cleanup, panic, unwinding,
  allocation/capacity failure, diagnostics, and payload construction/copy costs.
  See [typed results, construction, and handling](error-handling.md).
- [ ] **Q07 — Contracts:** precise `noalloc`/`noblock` boundaries, inference
  implementation/export under D18, callbacks, foreign calls, and compiler-generated work.
- [ ] **Q08 — Modules and build:** visibility, namespace resolution, packages, compile-time
  evaluation, reproducibility, target selection, and ABI boundaries. D19 selects
  the `holo` standard-library prefix, D21 selects source naming conventions,
  D22 selects `public func` for the experimental leaf modules, and D23 selects
  `namespace` for source namespace declarations. D24 defines visibility and
  dependency pinning for the experimental local package profile. D25 selects
  `using` for file-local namespace qualification.
  The [remaining package decisions](packages.md#implementation-boundary-and-remaining-decisions)
  stay open.

**First review topics:** Q02, Q09, and Q10 together for the narrow region subset.
Resolve the relationship between copying, access, reference validity, supported
growth, and construction destinations before expanding the keyword vocabulary.

### Stage B: memory and physical data

- [ ] **Q09 — Allocation and regions:** fixed frames, dynamic arenas, data stacks,
  pools, growth, bounded environments, and resource cleanup. Decide which strict
  Goose growth/lifetime invariants the baseline adopts, and specify the rules
  that establish D13's required bulk-reclamation cost.
- [ ] **Q10 — Destination construction:** guaranteed cases, fallible initialization,
  branching returns, and container insertion. Establish D14's eligible guaranteed
  paths, including nested callees and variable-sized values.
- [ ] **Q11 — Compact and relocatable data:** relative links, identity, validation,
  stable addresses, offsets, packed alignment, and persistence formats.
- [ ] **Q12 — Layout/access foundation:** rows, columns, blocking, custom mappings,
  codecs, references versus decoded accessors, operation availability, inline
  metadata, and fixed versus variable-sized tagged alternatives.
- [ ] **Q13 — Collection behavior:** append, lookup, update, deletion, capacity,
  conversions, rollback, and effect/cost visibility.

### Stage C: execution and systems integration

- [ ] **Q14 — Memory model:** atomics, races, synchronization, sharing, and reclamation.
- [ ] **Q15 — Scheduling:** stable loop references, legality, numeric permissions,
  specialization, schedule libraries, and compiler representation.
- [ ] **Q16 — Async protocols:** task storage, captures, suspension, cancellation,
  completion lifetimes, and ownership on every exit.
- [ ] **Q17 — Runtime policies:** queue access, backpressure, thread/core/NUMA
  placement, polling, interrupts, fairness, and progress requirements.
- [ ] **Q18 — OS/native/device interfaces:** freestanding support, FFI, device memory,
  host/device transfers, and resource registration.
- [ ] **Q19 — Persistence/distribution:** flush protocols, durable operations,
  retries, failure models, and recovery.

### Stage D: prove usefulness

- [ ] **Q20 — Readability and AI evaluation:** matched tasks, examples, repairs,
  canonical formatting, compiler queries, and measured token/correctness costs.
- [ ] **Q21 — Performance evaluation:** reproducible baselines and causal experiments
  against Goose, Rust, C/C++, and relevant domain libraries.
- [ ] **Q22 — Advanced composition:** capability scopes, handlers, permission tokens,
  resource-oriented concurrency, and typed execution plans.
- [ ] **Q23 — Optimization evidence:** independently checked rewrites, trusted
  primitives, verification limits, and reviewable AI-generated schedules.

### Proposed first end-to-end milestone

The [implementation plan](implementation-plan.md) records the initial
four-language control. The first checked region program exercises:

1. The preferred header and explicit scalar-copy/reference access modes.
2. Unsigned fields, variable-sized byte payloads, and one defined growable
   construction destination.
3. Eligible producers that construct nested records directly in their final storage.
4. Supported nonmoving growth, valid retained element references, and rejection of
   invalid escapes or resets.
5. A scoped-data release path with fixed marker bookkeeping and no per-object
   reclamation work, including defined partial-construction failure behavior.
6. Independent checksum checks, representation/generated-code inspection, and
   matched construction, scan, release, and memory measurements.

The telemetry/layout/SIMD tour remains a later target. After the region subset,
evaluate relative-link structures, nonnested pool workloads, and one measured
layout or scheduling improvement. Hardware I/O should follow once buffer
ownership and completion lifetimes have a concrete model.

### How to evolve this document

For each decision, record the problem, candidate rules, a readable example, an
invalid example or boundary case, expected cost, research basis, and validation
needed. Update the canonical tour when a decision changes it. Preserve older
choices as decision history rather than allowing contradictory examples to become
an accidental specification.
