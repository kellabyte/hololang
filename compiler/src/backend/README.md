# Native backends

[llvm](llvm/README.md) is the implemented native backend. Its Rust modules accept
checked compiler input and emit LLVM through the existing layout plan. Tool
selection, verification, optimization, linking, and native process orchestration
belong to [driver.rs](../driver.rs).

The public Rust API remains `holo::llvm`; the directory name makes ownership
visible without changing that API or creating a second checked representation.
[Runtime support](../../../runtime/README.md) is native target code consumed by
this backend/driver, separate from the Rust emission implementation.

Only the real LLVM implementation is present. Add another backend when it has a
reviewed purpose and actual code rather than reserving empty directories. Changes
must retain checked semantics before pursuing target-specific optimizations.

Run `cargo test --offline --test native --test layout_native` from the repository
root. Native checks require matched LLVM tools and an OS linker.
