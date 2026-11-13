# Terminal styles

`aurora.hss` is the embedded default palette for `holo highlight`. The CLI's
`style.rs` compiles it once into a fixed table of ANSI prefixes. This affects host
source presentation only; the compiler returns source ranges and categories.

Override individual properties with `holo highlight file.holo --style path.hss`.
Rules cascade in written order over this palette. Each rule has one selector;
comments use `/* ... */`. Supported properties are `color: #RRGGBB | default`,
`font-weight: bold | normal`, `font-style: italic | normal`,
`text-decoration: underline | none`, and `dim: true | false`. Unknown selectors,
properties, malformed colours, and unterminated blocks fail before output.

Selectors are `.plain`, `.identifier`, `.keyword`, `.type`, `.function`,
`.parameter`, `.property`, `.number`, `.text`, `.comment`, `.operator`, `.punctuation`,
`.invalid`, `.builtin`, `.contract`, `.header`, `.gutter`, and `.border`.
Token roles are lexical approximations, not resolved semantic facts. VS Code's
grammar uses editor themes; this terminal palette does not override those themes.

Rendering borrows precompiled prefixes and writes through a buffered stream. RGB,
italic, dim and underline appearance depends on terminal support and background;
the default palette targets dark backgrounds. `--color=never` emits no ANSI.
Validate palette parsing and rendering through `cargo test --offline -p holo
--test source_tools` from the root. No stylesheet generator is required.
