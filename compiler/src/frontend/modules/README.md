# Package namespace binding

`packaged.rs` binds namespaces in the local package experiment.
The package loader owns the immutable manifest graph and source snapshots. The
frontend borrows those bytes, assigns nonoverlapping diagnostic spans, and
binds function calls, record schemas, and error family names before ordinary
lowering.

A namespace comes from immediate Holo sources in its directory, in
sorted path order. A file directly under the package source root can
instead use its stem as a one-component namespace. For example,
`net.holo` can supply `holo.net` alongside `net/tcp/*.holo` for
`holo.net.tcp`. Each contributor declares the qualified namespace of
its own directory. A parent directory's contributors and its child
namespaces are resolved independently. Plain functions are visible across
contributors of one namespace. Public functions are available through
file-local `using` directives that name the same package or a direct
manifest dependency. The final path component provides the local
qualifier. Conflicting qualifiers, duplicate declarations, namespace
cycles, and two source shapes for the same namespace fail before lowering.

The package record experiment keeps separate type and function lookups. The
binder indexes every contributing file's structs before checking constructor
results and resolving inline field types. Plain structs are namespace-local.
A `public struct` with `u64`, `Bytes`, or same-namespace inline record fields
can be selected as `qualifier.Type` by the application's `core.batch`
declaration. Each inline child of a public record must also be public.
A public constructor can return only a public struct from its own namespace.
Qualified type names in other positions and record-reference parameters remain
outside this interface.

The same type namespace indexes `error` families across contributors.
`public error` makes the family available to imported `on` arms. A public
`Result<u64, E>` signature must expose a public local family. Qualified case
spelling resolves to the declared family before result checking, including
arms that bind a case's `u64` payload. A child declared with
`from qualifier.Family as Alias` uses the declaring file's imports, and a
bare child name uses the shared namespace's declarations. A public error
family can expose only public child families. A pattern can name a child
path relative to its subject, such as `Alias.Case`, or qualify it with the
parent family. A prefix that resolves to an imported public family is explicit.
Start with the subject's family to disambiguate a child path with that prefix.
A namespace-local
`Result<Record, E>` destination constructor uses the record and error maps
together. Both declarations must be public when exported, including unused
constructors. The signature is rewritten to both nominal identities before
checking the same destination append path as a standalone source.
Entry blocks using `with core.region` resolve the capacity expression,
success body, and acquisition recovery with the entry's imports. A qualified
producer and its qualified rejection case follow the existing scoped
construction rules. The binder supplies no name for the private backing.

The full package, namespace, and record name supplies the schema's nominal
identity after selecting one provider per package. A namespace's literal,
inline field type, and constructor result use that same identity as the
caller's batch declaration. The existing checker, layout planner, and
destination-construction path handle the record, including storage and field
access. Namespace binding allocates compiler tables and introduces no runtime
type descriptor.

An application starts at its entry's `using` directives. Checking a package
without an entry starts at every namespace selected by its own source
paths. The checker also follows namespace dependencies reachable from
those contributors. An empty root has zero namespaces to check. No
executable entry is created for library checking.

The iterative import traversal retains one node per reachable namespace and
source ASTs for its files. It does no runtime dependency lookup or source copying
into the generated program. Package diagnostics use the same source map and
checked boundary as the [legacy leaf loader](../modules.rs).

Run `cargo test --offline --test modules` from the repository root to check
the explicit leaf loader and `cargo test --offline --test packages` to check
packaged applications and libraries.
