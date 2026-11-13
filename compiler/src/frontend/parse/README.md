# Experimental grammar parser

The parser turns lexer tokens into the frontend's spanned AST. It preserves syntax
and evaluation order without claiming that a declaration is semantically valid.
Its parent [parse.rs](../parse.rs) owns token lookahead, separators, diagnostics,
and recursion bounds. The interface is `parse(&str) -> Result<ast::Module,
Diagnostic>`; it depends only on the frontend AST, lexer, and source ranges.

- `declarations.rs` owns structs and the preferred `func` / `with` / `arg` /
  `returns` header form. A rooted entry begins with `using holo.<leaf>` directives.
  A rooted library can declare `namespace holo.<leaf>` before its functions.
  Package source uses names such as `holo.net.tcp` and allows `using`
  directives after a library's namespace header. The rooted profile keeps
  `holo.<leaf>`. At a file declaration boundary, the
  retired `import` spelling produces a migration diagnostic. Both library
  modes recognize `public func`. Package libraries
  also accept `struct`, `public struct`, `error`, and `public error`. A case
  declaration can carry one named field, as in `Truncated(required u64)`.
  An error family can include a child via `from Family as Alias` or
  `from qualifier.Family as Alias` in package source.
  The parser collects public names by declaration kind. The resolver checks
  source identity, visibility, and the supported field types.
- `expressions.rs` owns calls, projections, indexing, record literals, and operator
  precedence. It also parses `return ok value`, `return error Family.Case`,
  `return error Family.Truncated(required)`, and the `on … then`
  continuations of return or binding operands. A named case arm can bind a
  payload with `on Truncated(required)` or discard it with `on Truncated(_)`.
  Child cases use dotted paths such as `on Session.PermissionDenied`.
  Error case paths and arm bindings retain their spans for resolution.
  Checked `u64` arithmetic uses the parsed operator precedence.
- `return ok Record:` parses a fresh destination with equally indented
  `field = expression` lines. A field can start with contextual `try`. Append
  handling adds `on Rejected Case(value) then expression`, including qualified
  case paths. An ordinary case named `Rejected` keeps its existing arm syntax.
- A binding operand can start with contextual `try` followed by a direct call.
  A call written `try(...)` keeps the ordinary call grammar. The semantic
  checks require an immutable binding and an error family eligible for
  propagation into the enclosing result.
- `statements.rs` owns lexical blocks, binding/assignment, branches, loops,
  hosted refill and returns. Refill parses `refill field, limit as view { ... }
  else status { ... }` with separate outcome blocks. The statement
  `core.construct(region, producer(...))` uses the same `on` headers as
  result expressions, with a brace-delimited block after each `then`. The
  one-operand form names only the producer. A
  `with core.region(bytes) { ... } else { ... }` statement preserves its
  budget and both owner outcomes in the AST. The exact introducer is
  contextual, so `with(...)` retains its ordinary call grammar.
  `with core.tcp_stream() as stream { ... }` binds one host-admitted stream
  in its body. A refill can name that stream with `refill field, limit from
  stream as view { ... } else status { ... }`. Both forms preserve the
  source boundary for later ownership checking.
- `naming.rs` checks declarations and bindings at their original identifier
  spans when the frontend requests naming diagnostics. It checks namespace
  headers, functions, arguments, types, error cases and child aliases, fields,
  and lexical bindings. References, including `using` paths, remain unchanged.
  A mechanically clear correction can appear in the diagnostic. Leading
  underscores are skipped while prefix conventions remain open, including
  the `_` outcome discard. Namespace components still follow the existing
  lowercase source path grammar.

Newlines or semicolons separate complete statements. Braces delimit executable
blocks. An `on` list begins on following physical lines, with each arm indented
by the same number of leading whitespace bytes and more deeply than its subject
line. Delimited expression lists can continue across newlines. This is an
explicitly experimental grammar and leaves the remaining whitespace, type,
error, and collection API decisions open.

Formatting can parse these file shapes independently. Import resolution and
visibility checks belong to [modules.rs](../modules.rs) and require a library root
only when compiling project source. Formatting and syntax-only parsing disable
naming collection and preserve identifier spellings.

The parser admits named types, the narrow `Result<T, E>` return syntax, and
explicit `readonly`/`readwrite` plus `copy`/`reference` argument modes. It rejects
reordered or duplicate contract clauses and other parameterized types. Semantic
checking admits scalar success and eligible aggregate destinations for append
and scoped construction. Record literals
immediately before control flow braces require parentheses because the brace
belongs to the body. This
avoids interpreting `for readonly entry in items { ... }` as an `items` record.

Compiler recursion and AST depth are capped at 128 as an experimental parser
resource limit, separate from any eventual language recursion policy. Token and
AST storage grows with admitted source; suffix construction moves existing nodes
rather than cloning the growing tree. AST depth checks inspect expression
subtrees; this initial implementation prioritizes bounded diagnostic behavior
over incremental parsing or measured frontend throughput.

Run the parser integration tests with `cargo test --offline --test parser`.
No LLVM installation is needed for syntax tests.
