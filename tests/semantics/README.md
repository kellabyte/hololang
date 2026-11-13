# Reference interpreter and native comparisons

These suites check expected Holo behavior against hand-derived results and then
compare compiled programs with the independent interpreter. The native comparison
requires LLVM and an OS linker. The explicit layouts are `Wide`, `Derived64`,
and `Compact32`.

Run from the repository root.

```sh
cargo test --offline --test model --test model_native
```

## Independent expectations

The independent [reference interpreter](../support/model/README.md) evaluates checked
Holo IR using safe Rust values. The seeded generator and explicit boundary
fixtures supply the same checked input to the interpreter and native compiler.

[`reference_interpreter.rs`](reference_interpreter.rs) checks the interpreter
against hand-derived boundary results before it is used as a native oracle. It
runs without LLVM.

[`native_vs_reference.rs`](native_vs_reference.rs) compares the interpreter and
native executable on each generated and boundary fixture at O0 and O3 across all
three layouts. The interpreter derives header eligibility and widths
independently of compiler layout plans. Address stability is observed within a
native run.

Cargo keeps the existing target names.

| Cargo target | Suite |
| --- | --- |
| `model` | `reference_interpreter.rs` |
| `model_native` | `native_vs_reference.rs` |
