# Holo source preview

Holo is an experimental native systems language. This source preview pairs a
Rust compiler that emits LLVM with a small Holo standard library and a bounded
RESP2 server. Language syntax, the Rust compiler API, and native calling
conventions may change during the preview.

- The compiler checks lexical lifetimes for bounded regions, batch elements,
  and borrowed byte views. Destination construction writes records directly
  into a receiving slot and publishes successful records. A single scoped
  record can be bound with writable access inside its success arm.
- `break` and `continue` transfer to the innermost loop and run cleanup for
  the scopes they leave.
- Enums name states stored in locals and record fields. Redis uses
  `Command.Get` and `Command.Set` for parsed requests.
- The `u64` operators `+`, `-`, `*`, `/`, and `%` trap on overflow,
  underflow, or division by zero. Explicit wrapping operations are available
  for code that needs modulo arithmetic.
- Local manifest packages resolve source namespaces and lock dependency
  versions and content. The `holo` CLI also provides syntax highlighting,
  formatting, diagnostics, and LLVM and layout inspection.
- Typed `Result` values support recoverable errors and explicit propagation.
  `Result<Unit, E>` lets socket helpers report success with `return ok`.
  Save an `Option<u64>` lookup result in a local binding and handle it later.
  Ordinary helpers can take and return `bool` values. Booleans can be stored
  in mutable locals and record fields, including Redis's slot occupancy flag.
  The standard library includes bounded byte search, integer readers, byte
  writer cursors, decimal parsing and writing, buffer operations, and socket
  helpers.
  [Standard library](../../lib/holo/README.md)
- The Redis example handles plain `GET` and `SET` over RESP2. It listens on
  IPv4 loopback, serves one client at a time, and keeps up to 16 binary keys
  across connections. Keys and values have limits of 32 and 128 bytes.
  [Redis demo](../../examples/redis_server/README.md)

## Verification

The local preview passed on macOS arm64 with LLVM 22.1.2 and `redis-cli` 8.2.1.
Run `make check` to build the compiler in a
fresh Cargo target directory, verify the exact Hello World output, replay
the Redis parser against its independent Python oracle, and check stream
replies and real `redis-cli -2` sessions. The loopback check exercises an
overwrite, an oversized request, reconnects, and a missing key.
[Setup and prerequisites](../milestone-0.1.md#public-preview-gate)

Run the workspace tests with `cargo test --locked --workspace`. The preview
workflow is configured for a clean checkout on GitHub's macOS arm64 runner.
GitHub Actions execution is pending.
[Preview workflow](../../.github/workflows/preview.yml)

The source distribution includes the maintained tests and demo verifiers.
[Documentation index](../README.md) · [Test guide](../../tests/README.md)

## Source archive

From a Git checkout, run `make archive` to create
`build/release/holo-0.1.0-rc-source.tar.gz` and its `.sha256` file. The archive
captures the current bytes of tracked files and unignored new files, including
uncommitted changes. Review `git status --short` and use a clean checkout of the
intended revision for a public release. To create a
second archive without replacing the first, run
`python3 -B tools/archive.py --output build/release-replay`.

Verify and test the archive from the repository root.

```sh
cd build/release
shasum -a 256 -c holo-0.1.0-rc-source.tar.gz.sha256
tar -xzf holo-0.1.0-rc-source.tar.gz
cd holo-0.1.0-rc-source
make check
```

The source distribution carries the repository's MIT license. The locked
Rust dependencies are fetched from crates.io or read from Cargo's existing
cache. LLVM, Redis, and compiled binaries are installed separately.
[License](../../LICENSE) · [Locked Rust dependencies](../../Cargo.lock)
