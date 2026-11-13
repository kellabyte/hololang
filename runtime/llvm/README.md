# LLVM runtime prelude

`prelude.ll` is embedded into every emitted LLVM module by the
[compiler backend](../../compiler/src/backend/llvm/README.md). It defines the native
region/batch metadata shapes, libc and LLVM declarations, output format, and a
terminating assertion helper. It contains target-program LLVM, not Rust code that
generates LLVM and not Holo standard-library source.

The emitter owns reservation checks, record representation, bounds checks,
publication, and lexical cleanup. Keep compiler-generated algorithms there.
The prelude preserves packed/unaligned data rules and supplies no unchecked alias
promise or per-object lifetime machinery.

`holo.assert` returns on a true predicate and calls `llvm.trap` followed by
`unreachable` on false, before the protected access. The fatal edge performs no
Holo formatting, libc abort, cleanup or callback. Host handling after the target
trap transfer lies outside Holo's execution-contract boundary. Termination signal
and debugger behavior are target dependent; the audited Darwin arm64 BRK produces
SIGTRAP. This boundary neither grants arbitrary external callees effects nor
changes recoverable construction statuses or normal owner cleanup.

Run `cargo test --offline --test native --test runtime_bounds --test entry_exit`
from the repository root with matched LLVM tools. Verification of emitted modules
and native execution are the build/check boundary; this file is not a standalone
executable or a separately linked library.
