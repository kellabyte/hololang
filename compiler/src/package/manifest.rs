//! Decode and validate the experimental package manifest. TOML and SemVer are
//! handled by their Rust libraries. Path syntax is checked before any source
//! walk, and all decoded values are compiler metadata.

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::{collections::BTreeMap, fs};

/// One package manifest, with dependency requests ordered by their names.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    /// Package identity, version, and source paths.
    pub package: Definition,
    /// Direct local dependency requests.
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
}

/// Source selection and version information from the `[package]` table.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Definition {
    /// Lowercase source namespace root and dependency identity.
    pub name: String,
    /// SemVer version supplied by the package author.
    pub version: String,
    /// Directory relative to the directory containing the manifest.
    pub source: String,
    /// Optional entry `.holo` path relative to `source`.
    pub entry: Option<String>,
}

/// One explicit local dependency. The lock records the root's requests as
/// written so a change in a requested path or requirement is detectable.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Dependency {
    /// Relative directory containing the dependency's `holo.toml`.
    pub path: String,
    /// Optional SemVer requirement, with Cargo-style requirement semantics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Read a regular manifest once and decode its strict schema. The raw text is
/// returned with the metadata so the digest and graph describe the same bytes.
pub(super) fn read(path: &Path) -> Result<(Manifest, String), String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("read package manifest {}: {error}", path.display()))?;
    let manifest: Manifest = toml::from_str(&contents)
        .map_err(|error| format!("invalid package manifest {}: {error}", path.display()))?;
    validate_name(&manifest.package.name)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    // Reject bad names even when a dependency is never imported. The complete
    // declared graph is part of a locked build.
    for name in manifest.dependencies.keys() {
        validate_name(name).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok((manifest, contents))
}

/// Parse a package's declared version. Errors name the owning manifest and
/// are reported before reading any of its source files.
pub(super) fn version(definition: &Definition, path: &Path) -> Result<Version, String> {
    Version::parse(&definition.version)
        .map_err(|error| format!("{}: invalid package version: {error}", path.display()))
}

/// Parse a dependency's optional version constraint, reporting its owning
/// manifest and request key if it is malformed.
pub(super) fn requirement(
    name: &str,
    request: &Dependency,
    manifest: &Path,
) -> Result<Option<VersionReq>, String> {
    request
        .version
        .as_deref()
        .map(VersionReq::parse)
        .transpose()
        .map_err(|error| {
            format!(
                "{}: dependency `{name}` has invalid version requirement: {error}",
                manifest.display()
            )
        })
}

/// Enforce the Holo package identifier alphabet before it is used as a
/// namespace root or a dependency identity.
fn validate_name(name: &str) -> Result<(), String> {
    // Builtins and Boolean literals are unavailable as package roots. The
    // standard-library package `holo` follows the ordinary identifier rules.
    if !name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        || matches!(name, "core" | "true" | "false")
    {
        return Err(format!("invalid package name `{name}`"));
    }
    Ok(())
}

/// Return a normalized relative path from a portable slash-separated string.
/// Only dependency paths may walk to a sibling package with `..`. The source
/// and entry paths stay within their declared root. This does no filesystem I/O.
pub(super) fn relative_path(value: &str, allow_parent: bool) -> Result<PathBuf, String> {
    // Do not let a Windows drive, backslash, or an absolute path change a
    // manifest's meaning when the project moves to another machine.
    if value.is_empty() || value.contains(['\\', ':', '\0']) || value.starts_with('/') {
        return Err(format!(
            "expected a relative slash-separated path, got {value:?}"
        ));
    }
    let mut result = PathBuf::new();
    // Drop redundant `.` components and preserve allowed `..` components until
    // filesystem resolution, where provider identity can be canonicalized.
    for component in Path::new(value).components() {
        match component {
            Component::Normal(name) => result.push(name),
            Component::CurDir => {}
            Component::ParentDir if allow_parent => result.push(".."),
            _ => return Err(format!("path leaves its declared root: {value:?}")),
        }
    }
    Ok(result)
}
