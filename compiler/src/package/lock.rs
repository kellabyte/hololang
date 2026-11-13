//! Serialize and verify the local dependency lock. A lock is a statement
//! about compiler inputs and never runs a resolver or chooses new sources.
//! Explicit writes install a complete same-directory file through rename.

use super::manifest::Dependency;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Current on-disk lock schema and checksum framing version.
const FORMAT: u32 = 1;

/// Suffix source for temporary lock paths created by this compiler process.
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Canonical selection of the declared local dependency graph.
#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Lock {
    /// Format checked before comparing source selections.
    format: u32,
    /// Root package name, whose own editable source is outside the lock.
    root: String,
    /// Root request keys, paths, and version requirements as written.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    root_dependencies: BTreeMap<String, Dependency>,
    /// Transitive selected packages, ordered by their unique names.
    #[serde(default, rename = "package", skip_serializing_if = "Vec::is_empty")]
    packages: Vec<LockedPackage>,
}

/// One exact local provider, including its version and full source digest.
#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct LockedPackage {
    /// Public package identity.
    pub name: String,
    /// Selected SemVer from the provider's manifest.
    pub version: String,
    /// Canonical provider manifest path relative to the root package directory.
    pub source: String,
    /// SHA-256 of its raw manifest and source snapshot.
    pub sha256: String,
}

impl Lock {
    /// Own the root's requests and ordered provider selection for this build.
    /// All values already describe the immutable source snapshots in the graph.
    pub(super) fn new(
        root: String,
        root_dependencies: BTreeMap<String, Dependency>,
        packages: Vec<LockedPackage>,
    ) -> Self {
        Self {
            format: FORMAT,
            root,
            root_dependencies,
            packages,
        }
    }

    /// Read and decode a regular lock before touching dependency sources. A
    /// missing or incompatible lock requires an explicit refresh.
    pub(super) fn read(path: &Path) -> Result<Self, String> {
        if !regular_destination(path)? {
            return Err(format!(
                "read package lock {}: missing lockfile; run holo lock",
                path.display()
            ));
        }
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("read package lock {}: {error}", path.display()))?;
        let lock: Self = toml::from_str(&raw)
            .map_err(|error| format!("invalid package lock {}: {error}", path.display()))?;
        // Unknown format numbers cannot be used as provenance for this build.
        if lock.format != FORMAT {
            return Err(format!(
                "unsupported package lock format {} in {}",
                lock.format,
                path.display()
            ));
        }
        Ok(lock)
    }

    /// Require that the verified lock names the sources this build selected.
    /// Describe the first differing input in canonical package order.
    pub(super) fn verify(&self, actual: &Self, path: &Path) -> Result<(), String> {
        // Equal requests and selections are sufficient because each provider
        // entry includes both the manifest and every selected source byte.
        if self == actual {
            return Ok(());
        }
        // Separate root metadata changes from differences in the dependency
        // graph or a particular provider, so refreshes are reviewable.
        let detail = if self.root != actual.root {
            "root package name changed".into()
        } else if self.root_dependencies != actual.root_dependencies {
            "root dependency requests changed".into()
        } else if self.packages.len() != actual.packages.len() {
            "dependency graph changed".into()
        } else {
            self.packages
                .iter()
                .zip(&actual.packages)
                .find_map(|(expected, selected)| {
                    // Names identify graph nodes. A selected path, version, or
                    // digest change is meaningful even when the name agrees.
                    if expected.name != selected.name {
                        Some("dependency graph changed".into())
                    } else if expected.source != selected.source {
                        Some(format!(
                            "dependency `{}` selected a different source path",
                            selected.name
                        ))
                    } else if expected.version != selected.version {
                        Some(format!(
                            "dependency `{}` selected a different version",
                            selected.name
                        ))
                    } else if expected.sha256 != selected.sha256 {
                        Some(format!(
                            "dependency `{}` changed its manifest or source contents",
                            selected.name
                        ))
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "dependency selection changed".into())
        };
        Err(format!(
            "package lock is stale at {}: {detail}; run holo lock",
            path.display()
        ))
    }

    /// Install a complete lock at `path` through an exclusive temporary file
    /// in the same directory. Failures clean up the temporary and leave the
    /// previous lock readable. Concurrent explicit writers publish whole locks.
    pub(super) fn write(&self, path: &Path) -> Result<(), String> {
        regular_destination(path)?;
        let contents = toml::to_string_pretty(self)
            .map_err(|error| format!("serialize package lock {}: {error}", path.display()))?;
        let (mut temporary, mut file) = TemporaryLock::create(path)?;
        file.write_all(contents.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                format!(
                    "write temporary package lock {}: {error}",
                    temporary.path.display()
                )
            })?;
        drop(file);
        fs::rename(&temporary.path, path)
            .map_err(|error| format!("install package lock {}: {error}", path.display()))?;
        temporary.installed = true;
        Ok(())
    }
}

/// Distinguish a missing path from a regular lock and reject every other type.
/// In particular, an explicit update cannot follow a destination symlink.
fn regular_destination(path: &Path) -> Result<bool, String> {
    // Only absence can be treated as an unwritten lock. Permission and type
    // errors must remain visible rather than silently changing the build.
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(format!(
            "expected a regular package lock at {}",
            path.display()
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("read package lock {}: {error}", path.display())),
    }
}

/// Temporary path owned by one explicit writer until installation or failure.
struct TemporaryLock {
    /// Same-directory path reserved with `create_new`.
    path: PathBuf,
    /// Successful publication transfers ownership away from the temporary name.
    installed: bool,
}

impl TemporaryLock {
    /// Reserve a same-directory temporary without opening another writer's
    /// file. A reused process ID can leave a stale path, which is skipped.
    fn create(lock_path: &Path) -> Result<(Self, File), String> {
        let directory = lock_path
            .parent()
            .expect("canonical manifest has a directory");
        // Try another unique suffix only after an exclusive-create collision.
        // Filesystem failures such as permission errors return immediately.
        loop {
            let suffix = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!(".holo.lock.{}.{}.tmp", std::process::id(), suffix));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok((
                        Self {
                            path,
                            installed: false,
                        },
                        file,
                    ));
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "create temporary package lock {}: {error}",
                        path.display()
                    ));
                }
            }
        }
    }
}

impl Drop for TemporaryLock {
    /// Remove an uninstalled temporary file on failure. Successful rename
    /// releases the old name, which may now be claimed by another writer.
    fn drop(&mut self) {
        // Only a writer that still owns the temporary name may remove it.
        if !self.installed {
            let _ = fs::remove_file(&self.path);
        }
    }
}
