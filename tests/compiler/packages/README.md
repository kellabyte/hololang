# Compiler package and module tests

These suites check how the compiler resolves source modules, namespaces, and
locked packages through the real CLI. Selected cases require LLVM and an OS linker.

Run from the repository root.

```sh
cargo test --offline --test modules --test packages
```

## Local modules

[`modules.rs`](modules.rs) checks explicit local library resolution, namespace
declarations, `using` directives and the diagnostic for the retired spelling,
`public func` visibility, isolated private names, diagnostics, path restrictions, and
dependency-free formatting. It checks the real CLI at O0/O3 and compares full emitted
LLVM IR with equivalent single-file source across all layouts.
`cargo test --offline --test modules` requires LLVM for the native CLI case. The IR
comparison measures code equivalence.

## Packages and locked dependencies

- **Manifests and lockfiles.** [`packages.rs`](packages.rs) checks the manifest CLI with
  split namespaces, namespace-private helpers, file-local `using` directives, dependency
  versions and provider conflicts. It exercises relocated lockfiles and checks that
  dependency edits invalidate a lock while root source edits preserve it.

- **Native equivalence.** The native cases execute an actual borrowed byte decode at
  O0/O3 and compare emitted LLVM against the same bodies in one file across all layouts.

- **Entryless packages and naming.** Library-only checks need no LLVM, validate every
  owned namespace and its reachable dependency namespaces, and reject source layout,
  visibility, and cycle failures without an entry. Build, run, and LLVM emission require
  an entry. `shipped_standard_library_passes_strict_naming` runs the real CLI against
  the shipped library manifest with `--strict-names` and requires no LLVM. Run
  `cargo test --offline --test packages` with LLVM installed for the native and emission
  checks.

<details>
<summary>Package construction, error unions, and hosted decoding</summary>

- **Fixed records.** The fixed-record cases link a public schema and destination
  constructor from two dependency files, reject equal-layout nominal mismatches and
  private exposure, and preserve field access and batch lifetimes. The package and
  same-file control use the same LLVM in all layouts. Native O0/O3 and package ASan
  cases check rollback, retry, exact slot size, and stable retained references.

- **Aggregate results.** The package aggregate result cases cover a public
  `Result<Header, FrameError>` constructor and separate record and error declarations in
  a locked dependency. The consumer calls a public exact-result forwarder declared in
  another dependency file. The test checks same-file LLVM identity in every layout and
  native O0, O3, and ASan for qualified payload handling, retained storage, retry, and
  failure after a private field write. The selected arms observe the published count on
  success and the restored marker on failure. Entryless checks reject either private
  component in an unused export. Equal-layout records and equal-spelling error families
  are rejected at the consumer.

- **Error unions.** The locked dashboard demo covers three child families and a fallback
  at O0 and O3. The cross-package union case verifies relative and qualified handlers,
  file-local qualifiers, and rejection of public exposure of a private child.

- **Owned byte payloads.** The byte-bearing cases package the actual nested frame
  decoder, check LLVM identity in all layouts, and execute O0, O3, and ASan. They
  observe truncation and malformed input after a private parent write, capacity failure
  during payload reservation, a retained address, and successful exact-slot retry.
  Mutating the caller's wire buffer leaves the published payload intact. Entryless
  checks cover later inline type declarations and reject private exposure or names from
  another namespace.

- **Hosted refill.** The packaged hosted-refill case passes the fresh native prefix to
  the same nested decoder from a locked dependency. Real files and nonblocking stream
  pairs exercise a short read with a stale tail, a complete frame that exceeds remaining
  capacity after private field writes, and an exact-slot retry. It compares
  byte-identical package and flat LLVM and checks rollback, retained addresses, owned
  payloads, and EOF across all three layouts at O0, O3, and under ASan.

- **Hosted scoped results.** The locked hosted scoped-result case uses the same
  production decoder. The imported constructor and qualified rejection see only four
  fresh bytes after a six-byte request. The failure arm observes the restored marker,
  updates the original slot through its writable alias, and retries a one-byte preview
  at the same address. Package and flat LLVM match in all three layouts. File and
  nonblocking socket runs cover O0, O3, and ASan.

- **Staged decoding.** The staged hosted-package case requires the constructed-slot
  proof and its outlined bulk call in the locked package's Derived64 LLVM. The ordinary
  locked CLI emits the same LLVM as the candidate for this bounded site. File and
  nonblocking stream runs deliver the same wire with wide and two-byte caps. They check
  typed truncation, capacity rollback, exact-slot retry, EOF, and every retained payload
  byte after both input slots have been reused.

</details>
