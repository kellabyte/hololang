# Design documents

This directory contains the authoritative fresh Holo design. Keep it small:
extend the relevant document rather than accumulating overlapping proposals,
status reports, or copied conversations.

| Document | Responsibility |
|---|---|
| [spec.md](spec.md) | Language purpose, syntax, identifier naming and diagnostic policy, semantics, decision statuses, and open questions. |
| [error-handling.md](error-handling.md) | Typed results, `on … then` handling, and `return ok Frame:` construction, including checked append rollback, lowering, and acceptance gates. |
| [packages.md](packages.md) | Selected `holo` standard-library prefix, file-scoped `using` directives, and open package design. The specification records decision statuses. |
| [compiler.md](compiler.md) | Compiler architecture, checked representation, backend boundaries, optimization, and staged testing strategy. |
| [implementation-plan.md](implementation-plan.md) | Immediate scope, executable milestones, testing/measurement gates, and evidence links. |
| [milestone-0.1.md](milestone-0.1.md) | Public preview target, bounded RESP2 GET/SET server, and staged acceptance checks. |
| [research.md](research.md) | Primary sources, applicability, limitations, and research directions. |
| [research/](research/README.md) | Six hardware research notes with worked explanations, Holo applicability, and future prototype plans; these do not define additional language rules. |

Read the specification and current milestone before implementation; use the
compiler document to place each responsibility. Research informs decisions but
does not automatically approve features. The specification's status labels remain
meaningful when a compiler design mentions an intended capability.

For the current stopping point, read the plan's
[M1 acceptance decision](implementation-plan.md#m1-acceptance-decision--november-10).
The finite checked core is accepted under its explicit Darwin arm64 native profile.
The historical [November 9 review](implementation-plan.md#m1-acceptance-review--november-9)
records earlier gaps, and the [cleanup audit](implementation-plan.md#generated-cleanup-path-acceptance--november-9)
retains its independent native/model evidence. The helper follow-up now implements
target traps, eager binding and a reviewed memcpy/memset/bzero profile. Unresolved
physical reset timing, unavailable commitment and deferred features remain limits.
The specification's [inference policy](spec.md#inference-and-explicit-promises)
records the approved simpler surface: infer provable effects and keep explicit
promises optional, without claiming that full native verification is implemented.
The user has directed completion of native-contract work and M1 acceptance before
another comparison campaign. The plan's [concrete decision package](implementation-plan.md#concrete-decision-package-for-m1)
records the approved failure/runtime policy, completed native acceptance and the
distinction between trusted profile assumptions and a universal effect proof.

## Implementation navigation

[compiler](../compiler/README.md) owns the Rust compiler,
[cli](../cli/README.md) owns the `holo` command, and
[runtime](../runtime/README.md) owns native target support.
[lib/holo](../lib/holo/README.md) contains Holo-written standard-library
modules. [Examples](../examples/README.md) contains the TCP echo and Redis
protocol demos. Holo programs used by regression checks live in
[test fixtures](../tests/fixtures/README.md). Rust API examples stay with the
compiler.

Disposable generated output belongs in [build](../README.md#build-output). Cargo's cache is
under `build/cargo/`, API-example output under `build/examples/`, benchmark work
under `build/benchmarks/`, and retained failing test inputs under `build/failures/`.
Preserved runs live separately in [evidence](../evidence/README.md), whose index
groups the existing campaigns, correctness records, and exploratory probes.
Cleaning build output must not erase those records.

## Boundaries and tradeoffs

The language specification defines observable behavior. The compiler document
describes how to establish and implement it. The plan decides what to build next.
Update these together when an approved decision crosses their boundaries. Keep
implementation status and measured evidence distinct from intended guarantees.

Use the [current source guide](../compiler/src/README.md) for supported features.
Link archived evidence with its actual labels and conditions, and preserve the
recorded inputs and results.

Before completing documentation changes, validate local links and anchors,
balanced code fences, current commands/paths, and consistency of status claims.
No compiler test run is needed solely to change prose.
