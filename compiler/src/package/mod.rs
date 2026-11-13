//! Experimental local package inputs for the checked source frontend. The
//! loader resolves and snapshots declared filesystem dependencies before any
//! source lowering. Package names, paths, versions, and checksums live entirely
//! in compiler memory and never enter generated storage or native calls.

mod lock;
mod manifest;
mod snapshot;
#[cfg(test)]
mod tests;

use self::lock::{Lock, LockedPackage};
use semver::{Version, VersionReq};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// One resolved local package graph and its exact source snapshots.
pub struct PackageGraph {
    /// Name of the package selected by the explicit manifest path.
    pub(crate) root: String,
    /// All selected packages, with at most one provider per name.
    pub(crate) packages: BTreeMap<String, Package>,
    /// Lock built from the same provider data and source bytes.
    lock: Lock,
    /// Canonical sibling path of the root manifest.
    lock_path: PathBuf,
}

/// Source inputs and permitted dependency names for one selected package.
pub(crate) struct Package {
    /// Optional entry file, relative to the manifest's source root.
    pub(crate) entry: Option<PathBuf>,
    /// Direct dependencies available to importers owned by this package.
    pub(crate) dependencies: BTreeSet<String>,
    /// Exact Holo source snapshots indexed by source-relative paths.
    pub(crate) sources: BTreeMap<PathBuf, PackageSource>,
}

/// An exact source snapshot and the original path for diagnostics.
pub(crate) struct PackageSource {
    /// Absolute path to the original file.
    pub(crate) path: PathBuf,
    /// UTF-8 source text read once for this compiler invocation.
    pub(crate) text: String,
}

/// Provider data retained while traversing the graph and constructing its lock.
struct SelectedPackage {
    /// Canonical manifest location, used for collision and provenance checks.
    manifest: PathBuf,
    /// Parsed version, checked against every incoming dependency requirement.
    version: Version,
    /// Exact source digest for a dependency. Root source is outside the lock.
    sha256: Option<String>,
    /// Package data transferred into the compiler's public graph.
    input: Package,
    /// Whether this provider is on the active dependency path.
    active: bool,
}

/// One requested package, either the root or an explicitly named dependency.
struct Request {
    /// Candidate manifest path before canonical provider lookup.
    manifest: PathBuf,
    /// Name and version requirement of the requesting dependency entry.
    expected: Option<Expected>,
}

/// Constraints that must hold for a selected dependency provider.
struct Expected {
    /// Package name expected at the explicitly selected directory.
    name: String,
    /// Optional version range the package must satisfy.
    version: Option<VersionReq>,
}

/// Explicit depth-first traversal steps. Manifest depth need not consume the
/// compiler's call stack, and a shared dependency is read only once.
enum Visit {
    /// Resolve this provider, then visit its declared dependencies.
    Enter(Request),
    /// Mark the current provider complete after visiting all its children.
    Leave,
}

impl PackageGraph {
    /// Report whether the selected root manifest declares an application entry.
    /// The resolver guarantees the root is present. No source bytes are read or
    /// copied when the CLI chooses its check mode.
    pub fn has_entry(&self) -> bool {
        self.packages[&self.root].entry.is_some()
    }

    /// Explicitly replace the root's lock with this snapshot's selection.
    /// Compilation and inspection never call this mutation. I/O or validation
    /// errors leave the previously installed lock in place.
    pub fn write_lock(&self) -> Result<(), String> {
        self.lock.write(&self.lock_path)
    }
}

/// Resolve the explicit local manifest and its transitive dependencies. When
/// `locked`, require the existing root lock to match all selected sources.
/// Returns the same owned source bytes used in the checksum comparison. No
/// package is fetched, no file is written, and native tools are not invoked.
pub fn load(manifest_path: &Path, locked: bool) -> Result<PackageGraph, String> {
    let root_manifest = snapshot::canonical_manifest(manifest_path)?;
    let directory = root_manifest
        .parent()
        .expect("canonical manifest has a directory");
    let lock_path = directory.join("holo.lock");
    // Fail before reading source or dependencies if a required lock cannot be
    // decoded. Unlocked checking ignores any old lock and never rewrites it.
    let previous = locked.then(|| Lock::read(&lock_path)).transpose()?;
    let mut root = None;
    let mut root_dependencies = BTreeMap::new();
    let mut selected: BTreeMap<String, SelectedPackage> = BTreeMap::new();
    let mut providers = BTreeMap::<PathBuf, String>::new();
    let mut active = Vec::<String>::new();
    let mut pending = vec![Visit::Enter(Request {
        manifest: root_manifest.clone(),
        expected: None,
    })];

    // Each provider is expanded once. Its leave marker follows its children,
    // so encountering an active provider identifies a dependency cycle.
    while let Some(visit) = pending.pop() {
        let Visit::Enter(request) = visit else {
            let name = active.pop().expect("leave follows an active provider");
            selected
                .get_mut(&name)
                .expect("active provider is selected")
                .active = false;
            continue;
        };
        // The explicit root was already canonicalized to find the lock. Each
        // dependency enters with its manifest-relative candidate path.
        let path = if request.expected.is_none() {
            request.manifest
        } else {
            snapshot::canonical_manifest(&request.manifest)?
        };
        // Reuse the canonical provider only after checking this new incoming
        // requirement. An active provider names an edge back into the path.
        if let Some(name) = providers.get(&path) {
            let package = &selected[name];
            check_request(request.expected.as_ref(), name, &package.version, &path)?;
            if package.active {
                let start = active
                    .iter()
                    .position(|entry| entry == name)
                    .expect("active provider is on the path");
                let cycle = active[start..]
                    .iter()
                    .map(String::as_str)
                    .chain(std::iter::once(name.as_str()))
                    .collect::<Vec<_>>()
                    .join(" -> ");
                return Err(format!("package dependency cycle: {cycle}"));
            }
            continue;
        }

        let (manifest, raw_manifest) = manifest::read(&path)?;
        let name = &manifest.package.name;
        let version = manifest::version(&manifest.package, &path)?;
        check_request(request.expected.as_ref(), name, &version, &path)?;
        // A second directory for the same public name would make type and
        // function identity ambiguous, even if its version happens to match.
        if let Some(previous) = selected.get(name) {
            return Err(format!(
                "multiple providers for package `{name}`: {} and {}",
                previous.manifest.display(),
                path.display()
            ));
        }
        let (entry, sources) = snapshot::read_sources(&path, &manifest.package)?;
        let dependencies = manifest.dependencies.keys().cloned().collect();
        let digest = request
            .expected
            .is_some()
            .then(|| snapshot::source_digest(&raw_manifest, &sources))
            .transpose()?;
        active.push(name.clone());
        pending.push(Visit::Leave);
        // Reverse the push order so DFS checks requests in lexical name order.
        for (dependency, requested) in manifest.dependencies.iter().rev() {
            let relative = manifest::relative_path(&requested.path, true).map_err(|error| {
                format!(
                    "{}: dependency `{dependency}` has invalid path: {error}",
                    path.display()
                )
            })?;
            let manifest_path = path
                .parent()
                .expect("canonical manifest has a directory")
                .join(relative)
                .join("holo.toml");
            let version = manifest::requirement(dependency, requested, &path)?;
            pending.push(Visit::Enter(Request {
                manifest: manifest_path,
                expected: Some(Expected {
                    name: dependency.clone(),
                    version,
                }),
            }));
        }
        // Only the explicit root contributes direct requests to the lock. Each
        // dependency's own requests are already covered by its raw manifest.
        if request.expected.is_none() {
            root = Some(name.clone());
            root_dependencies = manifest.dependencies;
        }
        providers.insert(path.clone(), name.clone());
        selected.insert(
            name.clone(),
            SelectedPackage {
                manifest: path,
                version,
                sha256: digest,
                input: Package {
                    entry,
                    dependencies,
                    sources,
                },
                active: true,
            },
        );
    }

    let root = root.expect("the root request is resolved before dependencies");
    let mut packages = BTreeMap::new();
    let mut selections = Vec::new();
    // Moving the selected sources into the public graph preserves the byte
    // snapshots whose digests are compared. Package order stays deterministic.
    for (name, package) in selected {
        if let Some(sha256) = package.sha256 {
            selections.push(LockedPackage {
                name: name.clone(),
                version: package.version.to_string(),
                source: snapshot::relative_to(directory, &package.manifest)?,
                sha256,
            });
        }
        packages.insert(name, package.input);
    }
    let lock = Lock::new(root.clone(), root_dependencies, selections);
    // Compare all selections before the frontend can access any checked input.
    if let Some(previous) = previous {
        previous.verify(&lock, &lock_path)?;
    }
    Ok(PackageGraph {
        root,
        packages,
        lock,
        lock_path,
    })
}

/// Require one explicit dependency's name and version to agree with the
/// selected provider. Root packages have no incoming dependency requirement.
fn check_request(
    expected: Option<&Expected>,
    actual: &str,
    version: &Version,
    path: &Path,
) -> Result<(), String> {
    // The root is selected by its explicit manifest and supplies its own name.
    let Some(expected) = expected else {
        return Ok(());
    };
    // Accept one unambiguous provider for the declared dependency name.
    if actual != expected.name {
        return Err(format!(
            "dependency `{}` resolves to package `{actual}` at {}",
            expected.name,
            path.display()
        ));
    }
    // When present, every SemVer requirement must admit this same provider.
    if let Some(requirement) = &expected.version
        && !requirement.matches(version)
    {
        return Err(format!(
            "package `{actual}` version {version} does not satisfy version requirement `{requirement}` at {}",
            path.display()
        ));
    }
    Ok(())
}
