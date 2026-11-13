# Rust compiler API examples

These programs exercise the experimental Rust compiler library. Native examples
drive real LLVM tools. Source-tooling examples need no LLVM. Runnable Holo
applications live in [examples](../../examples/README.md), and compiler test inputs
live in [tests/fixtures](../../tests/fixtures/README.md). The Cargo package for the API
examples is `holo-compiler`; they use its existing `holo` Rust library.

`region.rs` checks structured IR, emits LLVM, and builds/runs native O0/O3 programs.
The workload lives in [region/program.rs](region/program.rs), separately from the
tool-driving example. These are compiler API examples, not parsed Holo source.

Run `cargo run --offline -p holo-compiler --example region -- build/examples/region` from the repository root.
The output directory must be new. LLVM, native binaries, tool identities, and phase
timings are retained there. No dependency downloads or legacy imports are needed.
Without an explicit output argument, the example uses `build/examples/region`.

The example covers nested destination construction, retained aliases during
growth, byte mutation, failure rollback, marker reuse, and failed backing acquisition.
See [the source guide](../src/README.md) for current limitations and design boundaries.

`measure_region.rs` builds one optimized native lifecycle workload and records
sequential checked samples. Run, for example:

```sh
cargo run --offline --release -p holo-compiler --example measure_region -- build/examples/region-measurement 512 1000 7
```

Arguments select a fresh output directory, records per cycle, reuse cycles, and
sample count. The native clock covers construction, a materialization barrier,
linear scanning, and marker reset; allocation/free and validation stay outside.
The output archives exact tool commands, full compiler input/LLVM, source and
binary identities, output streams, raw samples, and min/median/max. This is an
experimental checked-IR measurement, not a C/Rust/Goose comparison or a source
compiler benchmark. Run timing campaigns without concurrent tests/builds.
Its default output is `build/examples/region-measurement`. Both example outputs
are disposable build work until deliberately preserved under
[evidence](../../evidence/README.md); neither command overwrites an existing run.

Backing acquisition does not pre-touch every page. First-touch/page-fault costs
can remain inside construction, and a warmup process does not prepare later
processes' backing. Inspect sample trends and the [recorded local evidence](../../docs/implementation-plan.md#m1--implement-the-checked-region-subset)
before interpreting any median as steady-state throughput.

`highlight.rs` measures the actual borrowed lossless scanner and highlighting
iterator on a repeated source corpus. From the repository root:

```sh
cargo run --offline --release -p holo-compiler --example highlight -- tests/fixtures/region.holo
```

The optional argument selects a UTF-8 source file (default `tests/fixtures/region.holo`).
The source is loaded once and replicated to at least 1 MiB before timing. Each
phase receives five warmups, then 31 sequential samples of eight complete corpus
passes. Coverage/range verification, source construction, sample storage, JSON
formatting, and output are outside the measured loops. Raw nanosecond samples,
average, nearest-rank p50/p75/p90/p95/p99/p99.9, min/max, average-based MiB/s, and
measured allocation/reallocation counts are emitted as JSON. The allocator observer
delegates unchanged to Rust's system allocator and affects only this example.
Checksums are black-boxed so token ranges and highlighting categories remain
observable. Save working records under `build/benchmarks/`; deliberately preserved
results belong in `evidence/` with exact source/tool identities.

This is warm resident lexical throughput on one replicated corpus, excluding
terminal rendering/I/O, theme parsing, cold startup, strict parsing, LLVM, and
semantic resolution. Run without concurrent builds/tests or other measurement
campaigns. With only 31 samples, the highest quantiles are descriptive order
statistics, not calibrated tail-latency predictions. No old-implementation or
other-language comparison is performed, and allocation observations concern these
specific scan/highlight loops rather than generated Holo programs.

`emit_target.rs` checks one standalone source and emits Derived64 LLVM for
AArch64 or x86-64 Linux musl. It uses the requested target in the same
compiler library that the native CLI uses. It needs no target toolchain or
emulator. The caller verifies the emitted module and owns native linking.

```sh
cargo run --offline --release -p holo-compiler --example emit_target -- \
  aarch64-unknown-linux-musl tests/fixtures/typed_append.holo > build/examples/typed-append-linux.ll
```

`emit_staging_copy.rs` checks one standalone source and emits opt-in
Derived64 byte staging or batch traversal for the native macOS arm64 target.
The source, checker, and ordinary emit command remain the same. The tool
prints LLVM to stdout. The caller verifies and links it with the usual
runtime and records the executable separately from baseline.

```sh
cargo build --offline --release -p holo-compiler --example emit_staging_copy
build/cargo/release/examples/emit_staging_copy WORKLOAD.holo > build/examples/staging-copy.ll
```

Add `--stable-slots` after the source path to try descriptor reuse across
fragments. The trial uses distinct live region allocation identities and
emits a stable-slot marker in each admitted enclosing loop.

Use `--proved-slots` to exercise the release-shaped candidate. It keeps
ordinary scalar loops when the enclosing proof cannot establish distinct
live backing and stable field descriptors.

Use `--uniform-batches` to test constant cursor advances and byte bounds for
single-byte-field records when all appends to a lexical batch prove the same
initialized extent. This option retains the ordinary staging policy and the
original layout. Unproved batches retain the ordinary captured traversal.

Use `--outlined-bulk` to test the private fixed-block leaf at all proved
staging sites. The ordinary CLI already selects it for wide transfers at
sites with a proved maximum of 65–256 bytes. This API example also allows
larger proved sites, with the same inline dispatch for short transfers.
