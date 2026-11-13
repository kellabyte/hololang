//! Snapshot the source tree used by the package resolver and lockfile. Each
//! `.holo` file is read once. The source map owns its exact text, so namespace
//! resolution cannot reopen a changed source after checking a dependency digest.

use super::{PackageSource, manifest};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Resolve a supplied regular manifest to a canonical location. This location
/// distinguishes providers and makes `./` or `../` aliases of one provider
/// agree. The symlink rule prevents a leaf manifest from changing the root.
pub(super) fn canonical_manifest(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("read package manifest {}: {error}", path.display()))?;
    // Never open symbolic, directory, or device manifests.
    if !metadata.is_file() {
        return Err(format!(
            "expected a regular package manifest at {}",
            path.display()
        ));
    }
    path.canonicalize()
        .map_err(|error| format!("resolve package manifest {}: {error}", path.display()))
}

/// Read the package's regular UTF-8 Holo files, sorted by their normalized
/// source-relative paths. Reads and tree walks allocate in the compiler only.
/// Source and entry locations may not escape their declaring root.
pub(super) fn read_sources(
    manifest_path: &Path,
    definition: &manifest::Definition,
) -> Result<(Option<PathBuf>, BTreeMap<PathBuf, PackageSource>), String> {
    let relative_source = manifest::relative_path(&definition.source, false)
        .map_err(|error| format!("{}: invalid source root: {error}", manifest_path.display()))?;
    let entry = definition
        .entry
        .as_deref()
        .map(|value| {
            manifest::relative_path(value, false)
                .map_err(|error| format!("{}: invalid entry: {error}", manifest_path.display()))
        })
        .transpose()?;
    let mut source = manifest_path
        .parent()
        .expect("canonical manifest has a directory")
        .to_owned();
    // Check each path component before descending. A symbolic ancestor within
    // the declared source path must not redirect the source tree elsewhere.
    for component in relative_source.components() {
        source.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&source).map_err(|error| {
            format!(
                "read package source directory {}: {error}",
                source.display()
            )
        })?;
        if !metadata.is_dir() {
            return Err(format!(
                "expected a non-symbolic package source directory at {}",
                source.display()
            ));
        }
    }

    let mut sources = BTreeMap::new();
    let mut directories = vec![(PathBuf::new(), source)];
    // An explicit directory stack handles deeply nested source trees without
    // consuming call-stack space. Every real directory enters it once.
    while let Some((relative_directory, directory)) = directories.pop() {
        let mut entries = fs::read_dir(&directory)
            .and_then(|entries| entries.collect::<Result<Vec<_>, _>>())
            .map_err(|error| {
                format!(
                    "read package source directory {}: {error}",
                    directory.display()
                )
            })?;
        // Discovery errors are deterministic within each directory, independent
        // of filesystem enumeration order. The map orders the final snapshot.
        entries.sort_unstable_by_key(fs::DirEntry::file_name);
        for item in entries {
            let path = item.path();
            let relative = relative_directory.join(item.file_name());
            let kind = item
                .file_type()
                .map_err(|error| format!("read package source type {}: {error}", path.display()))?;
            // Symlinks can escape the selected root or create directory loops.
            // A Holo source must be a regular file, and other ordinary data
            // files have no effect on this source-only compiler input.
            if kind.is_symlink() {
                return Err(format!("symbolic package source path {}", path.display()));
            } else if kind.is_dir() {
                directories.push((relative, path));
            } else if path
                .extension()
                .is_some_and(|extension| extension == "holo")
            {
                if !kind.is_file() {
                    return Err(format!(
                        "expected a regular package source file at {}",
                        path.display()
                    ));
                }
                portable_path(&relative)?;
                let text = fs::read_to_string(&path)
                    .map_err(|error| format!("read package source {}: {error}", path.display()))?;
                sources.insert(relative, PackageSource { path, text });
            }
        }
    }
    // An entry must select exactly one file in the same snapshot used by
    // dependency imports. Missing or non-Holo paths cannot become entry code.
    if let Some(entry) = &entry
        && !sources.contains_key(entry)
    {
        return Err(format!(
            "{}: package entry {} is not a Holo source file",
            manifest_path.display(),
            entry.display()
        ));
    }
    Ok((entry, sources))
}

/// Hash exact UTF-8 manifest and source bytes, including stable relative names.
/// Length framing avoids ambiguity between filenames and content. The digest
/// changes for an edit, addition, removal, or rename of any selected Holo file.
pub(super) fn source_digest(
    manifest: &str,
    sources: &BTreeMap<PathBuf, PackageSource>,
) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"holo-package-source-v1\0");
    frame(&mut digest, manifest.as_bytes());
    let mut files = sources
        .iter()
        .map(|(relative, source)| Ok((portable_path(relative)?, source.text.as_bytes())))
        .collect::<Result<Vec<_>, String>>()?;
    // Order portable names by UTF-8 bytes independently of the host's path
    // ordering or directory enumeration. The sort carries only names and
    // borrowed source slices, and SHA-256 streams the original bytes.
    files.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    for (name, bytes) in files {
        frame(&mut digest, name.as_bytes());
        frame(&mut digest, bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Add a fixed-width length and one byte sequence to the checksum without
/// copying the input or building an intermediate serialized snapshot.
fn frame(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

/// Write a relative source or provider path with portable slash separators.
/// Only UTF-8 component names are supported by the lock format.
pub(super) fn portable_path(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    // Convert only relative components. Separator-shaped bytes in a component
    // are rejected rather than given a different meaning on another host.
    for component in path.components() {
        match component {
            Component::Normal(name) => parts.push(
                name.to_str()
                    .filter(|name| !name.contains(['\\', ':']))
                    .ok_or_else(|| format!("non-portable package path {}", path.display()))?,
            ),
            Component::ParentDir => parts.push(".."),
            _ => {
                return Err(format!(
                    "expected a relative package path, got {}",
                    path.display()
                ));
            }
        }
    }
    Ok(parts.join("/"))
}

/// Find a relocatable path from the root manifest directory to a canonical
/// dependency manifest. An unrelated filesystem root cannot be expressed as
/// a relative lock path and must fail instead of persisting a host path.
pub(super) fn relative_to(base: &Path, manifest: &Path) -> Result<String, String> {
    let base: Vec<_> = base.components().collect();
    let target: Vec<_> = manifest.components().collect();
    let common = base
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    // Absolute paths on separate drives have no common canonical root.
    if common == 0 {
        return Err(format!(
            "dependency manifest {} has a different filesystem root",
            manifest.display()
        ));
    }
    let mut relative = PathBuf::new();
    // Each unmatched base component needs one parent step before the target.
    for _ in common..base.len() {
        relative.push("..");
    }
    // Append the target suffix unchanged, then write it in lock path syntax.
    for component in &target[common..] {
        relative.push(component.as_os_str());
    }
    portable_path(&relative)
}
