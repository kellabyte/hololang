# CLI tests

These suites invoke the Cargo-built `holo` executable and assert command behavior,
diagnostics, and artifact handling. The CLI suite also checks native compilation
and execution with LLVM.

Run from the repository root.

```sh
cargo test --offline --test cli --test source_tools
```

## Commands and diagnostics

- **Commands and output.** [`cli.rs`](cli.rs) checks default source commands, legacy
  flag compatibility, LLVM-free checking, source errors, UTF-8 invocation limits,
  emit/build/run, stderr timings, output protection, explicit physical modes, LLVM-free
  layout and copy inspection, and the full-width derived default.

- **Naming diagnostics.** Naming diagnostics cover declaration spans, warning and strict
  exit policy, semantic failure precedence, formatting preservation, and the pre-native
  build gate. The [package tests](../compiler/packages/README.md) cover naming policy for split
  contributors and entryless packages.

## Source presentation and formatting commands

| Suite | Coverage |
| --- | --- |
| [`source_tools.rs`](source_tools.rs) | Invokes the actual CLI for plain/ANSI output, filename headers, line numbers/frame, stylesheet overrides/errors, control-byte display, stdin, check status, atomic replacement, file permissions and symlink rejection. |

Run only `cargo test --offline --test source_tools` for the suite that requires no LLVM.
