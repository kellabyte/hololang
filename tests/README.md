# Holo integration tests

These suites exercise the compiler library, CLI, and native runtime. Private
unit tests live with their implementation. Shared [source fixtures](fixtures/README.md)
and [test helpers](support/README.md) live here.

| Group | Coverage |
| --- | --- |
| [Compiler frontend](compiler/frontend/README.md) | Source tools, parsing, checked source, and evaluation order |
| [Checked IR](compiler/check/README.md) | Definitions, diagnostics, access rules, and lifetimes |
| [Compiler storage](compiler/storage/README.md) | Layouts, reservation, initialization, and borrowed bytes |
| [Compiler backend](compiler/backend/README.md) | Optimization selection and native equivalence |
| [Compiler packages](compiler/packages/README.md) | Module resolution, namespace visibility, and lockfiles |
| [Native runtime](runtime/README.md) | Native linkage, guards, cleanup, and hosted I/O |
| [CLI](cli/README.md) | Commands, diagnostics, source presentation, and artifacts |
| [Semantics](semantics/README.md) | Reference interpreter and comparisons with compiled programs |

## Run tests

Run these commands from the repository root.

```sh
cargo test --offline --workspace
cargo test --offline --test frontend_native
cargo test --offline --test highlight --test format --test source_tools
```

The first command runs the workspace, the second runs one native suite, and
the third runs the source tools without LLVM. Native suites require matched
LLVM `opt` and `clang` plus an OS linker. Set `HOLO_LLVM_PREFIX` if discovery
needs an explicit toolchain prefix. See the [runtime tests](runtime/README.md)
for opt-in loopback cases.

Register each new integration suite in [cli/Cargo.toml](../cli/Cargo.toml)
with a target name and source path. The workspace root has no package to
discover the files. The CLI package gives the suites access to the Cargo-built
`holo` executable.

Failed native checks preserve replay inputs under `build/failures/`.
Run `make check` to verify Hello World and the Redis demo from a fresh
source snapshot. [Preview verification](../docs/releases/unreleased.md#verification)
