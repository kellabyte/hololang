# Holo command-line tool

This package builds the `holo` executable. [src/main.rs](src/main.rs) owns command
arguments, source diagnostics, output ownership, and process exit handling. The
[compiler library](../compiler/README.md) supplies parsing, checking, layouts, LLVM
emission, and native tool driving through one local dependency.

The CLI exposes `check`, `inspect-layout`, `inspect-copies`, `emit-llvm`, `build`,
and `run` for the experimental Holo source subset. Checking and inspection work
without LLVM. Native commands require matched `opt`/`clang` tools and an OS linker.
Compiler artifact outputs must be new paths. Source contracts and API limits are
in the [specification](../docs/spec.md#experimental-source-subset).

Existing scripts may still pass `--experimental`.

From the repository root:

```sh
cargo run --offline -- highlight tests/fixtures/region.holo --header -n --frame
cargo run --offline -- highlight tests/fixtures/region.holo --plain --color=always
cargo run --offline -- fmt tests/fixtures/region.holo
cargo run --offline -- fmt tests/fixtures/region.holo --check
```

## Highlight and format source

`highlight` and `fmt` are LLVM-free source tools that share the compiler scanner.
Highlighting tolerates incomplete source. Formatting requires valid experimental
syntax and preserves its structure.

Both accept `-` or `--stdin`. Default highlight output shows a filename and line
numbers on terminals; pipes receive byte-exact source without ANSI or chrome.
`--header`/`--no-header`, `-n`/`--line-numbers`/`--no-line-numbers`, and `--frame`
control decoration. `--plain` suppresses decoration; it can retain ANSI colour.
`--color=auto|always|never` (or separate value) controls ANSI. Automatic colour
respects nonempty `NO_COLOR` and `TERM=dumb`; explicit `always` overrides them.
The [CSS-like stylesheet](styles/README.md) compiles once into a fixed style table;
`--style` cascades overrides over the bundled palette. Lexical categories remain
approximate, especially unresolved identifiers, incomplete code and invalid numbers.

Display mode escapes C0/C1 control scalars except tabs and real LF/CRLF boundaries,
including those in filenames, so source comments cannot inject terminal commands.
Plain redirected output preserves original bytes. Coloured/plain output preserves
the final source newline; a decorated view may add a newline before its footer or
the next prompt. No width reflow, automatic pager, semantic tokens or terminal
capability negotiation is implemented; the RGB default palette targets dark
backgrounds. Header/number/frame metadata is display output, never file content.

`fmt` defaults to plain formatted stdout, including for unsaved editor text.
`--check` produces no source output and exits 1 if formatting would change it.
`--write` opts into regular-file replacement using an exclusive same-directory
temporary, file sync and atomic rename with permission preservation. A detected
intervening edit aborts replacement; this is not a concurrent-writer lock or a
directory-fsync durability guarantee. Symlink replacement is unsupported, and
replacement does not retain inode identity, hard links or extended metadata.
Syntax errors, unsupported format layouts and invalid options produce no partial
formatted source or replacement. No identifiers are renamed; width reflow and
general language syntax remain unsupported.

The adapters reserve one input buffer and buffer highlight output in 64 KiB.
Scanning and highlighting borrow source spans; no per-token style parsing or
output tree is required. Formatting deliberately allocates and reparses to check
preservation. These are implementation costs, not native Holo performance claims.
Run `cargo test --offline -p holo --test source_tools --test highlight --test format`.

## Compiler commands

```sh
cargo run --offline -p holo -- check tests/fixtures/region.holo
cargo run --offline -p holo -- inspect-copies tests/fixtures/byte_input.holo
cargo run --offline -p holo -- run tests/fixtures/byte_input.holo --release
cargo test --offline -p holo --test cli
```

To compile an experimental local `using` directive, pass `--library-root <dir>`.
The root is the directory containing the `holo`
leaf modules. In this repository, `--library-root lib/holo` resolves
`using holo.bytes` to `lib/holo/bytes.holo`. The file declares
`namespace holo.bytes`, which the compiler checks against the requested namespace. The
option applies to check, inspection, emission, build, and run. The directory
and its parent paths are
trusted build inputs and must stay stable during compilation. The CLI reads
UTF-8 regular files from that directory and rejects symbolic leaf files.
`fmt` can format entry and library syntax without a root because it reads no
dependencies. [Experimental package design](../docs/packages.md)

A hosted input example takes its descriptor arguments at native launch.
Use `build` and launch the resulting executable with `file PATH` or
`stream-fd FD`. The `run` command launches with no program arguments.
[Hosted telemetry worker guide](../tests/fixtures/telemetry_worker/README.md)

## Naming diagnostics

Compilation reports declaration naming advisories on stderr after source
checking. Add `--strict-names` to fail on those advisories. In package mode the
policy covers the selected package's checked source files. With a standalone
file and `--library-root`, it covers the entry file.

```sh
cargo run --offline -p holo -- check --strict-names \
  --package lib/holo/holo.toml
```

See the [identifier conventions](../docs/spec.md#identifier-naming-conventions).

## Local packages

Use `--package <holo.toml>` to select a local package. When the manifest has an
entry, `check` checks the entry and the namespaces its imports reach. When the
entry is omitted, `check` checks every namespace in the root library and
follows the imports of all its source files. Inspection, LLVM emission, build,
and run require an entry. With `--locked`, the command requires the resolved
dependencies to match the existing sibling `holo.lock`. The separate `lock`
command resolves local dependencies and writes that file explicitly. Both lock
and check need no native toolchain.

```sh
cargo run --offline -p holo -- check --package lib/holo/holo.toml
cargo run --offline -p holo -- check --locked \
  --package tests/fixtures/telemetry_worker/holo.toml
cargo run --offline -p holo -- lock \
  --package tests/fixtures/telemetry_worker/holo.toml
```

Package mode gets its source root from the manifest. Choose either package
mode or a positional source path with the optional legacy leaf root.
[Package design](../docs/packages.md#local-manifests-and-locked-builds)

`inspect-copies` lists the byte initializer decisions for the native host. It uses
the same checked selector as LLVM emission and reports which shape can use a
guarded inline path, when it falls back, and why other initializers use the
ordinary loop. The report describes Holo's lowering before LLVM optimization.

Root [integration tests](../tests/README.md) are explicitly registered in this
package's Cargo manifest. This supplies the real `holo` binary to CLI tests while
keeping shared compiler/source/native tests together in the visible root test
suite. It does not duplicate tests or introduce another implementation.
