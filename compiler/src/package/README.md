# Local package inputs

This component resolves the experimental package graph before source checking.
The CLI passes an explicit manifest to `load`. The source frontend borrows the
selected UTF-8 snapshots and applies its ordinary namespace, effect, and
storage checks. Package metadata has no native representation.

| File | Responsibility |
| --- | --- |
| `mod.rs` | Own the graph, enforce one provider per package name, and check every incoming version requirement. An iterative traversal rejects cycles without using the Rust call stack for package depth. |
| `manifest.rs` | Decode `holo.toml`, validate package names and paths, and parse semantic versions. |
| `snapshot.rs` | Discover regular `.holo` sources, preserve the exact bytes consumed by the frontend, and compute portable dependency fingerprints. |
| `lock.rs` | Encode and verify `holo.lock` and publish it through an explicit atomic write. |
| `tests.rs` | Exercise the loader with isolated local packages and filesystem inputs. |

Each package declares a name, semantic version, and source directory. A root
application can select an entry file relative to that source directory. Direct
dependencies identify a local directory and can constrain its version. The
dependency key must match the provider's declared name. Providers are resolved
from explicit local paths, and all incoming requirements must accept the same
provider. The selected manifest directories are trusted build inputs. Source
directory components and entries reject symlinks. Source paths stay within the
package, and entry paths stay within its source directory.

A root manifest can omit the entry. Its source snapshot is then available to
the library check, which starts from all root library files and follows their
`using` directives. A root with an entry starts from the entry's namespace
dependencies. Package loading and lock validation precede both checking modes. The
frontend owns namespace mapping, visibility, and function checking. Neither
mode needs LLVM.

The lock records the root name and direct requests, plus each selected
dependency's name, version, canonical manifest path relative to the root
directory, and SHA-256 fingerprint. The digest frames the exact manifest text
and each source-relative path and `.holo` file in sorted order. This includes
unimported source files. The compiler uses the same bytes it hashed. Root
source and version remain editable in a locked build. The lock detects input
changes against a trusted prior selection. Workspace relocation preserves the
lock when the relative layout and pinned bytes agree.

Normal checking and builds read the graph without mutating the lock. Locked
builds compare the complete selection before lowering. An explicit `holo lock`
command creates an exclusive sibling temporary, syncs the complete contents,
then renames it into place. An existing symbolic or nonregular lock path is
rejected. The loader does not download sources or query a package registry.

The compiler uses `toml` with Serde for metadata, `semver` for version matching,
and `sha2` for checksums. Run the package component tests from the repository
root with `cargo test --offline -p holo-compiler --lib package::tests`.
Run the CLI checks with `cargo test --offline --test packages`.
