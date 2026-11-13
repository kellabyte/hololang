# Holo compiler

This Rust package implements parsing, checking, storage planning, LLVM generation,
and native tool driving for fresh Holo. Its package name is `holo-compiler`; its
public Rust library remains `holo` so existing compiler API users retain that path.
The [CLI](../cli/README.md) depends on this library and owns command-line behavior.

The library also exports LLVM-free `lex::scan`, `highlight::highlight`, and
`format::format` source tools. Their borrowed ranges and syntax-preserving text
results keep terminal/VS Code presentation separate from semantic compilation.

- [src](src/README.md) maps the implemented compiler components and current limits.
- [backend/llvm](src/backend/llvm/README.md) contains Rust code that emits LLVM.
- [examples](examples/README.md) contains Rust compiler API demonstrations.
- [runtime](../runtime/README.md) contains the native prelude and measurement support
  embedded or compiled by the compiler; these are not Holo standard-library code.
- [shared tests](../tests/README.md) exercise the compiler and CLI from the workspace.

The source frontend and structured IR share one checked pipeline. The LLVM emitter
requires the checker's private authorization token; moving these files does not
create another lowering path, change language semantics, or establish a speedup.
The public Rust API and supported Holo subset remain experimental. The finite
[M1 checked core is accepted under its reviewed native profile](../docs/implementation-plan.md#m1-acceptance-decision--november-10);
that does not stabilize APIs or complete the full language.

From the repository root:

```sh
cargo check --offline -p holo-compiler
cargo test --offline -p holo-compiler --lib
cargo run --offline -p holo-compiler --example region -- build/examples/region
```

Run the complete workspace checks documented in the [root guide](../README.md)
when changes also affect CLI behavior, shared tests, or runtime files. Native API
examples require matched LLVM tools and an OS linker. No legacy code or generated
artifacts are an implicit build input.

The workspace's [Cargo configuration](../.cargo/config.toml) places Rust build
output in `build/cargo/`. Example output belongs in `build/examples/`. The
[implementation plan](../docs/implementation-plan.md) records historical
validation and measurement results.
