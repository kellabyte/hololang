# Checked IR and lifetime tests

These suites exercise the public checker with structured IR and selected parsed
source. Positive and negative fixtures establish the static boundary before
lowering. They require no LLVM.

Run from the repository root.

```sh
cargo test --offline --test check --test check_locations --test byte_input_check --test owner_check
```

| Suite | Coverage |
| --- | --- |
| [`check.rs`](check.rs) | Tests definition, lifetime, permission, and initialization rejection. |
| [`check_locations.rs`](check_locations.rs) | Tests stable preorder diagnostic locations without IR spans. |
| [`byte_input_check.rs`](byte_input_check.rs) | Checks typed producer arguments, input expression contexts, live byte-field projections, same-root aliases/nested forwarding, and fresh scalar `Apply` frames independently of the source frontend. |
| [`owner_check.rs`](owner_check.rs) | Checks root-specific reset permission through independent owners, captured cursors, nested branches/loops, and lexical name shadowing. It also checks that explicit reset retains tail ownership while lexical batch completion permits same-region reuse. Parsed-source rejection checks include exact reset spans. |
