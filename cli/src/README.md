# CLI source

`main.rs` is the executable boundary for the experimental Holo commands. It parses
arguments, renders errors, preserves native exit status, and refuses to overwrite
existing compiler artifacts. Source formatting has a separate explicit `--write`
contract. Compiler work is delegated to the local `holo` library from
[compiler](../../compiler/README.md).

`--library-root` selects the explicit trusted directory for resolving local
`using` directives. `main.rs` passes the entry path and bytes to the compiler, then
renders errors through the returned source map. Checking and layout inspection
remain LLVM-free with this option.

`--package` selects a manifest-backed package. The CLI delegates resolution,
snapshot ownership, and lock verification to the compiler's
[package component](../../compiler/src/package/README.md). With no entry,
`check` checks the root library's namespaces and their reachable dependencies, then
returns without invoking LLVM. Package actions that use the native driver
require an entry. The explicit `lock` action writes the resolved lockfile.

Checked source commands report [identifier naming convention](../../docs/spec.md#identifier-naming-conventions)
violations on stderr after source verification. `--strict-names` reports those
diagnostics as errors and exits before native tool discovery or artifact creation.
Use `holo check --strict-names <source.holo>` for an LLVM-free
naming check. With `--package`, naming diagnostics cover the checked files in
the root package. With `--library-root`, they cover the entry file. Formatting
preserves identifier spellings.

`highlight.rs` owns the LLVM-free `highlight` adapter: output-stream colour policy,
buffered ANSI rendering, optional filename header, line numbers and compact frame.
`style.rs` compiles [stylesheet data](../styles/README.md) once into a fixed category
table. `format.rs` owns `fmt` stdout/check modes and explicit atomic regular-file
replacement. `source_input.rs` acquires one UTF-8 file/stdin buffer for those text
tools; scanning/classification/formatting stay in the compiler library. Both
tools operate on the experimental source grammar.

This directory contains no lexer, checker, storage implementation, or LLVM backend.
Keep those responsibilities in their compiler components. Host argument and path
allocations are orchestration costs, separate from generated-program performance.

Run `cargo test --offline -p holo --test cli --test source_tools` from the repository root. Native CLI
checks require the matched LLVM installation described in the
[shared tests guide](../../tests/README.md).
