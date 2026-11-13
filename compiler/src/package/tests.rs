//! Exercise manifest, graph, and lock invariants with small on-disk packages.
//! Each test owns an isolated temporary directory. No native tools, registry,
//! or generated-program allocations are involved.

use super::load;
use crate::driver::TempDir;
use std::fs;
use std::path::{Path, PathBuf};

/// Write a fixture manifest and one source snapshot. `entry` chooses an
/// application fixture, otherwise the source is a library leaf.
fn package(directory: &Path, name: &str, dependencies: &str, entry: bool) -> PathBuf {
    let source = directory.join("src");
    fs::create_dir_all(&source).unwrap();
    // A dependency fixture has no application entry but still has hashed code.
    let entry_field = if entry { "entry = \"main.holo\"\n" } else { "" };
    let file = if entry { "main.holo" } else { "bytes.holo" };
    let manifest = directory.join("holo.toml");
    fs::write(
        &manifest,
        format!("[package]\nname = \"{name}\"\nversion = \"1.2.3\"\nsource = \"src\"\n{entry_field}{dependencies}"),
    ).unwrap();
    fs::write(source.join(file), "// exact source snapshot\n").unwrap();
    manifest
}

/// Build the same relative layout at any workspace location. Copying its lock
/// to an equivalent workspace should preserve the provider selection.
fn workspace(directory: &Path) -> PathBuf {
    package(&directory.join("wire"), "wire", "", false);
    package(
        &directory.join("app"),
        "app",
        "[dependencies]\nwire = { path = \"../wire\", version = \"^1\" }\n",
        true,
    )
}

/// Return the rejected load's diagnostic. A successful load violates the
/// negative test and fails immediately without producing any artifact.
fn rejection(path: &Path, locked: bool) -> String {
    load(path, locked)
        .err()
        .expect("expected package load to fail")
}

/// Verify a stable relative lock, editable root source, and exact dependency
/// snapshots, including additions and raw manifest edits.
#[test]
fn lock_pins_dependency_inputs_and_survives_relocation() {
    let first = TempDir::new().unwrap();
    let manifest = workspace(first.path());
    let lock = manifest.with_file_name("holo.lock");
    assert!(rejection(&manifest, true).contains("missing lockfile"));
    let graph = load(&manifest, false).unwrap();
    assert!(!lock.exists());
    graph.write_lock().unwrap();
    let original = fs::read_to_string(&lock).unwrap();
    assert!(
        original.contains("source = \"../wire/holo.toml\""),
        "{original}"
    );
    let graph = load(&manifest, true).unwrap();
    let dependency = &graph.packages["wire"];
    assert_eq!(
        dependency.sources[Path::new("bytes.holo")].text,
        "// exact source snapshot\n"
    );
    graph.write_lock().unwrap();
    assert_eq!(fs::read_to_string(&lock).unwrap(), original);

    let second = TempDir::new().unwrap();
    let relocated = workspace(second.path());
    fs::copy(&lock, relocated.with_file_name("holo.lock")).unwrap();
    fs::write(
        second.path().join("app/src/main.holo"),
        "// root source can be edited\n",
    )
    .unwrap();
    assert!(load(&relocated, true).is_ok());
    let added = second.path().join("wire/src/unused.holo");
    fs::write(&added, "// included in dependency identity\n").unwrap();
    assert!(rejection(&relocated, true).contains("dependency `wire` changed"));
    fs::remove_file(added).unwrap();
    let provider = second.path().join("wire/holo.toml");
    let previous = fs::read_to_string(&provider).unwrap();
    fs::write(
        &provider,
        format!("# manifest text changes the provider digest\n{previous}"),
    )
    .unwrap();
    assert!(rejection(&relocated, true).contains("package lock is stale"));
    assert_eq!(
        fs::read_to_string(relocated.with_file_name("holo.lock")).unwrap(),
        original
    );
}

/// Ensure a shared provider satisfies all incoming requirements, and reject
/// distinct directories claiming the same package identity.
#[test]
fn every_incoming_edge_checks_the_provider() {
    let directory = TempDir::new().unwrap();
    let root = directory.path();
    package(&root.join("wire"), "wire", "", false);
    package(&root.join("other_wire"), "wire", "", false);
    let app = package(
        &root.join("app"),
        "app",
        "[dependencies]\nleft = { path = \"../left\" }\nwire = { path = \"../wire\", version = \"^2\" }\n",
        true,
    );
    package(
        &root.join("left"),
        "left",
        "[dependencies]\nwire = { path = \"../wire\", version = \"^1\" }\n",
        false,
    );
    assert!(rejection(&app, false).contains("does not satisfy version requirement"));
    package(
        &root.join("left"),
        "left",
        "[dependencies]\nwire = { path = \"../other_wire\", version = \"^1\" }\n",
        false,
    );
    package(
        &root.join("app"),
        "app",
        "[dependencies]\nleft = { path = \"../left\" }\nwire = { path = \"../wire\", version = \"^1\" }\n",
        true,
    );
    assert!(rejection(&app, false).contains("multiple providers for package `wire`"));
}

/// Package requests are acyclic even when namespace bodies would never use a
/// dependency. Check the path reported for a back edge to the root.
#[test]
fn reports_the_dependency_cycle() {
    let directory = TempDir::new().unwrap();
    let root = directory.path();
    let app = package(
        &root.join("app"),
        "app",
        "[dependencies]\nwire = { path = \"../wire\" }\n",
        true,
    );
    package(
        &root.join("wire"),
        "wire",
        "[dependencies]\napp = { path = \"../app\" }\n",
        false,
    );
    assert!(rejection(&app, false).contains("package dependency cycle: app -> wire -> app"));
}

/// A source path cannot leave its package. A symlink cannot redirect source
/// discovery or lock installation to a different filesystem object.
#[cfg(unix)]
#[test]
fn symbolic_sources_and_lock_destinations_are_rejected() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new().unwrap();
    let manifest = workspace(directory.path());
    let root_source = directory.path().join("app/src");
    let link = root_source.join("escape.holo");
    symlink(directory.path().join("wire/src/bytes.holo"), &link).unwrap();
    assert!(rejection(&manifest, false).contains("symbolic package source path"));
    fs::remove_file(link).unwrap();
    let lock = manifest.with_file_name("holo.lock");
    symlink(&manifest, &lock).unwrap();
    let graph = load(&manifest, false).unwrap();
    assert!(
        graph
            .write_lock()
            .unwrap_err()
            .contains("expected a regular package lock")
    );
    assert!(rejection(&manifest, true).contains("expected a regular package lock"));
    let old = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        old.replace("source = \"src\"", "source = \"../wire/src\""),
    )
    .unwrap();
    assert!(rejection(&manifest, false).contains("invalid source root"));
}
