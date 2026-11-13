//! Command-level conformance for experimental local packages and lockfiles.
//!
//! The fixtures exercise real package snapshots, namespace binding, the checked
//! frontend, and LLVM native generation through the Cargo-built CLI. Flat
//! source controls preserve borrowed byte loads, destination construction,
//! and rollback, including when a hosted read supplies a short prefix.
//! The shipped standard library is checked separately under strict naming.
//! Temporary directories own all generated files. These tests make no timing or
//! compiler-memory measurement claim.

#[path = "../../support/hosted_stream.rs"]
mod hosted_stream;

use holo::driver::TempDir;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

/// Root manifest with an explicit entry and a version-constrained local library.
const MANIFEST: &str = r#"
[package]
name = "telemetry"
version = "0.1.0"
source = "src"
entry = "main.holo"

[dependencies]
wire = { path = "../wire", version = "^1" }
"#;

/// A library manifest may omit an entry when another package imports its code.
const WIRE_MANIFEST: &str = r#"
[package]
name = "wire"
version = "1.2.3"
source = "src"
"#;

/// Construct an initialized slot, borrow it, and print the decoded first byte.
const ENTRY: &str = "using telemetry.frame
struct Slot { data Bytes }
func seed returns Slot { return Slot { data: core.bytes(2, 7) } }
func main returns u64 {
    let memory = core.region(256) else { return 0 }
    let slots = core.batch(memory, Slot)
    let status = core.append(slots, seed())
    core.assert(status == 0)
    let slot = core.borrow_readonly(slots, 0)
    core.print(frame.decode(slot.data))
    return 0
}
";

/// Export a byte consumer that can call a private function from a sibling file.
const DECODE: &str = "namespace telemetry.frame
using telemetry.protocol
public func decode with noalloc, noblock arg input Bytes as readonly reference returns u64 {
    return ensure(protocol.normalize(input[0]))
}
";

/// Contribute a namespace-private arithmetic helper without importing anything.
const VALIDATE: &str = "namespace telemetry.frame
func ensure with noalloc, noblock arg value u64 as readonly copy returns u64 {
    return core.add_wrapping(value, 1)
}
";

/// Connect the root's two namespaces to an explicit manifest dependency.
const PROTOCOL: &str = "namespace telemetry.protocol
using wire.octets
public func normalize with noalloc, noblock arg value u64 as readonly copy returns u64 {
    return octets.double(value)
}
";

/// Supply a callable leaf in the dependency and preserve full-width wrapping.
const OCTETS: &str = "namespace wire.octets
public func double with noalloc, noblock arg value u64 as readonly copy returns u64 {
    return core.add_wrapping(value, value)
}
";

/// Owned package trees, scoped to a test's lockfiles, inputs, and executables.
struct Fixture {
    /// The temporary-directory owner removes generated artifacts on drop.
    scratch: TempDir,
}

impl Fixture {
    /// Create the package and its dependency in sibling directories. Test
    /// fixtures are small, immutable until a case deliberately overwrites them.
    fn new() -> Self {
        let fixture = Self {
            scratch: TempDir::new().unwrap(),
        };
        fixture.write("telemetry/holo.toml", MANIFEST);
        fixture.write("telemetry/src/main.holo", ENTRY);
        fixture.write("telemetry/src/frame/decode.holo", DECODE);
        fixture.write("telemetry/src/frame/validate.holo", VALIDATE);
        fixture.write("telemetry/src/protocol.holo", PROTOCOL);
        fixture.write("wire/holo.toml", WIRE_MANIFEST);
        fixture.write("wire/src/octets.holo", OCTETS);
        fixture
    }

    /// Reuse this fixture's namespaces and dependency graph as a library.
    /// Remove only the test-owned application entry and its manifest field.
    fn into_library(self) -> Self {
        fs::remove_file(self.path("telemetry/src/main.holo")).unwrap();
        self.write(
            "telemetry/holo.toml",
            &MANIFEST.replace("entry = \"main.holo\"\n", ""),
        );
        self
    }

    /// Resolve one test-owned path without consulting the process working directory.
    fn path(&self, relative: &str) -> PathBuf {
        self.scratch.path().join(relative)
    }

    /// Write source or metadata into this fixture, creating only its parent path.
    /// Fail immediately on a test setup error. Intentional overwrites are isolated.
    fn write(&self, relative: &str, contents: &str) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    /// Prepare a CLI process against the explicit root manifest. The caller
    /// owns execution and may append options before spawning the process.
    fn command(&self, action: &str) -> Command {
        let mut command = cli();
        command
            .args([action, "--package"])
            .arg(self.path("telemetry/holo.toml"));
        command
    }

    /// Prepare an LLVM-free command with an authoritative absent toolchain.
    /// A positive result proves the action reached no native discovery.
    fn metadata_command(&self, action: &str) -> Command {
        let mut command = self.command(action);
        command.env("HOLO_LLVM_PREFIX", self.path("no LLVM installation"));
        command
    }

    /// Read this fixture's exact published lockfile bytes for identity checks.
    fn lock_bytes(&self) -> Vec<u8> {
        fs::read(self.path("telemetry/holo.lock")).unwrap()
    }

    /// Generate or refresh the lockfile through the real CLI. Locking itself
    /// should require no LLVM installation.
    fn lock(&self) {
        success(self.metadata_command("lock").output().unwrap());
    }
}

/// Return a direct invocation of the Cargo-built compiler with inherited environment.
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_holo"))
}

/// Require successful completion and return stdout. Failures retain both
/// streams and process status for diagnosis.
fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "status {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output.stdout
}

/// Require rejection with no normal output and return the decoded diagnostic.
fn rejection(output: Output) -> String {
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    String::from_utf8(output.stderr).unwrap()
}

/// Require every expected term in one diagnostic and retain its original text
/// in assertion failures. Paths can be checked without depending on host roots.
fn assert_terms(diagnostic: &str, terms: &[&str]) {
    // Each term is an independent promised part of the source or lock diagnostic.
    for term in terms {
        assert!(diagnostic.contains(term), "missing {term:?}: {diagnostic}");
    }
}

/// Run the documented multi-file dashboard through the locked CLI. The
/// expected UI outputs witness all three child families and the catch-all.
#[test]
fn dashboard_error_union_example_runs_in_a_locked_package() {
    let manifest =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/error_unions/holo.toml");
    assert!(
        success(
            cli()
                .args(["check", "--locked", "--package"])
                .arg(&manifest)
                .env("HOLO_LLVM_PREFIX", "/missing-llvm-for-metadata-test")
                .output()
                .unwrap()
        )
        .is_empty()
    );
    for (profile, options) in [("O0", None), ("O3", Some("--release"))] {
        assert_eq!(
            success(
                cli()
                    .args(["run", "--locked", "--package"])
                    .arg(&manifest)
                    .args(options)
                    .output()
                    .unwrap()
            ),
            b"42\n1\n500\n3\n4\n",
            "{profile}"
        );
    }
}

/// Validate nominal child visibility and local aliases through the real
/// package binder, including a qualified handler for a public result.
#[test]
fn package_error_unions_bind_aliases_and_enforce_visibility() {
    let fixture = Fixture::new();
    const CHILD: &str = r#"namespace wire.errors
public error InputError { Bad(value u64) }
public func read returns Result<u64, InputError> { return error InputError.Bad(73) }
"#;
    const WRAPPER: &str = r#"namespace telemetry.parser
using wire.errors
error Local { Other }
public error ParseError { Invalid; from errors.InputError as Local }
public func parse returns Result<u64, ParseError> {
    let value = try errors.read()
    return ok value
}
public func recover returns u64 {
    return parse()
        on ok then 0
        on Local.Bad(value) then value
        on error then 1
}
"#;
    const ENTRY: &str = r#"using telemetry.parser
func main returns u64 {
    let parsed = parser.parse()
        on ok then 0
        on parser.ParseError.Local.Bad(value) then value
        on error then 1
    core.print(parsed)
    core.print(parser.recover())
    return 0
}
"#;
    fixture.write("wire/src/errors.holo", CHILD);
    fixture.write("telemetry/src/parser.holo", WRAPPER);
    fixture.write("telemetry/src/main.holo", ENTRY);
    fixture.lock();
    assert_eq!(
        success(
            fixture
                .command("run")
                .args(["--locked", "--release"])
                .output()
                .unwrap()
        ),
        b"73\n73\n"
    );
    // Test in a fresh library snapshot. Public exposure is rejected even
    // when nobody calls a function using the union. A neighboring contributor's
    // using never supplies the current file with a child type qualifier.
    let library = Fixture::new().into_library();
    library.write("wire/src/errors.holo", CHILD);
    library.write(
        "telemetry/src/parser/imports.holo",
        "namespace telemetry.parser\nusing wire.errors\n",
    );
    for (declaration, expected) in [
        (
            "error Hidden { Secret }\npublic error Visible { from Hidden as Child }",
            "cannot expose private child error family",
        ),
        (
            "public error Visible { from errors.InputError as Child }",
            "no `using` directive for namespace qualifier `errors` in this file",
        ),
    ] {
        library.write(
            "telemetry/src/parser/errors.holo",
            &format!("namespace telemetry.parser\n{declaration}\n"),
        );
        let diagnostic = rejection(library.metadata_command("check").output().unwrap());
        assert_terms(&diagnostic, &[expected, "errors.holo"]);
    }
}

/// Flatten the exact same helper bodies and ordered argument expressions into
/// ordinary standalone source. Namespace calls become plain calls, and a local
/// definition's explicit visibility marker is omitted from the control.
fn flat_control() -> String {
    let mut source = ENTRY
        .strip_prefix("using telemetry.frame\n")
        .unwrap()
        .replace("frame.decode(", "decode(");
    // Strip whole package header lines. The fixture deliberately has no other
    // matching directives inside a function body or comment.
    for helper in [DECODE, VALIDATE, PROTOCOL, OCTETS] {
        for line in helper.lines() {
            if !line.starts_with("namespace ") && !line.starts_with("using ") {
                source.push_str(line.strip_prefix("public ").unwrap_or(line));
                source.push('\n');
            }
        }
    }
    source
        .replace("protocol.normalize(", "normalize(")
        .replace("octets.double(", "double(")
}

/// Build and execute a local package using a sibling private helper,
/// two namespaces, a separate dependency, and borrowed Bytes. Compare exact
/// checked LLVM with the same source bodies in one file at all layouts.
#[test]
fn packaged_sources_share_private_helpers_and_native_lowering() {
    let fixture = Fixture::new();
    fixture.lock();
    let output = fixture
        .metadata_command("check")
        .arg("--locked")
        .output()
        .unwrap();
    assert!(success(output).is_empty());

    let debug = fixture.command("run").arg("--locked").output().unwrap();
    assert_eq!(success(debug), b"15\n");

    let binary = fixture.path("telemetry-worker");
    success(
        fixture
            .command("build")
            .args(["--locked", "--release", "-o"])
            .arg(&binary)
            .output()
            .unwrap(),
    );
    assert_eq!(success(Command::new(&binary).output().unwrap()), b"15\n");

    fixture.write("flat.holo", &flat_control());
    // Each layout has its own independently emitted baseline. Byte equality
    // detects package dispatch or storage differences without timing them.
    for mode in ["wide", "derived", "compact"] {
        let packaged = success(
            fixture
                .command("emit-llvm")
                .args(["--locked", "--layout", mode])
                .output()
                .unwrap(),
        );
        let flat = success(
            cli()
                .args(["emit-llvm", "--layout", mode])
                .arg(fixture.path("flat.holo"))
                .output()
                .unwrap(),
        );
        assert_eq!(packaged, flat, "package lowering changed {mode} LLVM");
    }
}

/// A lock requires explicit creation, survives root source edits, and changes
/// when a dependency's raw source or manifest changes. Verify that refresh
/// restores locked checking and produces byte-identical locks for equal inputs.
#[test]
fn package_lock_pins_dependency_manifest_and_source() {
    let fixture = Fixture::new();
    let check = || {
        fixture
            .metadata_command("check")
            .arg("--locked")
            .output()
            .unwrap()
    };
    assert_terms(&rejection(check()), &["holo.lock"]);
    fixture.lock();
    let original = fixture.lock_bytes();
    fixture.lock();
    assert_eq!(fixture.lock_bytes(), original);
    assert!(success(check()).is_empty());

    // The lock pins dependencies. Editing the root program keeps it usable.
    fixture.write("telemetry/src/main.holo", &format!("// Root edit\n{ENTRY}"));
    assert!(success(check()).is_empty());

    // The dependency manifest's exact bytes are pinned, including comments
    // that leave its package identity, version and dependency graph unchanged.
    fixture.write(
        "wire/holo.toml",
        &format!("# Dependency manifest edit\n{WIRE_MANIFEST}"),
    );
    assert_terms(&rejection(check()), &["lock"]);
    assert_eq!(fixture.lock_bytes(), original);
    fixture.write("wire/holo.toml", WIRE_MANIFEST);
    // A source edit also changes the pinned snapshot. An unlocked check can
    // compile it while leaving the old pin intact until an explicit refresh.
    fixture.write(
        "wire/src/octets.holo",
        &format!("// Dependency edit\n{OCTETS}"),
    );
    assert_terms(&rejection(check()), &["lock"]);
    assert_eq!(fixture.lock_bytes(), original);
    assert!(success(fixture.metadata_command("check").output().unwrap()).is_empty());
    assert_eq!(fixture.lock_bytes(), original);
    fixture.lock();
    assert_ne!(fixture.lock_bytes(), original);
    assert!(success(check()).is_empty());

    // A new root requirement is visible even when the selected version still
    // satisfies it. The old lock records the prior request until refresh.
    let refreshed = fixture.lock_bytes();
    fixture.write(
        "telemetry/holo.toml",
        &MANIFEST.replace("version = \"^1\"", "version = \"=1.2.3\""),
    );
    assert_terms(&rejection(check()), &["root dependency requests changed"]);
    assert_eq!(fixture.lock_bytes(), refreshed);
    fixture.lock();
    assert_ne!(fixture.lock_bytes(), refreshed);
    assert!(success(check()).is_empty());
}

/// Equal source trees yield equal locks even when their absolute paths differ.
#[test]
fn local_dependency_locks_are_relocatable() {
    let original = Fixture::new();
    let relocated = Fixture::new();
    original.lock();
    relocated.lock();
    assert_eq!(relocated.lock_bytes(), original.lock_bytes());

    // Copy a lock across roots and let the resolver verify it in that location.
    fs::copy(
        original.path("telemetry/holo.lock"),
        relocated.path("telemetry/holo.lock"),
    )
    .unwrap();
    assert!(
        success(
            relocated
                .metadata_command("check")
                .arg("--locked")
                .output()
                .unwrap()
        )
        .is_empty()
    );
}

/// Rejected dependency versions must leave the existing lock intact. The
/// invalid version is caught even without the `--locked` source-build gate.
#[test]
fn dependency_versions_are_checked_before_lock_publication() {
    let fixture = Fixture::new();
    fixture.lock();
    let original = fixture.lock_bytes();
    fixture.write("wire/holo.toml", &WIRE_MANIFEST.replace("1.2.3", "2.0.0"));
    let output = fixture.metadata_command("lock").output().unwrap();
    assert_terms(
        &rejection(output),
        &["wire", "does not satisfy version requirement"],
    );
    assert_eq!(fixture.lock_bytes(), original);
}

/// Duplicate private functions within one namespace must report both source
/// paths even though a caller never sees those functions in its public scope.
#[test]
fn duplicate_namespace_declarations_locate_both_files() {
    let fixture = Fixture::new();
    fixture.write(
        "telemetry/src/frame/conflict.holo",
        &VALIDATE.replace("return core.add_wrapping(value, 1)", "return value"),
    );
    let output = fixture.metadata_command("check").output().unwrap();
    assert_terms(
        &rejection(output),
        &["duplicate", "ensure", "conflict.holo", "validate.holo"],
    );
}

/// Check that package identity, privacy, and file-local directives are enforced
/// before LLVM discovery. Each case owns an independent, otherwise valid tree.
#[test]
fn namespace_identity_and_access_are_checked() {
    let private = ENTRY.replace("frame.decode(slot.data)", "frame.ensure(7)");
    let wrong_namespace = DECODE.replace("namespace telemetry.frame", "namespace telemetry.other");
    let missing_namespace = DECODE.strip_prefix("namespace telemetry.frame\n").unwrap();
    let missing_import =
        VALIDATE.replace("core.add_wrapping(value, 1)", "protocol.normalize(value)");
    let retired_directive =
        DECODE.replacen("using telemetry.protocol", "import telemetry.protocol", 1);
    let cases = [
        (
            "telemetry/src/main.holo",
            private.as_str(),
            &["main.holo", "has no public function", "ensure"][..],
        ),
        (
            "telemetry/src/frame/decode.holo",
            wrong_namespace.as_str(),
            &["decode.holo", "telemetry.other", "telemetry.frame"][..],
        ),
        (
            "telemetry/src/frame/decode.holo",
            missing_namespace,
            &["decode.holo", "expected `namespace telemetry.frame`"][..],
        ),
        (
            "telemetry/src/frame/validate.holo",
            missing_import.as_str(),
            &[
                "validate.holo",
                "no `using` directive for namespace qualifier",
                "protocol",
            ][..],
        ),
        (
            "telemetry/src/frame/decode.holo",
            retired_directive.as_str(),
            &[
                "decode.holo:2:",
                "replace `import` with `using` for namespace directives",
            ][..],
        ),
    ];
    // A case overwrites a single boundary to isolate its source diagnostic.
    for (path, source, terms) in cases {
        let fixture = Fixture::new();
        fixture.write(path, source);
        assert_terms(
            &rejection(fixture.metadata_command("check").output().unwrap()),
            terms,
        );
    }
}

/// Refuse ambiguous file layout and namespace cycles. A cycle in imports
/// exists even when a particular imported function is never called.
#[test]
fn conflicting_namespace_shapes_and_cycles_are_rejected() {
    let fixture = Fixture::new();
    fixture.write("telemetry/src/frame.holo", DECODE);
    assert_terms(
        &rejection(fixture.metadata_command("check").output().unwrap()),
        &["telemetry.frame", "has both a file and a directory"],
    );
    fs::remove_file(fixture.path("telemetry/src/frame.holo")).unwrap();
    fixture.write(
        "telemetry/src/protocol.holo",
        &PROTOCOL.replace(
            "using wire.octets\n",
            "using wire.octets\nusing telemetry.frame\n",
        ),
    );
    assert_terms(
        &rejection(fixture.metadata_command("check").output().unwrap()),
        &[
            "namespace dependency cycle",
            "telemetry.frame",
            "telemetry.protocol",
        ],
    );
}

/// The local qualifier is the final namespace component. Conflicting
/// qualifiers in a file must fail before either imported function can bind.
#[test]
fn equal_namespace_qualifiers_are_rejected_in_one_file() {
    let fixture = Fixture::new();
    fixture.write(
        "wire/src/frame.holo",
        &OCTETS.replace("namespace wire.octets", "namespace wire.frame"),
    );
    fixture.write(
        "telemetry/src/main.holo",
        &ENTRY.replace(
            "using telemetry.frame\n",
            "using telemetry.frame\nusing wire.frame\n",
        ),
    );
    assert_terms(
        &rejection(fixture.metadata_command("check").output().unwrap()),
        &[
            "main.holo",
            "namespace qualifier",
            "conflicts between",
            "telemetry.frame",
            "wire.frame",
        ],
    );
}

/// A dependency's transitive package remains inaccessible without a direct
/// manifest edge, even when the resolver has already read its sources.
#[test]
fn transitive_packages_require_direct_manifest_edges() {
    let fixture = Fixture::new();
    fixture.write(
        "wire/holo.toml",
        &format!("{WIRE_MANIFEST}\n[dependencies]\nbase = {{ path = \"../base\" }}\n"),
    );
    fixture.write(
        "base/holo.toml",
        &WIRE_MANIFEST.replace("name = \"wire\"", "name = \"base\""),
    );
    fixture.write(
        "base/src/octets.holo",
        &OCTETS.replace("namespace wire.octets", "namespace base.octets"),
    );
    fixture.write(
        "telemetry/src/protocol.holo",
        &PROTOCOL.replace("using wire.octets", "using base.octets"),
    );
    assert_terms(
        &rejection(fixture.metadata_command("check").output().unwrap()),
        &["protocol.holo", "base", "is not a direct dependency"],
    );
}

/// One resolved provider per package name prevents accidental merging of
/// equal names from independent source directories in transitive dependencies.
#[test]
fn conflicting_package_providers_are_rejected() {
    let fixture = Fixture::new();
    fixture.write(
        "telemetry/holo.toml",
        &format!("{MANIFEST}relay = {{ path = \"../relay\" }}\n"),
    );
    fixture.write(
        "wire/holo.toml",
        &format!("{WIRE_MANIFEST}\n[dependencies]\nbase = {{ path = \"../base-one\" }}\n"),
    );
    fixture.write(
        "relay/holo.toml",
        &format!(
            "{}\n[dependencies]\nbase = {{ path = \"../base-two\" }}\n",
            WIRE_MANIFEST.replace("name = \"wire\"", "name = \"relay\""),
        ),
    );
    fixture.write("relay/src/relay.holo", "namespace relay.relay\n");
    let base_manifest = WIRE_MANIFEST.replace("name = \"wire\"", "name = \"base\"");
    // Both directory trees supply the same named package and remain valid on
    // their own, so the ambiguity requires detecting two resolved providers.
    for directory in ["base-one", "base-two"] {
        fixture.write(&format!("{directory}/holo.toml"), &base_manifest);
        fixture.write(
            &format!("{directory}/src/base.holo"),
            "namespace base.base\n",
        );
    }
    assert_terms(
        &rejection(fixture.metadata_command("check").output().unwrap()),
        &["multiple providers for package", "base"],
    );
}

/// A cycle between manifests must be rejected before publishing a lock.
#[test]
fn package_dependency_cycles_are_rejected() {
    let fixture = Fixture::new();
    fixture.write(
        "wire/holo.toml",
        &format!("{WIRE_MANIFEST}\n[dependencies]\ntelemetry = {{ path = \"../telemetry\" }}\n"),
    );
    assert_terms(
        &rejection(fixture.metadata_command("lock").output().unwrap()),
        &["package dependency cycle", "telemetry", "wire"],
    );
    assert!(!fixture.path("telemetry/holo.lock").exists());
}

/// Package selection and locked mode have a single effective input. These
/// combinations should fail before loading a manifest or calling a tool.
#[test]
fn package_flags_are_mutually_exclusive_with_legacy_inputs() {
    let fixture = Fixture::new();
    let legacy_path = fixture.path("telemetry/src/main.holo");
    let old_root = fixture.path("wire/src");
    // A manifest and a positional entry describe competing compilation roots.
    assert_terms(
        &rejection(
            fixture
                .metadata_command("check")
                .arg(&legacy_path)
                .output()
                .unwrap(),
        ),
        &["--package cannot be combined with a source path"],
    );
    // The experimental leaf resolver has a distinct explicit source root.
    assert_terms(
        &rejection(
            fixture
                .metadata_command("check")
                .arg("--library-root")
                .arg(old_root)
                .output()
                .unwrap(),
        ),
        &["--package cannot be combined", "--library-root"],
    );
    // Standalone input has no dependency lockfile to verify.
    assert_terms(
        &rejection(
            cli()
                .args(["check", "--locked"])
                .arg(legacy_path)
                .output()
                .unwrap(),
        ),
        &["--locked requires --package"],
    );
}

/// Enforce naming on the actual shipped library, including private and unused
/// declarations. An absent LLVM prefix keeps this workspace gate source-only.
#[test]
fn shipped_standard_library_passes_strict_naming() {
    let scratch = TempDir::new().unwrap();
    let shipped = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../lib/holo/holo.toml");
    let output = cli()
        .args(["check", "--strict-names", "--package"])
        .arg(shipped)
        .env(
            "HOLO_LLVM_PREFIX",
            scratch.path().join("no LLVM installation"),
        )
        .output()
        .unwrap();
    assert!(success(output).is_empty());
}

/// Validate the shipped socket adapters at the package and native stream boundaries.
mod socket_results {
    use super::{Fixture, MANIFEST, hosted_stream, success};
    use std::fmt::Write;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    /// Prepare an application that imports the actual shipped provider.
    /// The temporary fixture owns the dependency link and all generated output.
    fn stdlib_fixture(source: &str) -> Fixture {
        let fixture = Fixture::new();
        let shipped = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../lib/holo");
        symlink(shipped, fixture.path("stdlib")).unwrap();
        fixture.write(
            "telemetry/holo.toml",
            &format!("{MANIFEST}holo = {{ path = \"../stdlib\", version = \"^0.1\" }}\n"),
        );
        fixture.write("telemetry/src/main.holo", source);
        fixture
    }

    /// Runtime outcomes map to disjoint observation ranges in the witness.
    /// Raw counts print unchanged. Native codes print with this added base.
    const NATIVE_BASE: u64 = 1 << 32;
    /// Observation of a signal interruption before progress.
    const INTERRUPTED: u64 = 1 << 33;
    /// Observation of a nonblocking attempt that made no progress.
    const WOULD_BLOCK: u64 = INTERRUPTED + 1;
    /// Observation of a raw word outside that operation's contract.
    const INVALID: u64 = INTERRUPTED + 2;

    /// Exercise exact successes, typed outcomes, native payloads and malformed
    /// words through a manifest dependency. A nested union also checks that
    /// `try` preserves the original error variant and native code across calls.
    #[test]
    fn socket_status_results_preserve_progress_and_native_error_payloads() {
        const NATIVE_TAG: u64 = 1 << 63;
        const MAX_COUNT: u64 = i32::MAX as u64;
        const FIRST_NATIVE: u64 = NATIVE_BASE + 1;
        const LAST_NATIVE: u64 = NATIVE_BASE + MAX_COUNT;
        const PROBE: &str = r#"
// Observe the three socket results through a locked package without allocation.
// Counters and typed error observations occupy separate numeric ranges.
namespace telemetry.socket_probe
using holo.net

// Wrap the child family to exercise automatic typed propagation through a call.
error ProbeError { from net.SocketError as Socket }

// Return the receive count or forward the socket error and any native payload.
func forward_recv with noalloc, noblock
    arg status u64 as readonly copy
    returns Result<u64, ProbeError>
{
    let count = try net.recv_result(status)
    return ok count
}

// Observe receive success (including EOF), either transient error, a native
// payload, or rejection of a word the receive adapter cannot produce.
public func observe_recv with noalloc, noblock
    arg status u64 as readonly copy
    returns u64
{
    return forward_recv(status)
        on ok(count) then count
        on Socket.Interrupted then 8589934592
        on Socket.WouldBlock then 8589934593
        on Socket.Native(code) then core.add_wrapping(4294967296, code)
        on Socket.InvalidStatus then 8589934594
}

// Observe exact positive progress and distinguish every typed failure.
public func observe_send with noalloc, noblock
    arg status u64 as readonly copy
    returns u64
{
    return net.send_result(status)
        on ok(count) then count
        on net.SocketError.Interrupted then 8589934592
        on net.SocketError.WouldBlock then 8589934593
        on net.SocketError.Native(code) then core.add_wrapping(4294967296, code)
        on net.SocketError.InvalidStatus then 8589934594
}

// Observe readiness, interruption, a native error, or an inadmissible word.
public func observe_wait with noalloc, noblock
    arg status u64 as readonly copy
    returns u64
{
    return net.wait_result(status)
        on ok(ready) then ready
        on net.SocketError.Interrupted then 8589934592
        on net.SocketError.WouldBlock then 8589934593
        on net.SocketError.Native(code) then core.add_wrapping(4294967296, code)
        on net.SocketError.InvalidStatus then 8589934594
}
"#;

        let cases = [
            (0, [0, INVALID, 0]),
            (1, [1, 1, INVALID]),
            (23, [23, 23, INVALID]),
            (MAX_COUNT, [MAX_COUNT, MAX_COUNT, INVALID]),
            (MAX_COUNT + 1, [INVALID; 3]),
            (NATIVE_TAG - 1, [INVALID; 3]),
            (NATIVE_TAG, [INVALID; 3]),
            (NATIVE_TAG | 1, [FIRST_NATIVE; 3]),
            (NATIVE_TAG | 123, [NATIVE_BASE + 123; 3]),
            (NATIVE_TAG | MAX_COUNT, [LAST_NATIVE; 3]),
            (NATIVE_TAG | (MAX_COUNT + 1), [INVALID; 3]),
            (NATIVE_TAG | (1 << 32) | 1, [INVALID; 3]),
            (u64::MAX - 2, [INVALID; 3]),
            (u64::MAX - 1, [WOULD_BLOCK, WOULD_BLOCK, INVALID]),
            (u64::MAX, [INTERRUPTED; 3]),
        ];
        let operations = ["observe_recv", "observe_send", "observe_wait"];
        let mut entry = String::from(
            "// Print the independently classified results in status and operation order.\n\
             using telemetry.socket_probe\n\
             // Calls the checked probe with each raw status and returns zero after printing.\n\
             func main returns u64 {\n",
        );
        let mut expected = Vec::new();
        // Every operation receives every word, including unsupported sentinels.
        for (status, observations) in cases {
            for (operation, observation) in operations.into_iter().zip(observations) {
                writeln!(entry, "    core.print(socket_probe.{operation}({status}))").unwrap();
                expected.push((status, operation, observation));
            }
        }
        entry.push_str("    return 0\n}\n");
        let fixture = stdlib_fixture(&entry);
        fixture.write("telemetry/src/socket_probe.holo", PROBE);
        fixture.lock();
        success(
            fixture
                .metadata_command("check")
                .args(["--locked", "--strict-names"])
                .output()
                .unwrap(),
        );

        // Run the same locked provider for each supported layout and optimization level.
        for layout in ["wide", "derived", "compact"] {
            for options in [None, Some("--release")] {
                let stdout = success(
                    fixture
                        .command("run")
                        .args(["--locked", "--layout", layout])
                        .args(options)
                        .output()
                        .unwrap(),
                );
                let output = String::from_utf8(stdout).unwrap();
                let lines: Vec<_> = output.lines().collect();
                assert_eq!(
                    lines.len(),
                    expected.len(),
                    "{layout} {options:?}: {output}"
                );
                // Keep the raw status and operation in assertion diagnostics.
                for (line, (status, operation, observation)) in lines.into_iter().zip(&expected) {
                    assert_eq!(
                        line.parse::<u64>().unwrap(),
                        *observation,
                        "{layout} {options:?}: {operation}({status:#x})",
                    );
                }
            }
        }
    }

    /// Drive the real hosted runtime with a connected stream. Its wait, send,
    /// and empty receive words cross the package API as typed outcomes.
    #[test]
    fn socket_status_results_accept_native_connected_stream_progress() {
        const ENTRY: &str = r#"
// Feed native wait, send, and empty receive outcomes to the socket adapters.
using holo.net

// Own three initialized reply bytes inline until the synchronous send returns.
struct Reply { wire Bytes }

// Construct the owned bytes directly in the selected batch slot.
func reply with noblock returns Reply { return Reply { wire: core.bytes(3, 0x41) } }

// Print typed readiness, send progress, and the outcome of an empty receive.
// An allocation failure exits before any I/O and leaves the witness absent.
func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let replies = core.batch(memory, Reply)
    let added = core.append(replies, reply())
    core.assert(added == 0)
    let prepared = core.borrow_readwrite(replies, 0)

    let wait_status = core.wait_output()
    let ready = net.wait_result(wait_status)
        on ok(value) then value
        on error then 99
    let send_status = core.send(prepared.wire, 0, 3)
    let sent = net.send_result(send_status)
        on ok(count) then count
        on error then 99
    core.print(ready)
    core.print(sent)

    // The peer sends no data. An open peer yields WouldBlock and a half-closed
    // peer yields EOF, which does not prevent the preceding send.
    refill prepared.wire, 1 as received {
        core.print(98)
    } else status {
        let read = net.recv_result(status)
            on ok(count) then count
            on net.SocketError.WouldBlock then 4
            on error then 99
        core.print(read)
    }
    return 0
}
"#;
        let fixture = stdlib_fixture(ENTRY);
        fixture.lock();
        let executable = fixture.path("socket-progress");
        success(
            fixture
                .command("build")
                .args(["--locked", "--release", "-o"])
                .arg(&executable)
                .output()
                .unwrap(),
        );
        let open = hosted_stream::read_stream(&executable, &[], false);
        let half_closed = hosted_stream::read_stream(&executable, &[], true);
        assert_eq!(open.stdout, b"0\n3\n4\n");
        assert_eq!(half_closed.stdout, b"0\n3\n0\n");
    }
}

/// Library checking validates a locked package with split private helpers and
/// explicit dependencies, without an application entry or LLVM.
#[test]
fn library_only_packages_check_without_an_entry() {
    let fixture = Fixture::new().into_library();
    // Only namespaces reached through imports need checking in dependencies.
    // This source is part of the pinned snapshot but no root file imports it.
    fixture.write(
        "wire/src/unused.holo",
        "namespace wire.unused\nfunc broken returns u64 {\n",
    );
    assert!(success(fixture.metadata_command("check").output().unwrap()).is_empty());
    fixture.lock();
    assert!(
        success(
            fixture
                .metadata_command("check")
                .arg("--locked")
                .output()
                .unwrap()
        )
        .is_empty()
    );

    // The imported dependency's bytes are checked through the ordinary
    // frontend even when the library root has no program entry or call site.
    fixture.write(
        "wire/src/octets.holo",
        &OCTETS.replace("core.add_wrapping(value, value)", "missing(value)"),
    );
    assert_terms(
        &rejection(fixture.metadata_command("check").output().unwrap()),
        &["octets.holo", "missing"],
    );
}

/// Put a naming violation in a root contributor and a reachable dependency.
/// Both bad-style declarations remain valid and referenced by their consumers.
fn naming_fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.write(
        "telemetry/src/frame/validate.holo",
        &VALIDATE.replace("ensure", "ensureValue"),
    );
    fixture.write(
        "telemetry/src/frame/decode.holo",
        &DECODE.replace("ensure", "ensureValue"),
    );
    fixture.write(
        "telemetry/src/protocol.holo",
        &PROTOCOL.replace("octets.double", "octets.doubleValue"),
    );
    fixture.write(
        "wire/src/octets.holo",
        &OCTETS.replace("double", "doubleValue"),
    );
    fixture
}

/// The selected root owns naming diagnostics for its original declarations.
#[test]
fn package_naming_diagnostics_exclude_reachable_dependencies() {
    let fixture = naming_fixture();
    fixture.lock();
    let output = fixture
        .metadata_command("check")
        .arg("--locked")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_terms(&stderr, &["validate.holo:2:6: warning:", "ensureValue"]);
    assert_eq!(stderr.matches(": warning:").count(), 1, "{stderr}");
    assert!(!stderr.contains("doubleValue"), "{stderr}");
    assert!(success(output).is_empty());

    // Selecting the dependency directly makes its declarations part of the lint scope.
    let output = cli()
        .args(["check", "--package"])
        .arg(fixture.path("wire/holo.toml"))
        .env("HOLO_LLVM_PREFIX", fixture.path("no LLVM installation"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_terms(&stderr, &["octets.holo:2:13: warning:", "doubleValue"]);
    assert_eq!(stderr.matches(": warning:").count(), 1, "{stderr}");
    assert!(!stderr.contains("ensureValue"), "{stderr}");
    assert!(success(output).is_empty());
}

/// Strict checking of an entryless library includes its unreferenced namespaces.
#[test]
fn library_naming_diagnostics_cover_root_contributors() {
    let fixture = naming_fixture().into_library();
    fixture.write(
        "telemetry/src/unused.holo",
        "namespace telemetry.unused\nfunc unusedValue returns u64 { return 0 }\n",
    );
    fixture.lock();
    let stderr = rejection(
        fixture
            .metadata_command("check")
            .args(["--locked", "--strict-names"])
            .output()
            .unwrap(),
    );
    assert_terms(
        &stderr,
        &[
            "validate.holo:2:6: error:",
            "ensureValue",
            "unused.holo:2:6: error:",
            "unusedValue",
        ],
    );
    assert_eq!(stderr.matches(": error:").count(), 2, "{stderr}");
    assert!(!stderr.contains(": warning:"), "{stderr}");
    assert!(!stderr.contains("doubleValue"), "{stderr}");
}

/// An application checks the namespaces its entry imports. Library checking
/// also checks unreferenced namespaces and private functions with no callers.
#[test]
fn library_check_rejects_an_unused_private_function_body() {
    let fixture = Fixture::new();
    fixture.write(
        "telemetry/src/unused.holo",
        "namespace telemetry.unused
func invalid with noalloc arg input Bytes as readonly reference returns u64 {
    input[0] = 1
    return 0
}

",
    );
    // The app never imports telemetry.unused, so its entry path stays valid.
    assert!(success(fixture.metadata_command("check").output().unwrap()).is_empty());
    let fixture = fixture.into_library();
    assert_terms(
        &rejection(fixture.metadata_command("check").output().unwrap()),
        &[
            "unused.holo",
            "cannot write through a readonly byte parameter",
        ],
    );

    // A recursive ordinary call can lower to IR but its call graph fails the
    // common checker. Library checking must reach that checker with no entry.
    fixture.write(
        "telemetry/src/unused.holo",
        "namespace telemetry.unused
func recurse arg input Bytes as readonly reference returns u64 {
    return recurse(input)
}
",
    );
    assert_terms(
        &rejection(fixture.metadata_command("check").output().unwrap()),
        &["recursive ordinary call graph", "telemetry.unused.recurse"],
    );
}

/// Public typed results can be imported, with qualifiers bound by nominal
/// declaration. Library-only checking rejects even an unused private exposure.
#[test]
fn public_errors_follow_package_visibility() {
    let fixture = Fixture::new();
    let definition = "namespace telemetry.admission
public error Rejection { BadMagic; Truncated(required u64) }
public func validate arg magic u64 as readonly copy returns Result<u64, Rejection> {
    if magic == 0 { return error Rejection.BadMagic }
    if magic == 1 { return error Rejection.Truncated(core.add_wrapping(magic, 1)) }
    return ok magic
}
";
    fixture.write("telemetry/src/admission.holo", definition);
    fixture.write(
        "telemetry/src/main.holo",
        "using telemetry.admission
func main returns u64 {
    let result = admission.validate(0)
        on ok(value) then value
        on admission.Rejection.BadMagic then 7
        on admission.Rejection.Truncated(required) then required
    let missing = admission.validate(1)
        on ok(value) then value
        on admission.Rejection.Truncated(required) then required
        on error then 99
    core.print(result)
    core.print(missing)
    return 0
}
",
    );
    fixture.lock();
    success(
        fixture
            .metadata_command("check")
            .arg("--locked")
            .output()
            .unwrap(),
    );
    // Compile the exact imported provider with both native optimization levels.
    for options in [None, Some("--release")] {
        assert_eq!(
            success(
                fixture
                    .command("run")
                    .arg("--locked")
                    .args(options)
                    .output()
                    .unwrap()
            ),
            b"7\n2\n"
        );
    }
    let fixture = fixture.into_library();
    fixture.write(
        "telemetry/src/admission.holo",
        &definition.replace("public error", "error"),
    );
    assert_terms(
        &rejection(fixture.metadata_command("check").output().unwrap()),
        &["admission.holo", "private error family `Rejection`"],
    );
}

/// Every library-owned Holo file contributes to its path-selected namespace.
/// Invalid layouts and headers must be caught even without a dependency edge.
#[test]
fn library_check_rejects_unreferenced_namespace_layout_and_header_errors() {
    let cases = [
        (
            "telemetry/src/unused.holo",
            "func helper returns u64 { return 0 }\n",
            &["unused.holo", "expected `namespace telemetry.unused`"][..],
        ),
        (
            "telemetry/src/unused.holo",
            "namespace telemetry.other\nfunc helper returns u64 { return 0 }\n",
            &[
                "unused.holo",
                "telemetry.other",
                "does not match source path",
                "telemetry.unused",
            ][..],
        ),
        (
            "telemetry/src/unused/deep/extra.holo",
            "namespace telemetry.unused\nfunc helper returns u64 { return 0 }\n",
            &[
                "extra.holo",
                "telemetry.unused",
                "does not match source path",
                "telemetry.unused.deep",
            ][..],
        ),
        (
            "telemetry/src/Caps.holo",
            "namespace telemetry.caps\nfunc helper returns u64 { return 0 }\n",
            &["Caps.holo", "invalid library namespace path"][..],
        ),
        (
            "telemetry/src/frame.holo",
            DECODE,
            &["telemetry.frame", "has both a file and a directory"][..],
        ),
    ];
    // Each layout is rejected on its own, so an earlier error cannot satisfy
    // the check for another misplaced or incorrectly declared source file.
    for (path, source, terms) in cases {
        let fixture = Fixture::new().into_library();
        fixture.write(path, source);
        assert_terms(
            &rejection(fixture.metadata_command("check").output().unwrap()),
            terms,
        );
    }
}

/// Verify nested package namespaces without changing the isolated root profile.
mod nested_namespaces {
    use super::{Fixture, assert_terms, rejection, success};

    /// Resolve a single-file parent, split children and a grandchild. The
    /// entryless checker also discovers nested files from their physical path.
    #[test]
    fn parent_sources_and_child_contributors_stay_independent() {
        let fixture = Fixture::new();
        fixture.write(
            "wire/src/net.holo",
            "namespace wire.net\npublic func version returns u64 { return 3 }\n",
        );
        fixture.write(
            "wire/src/net/tcp/protocol.holo",
            "namespace wire.net.tcp\nusing wire.net.tcp.framing\npublic func code returns u64 { return core.add_wrapping(framing.bias(), tag()) }\n",
        );
        fixture.write(
            "wire/src/net/tcp/tag.holo",
            "namespace wire.net.tcp\nfunc tag returns u64 { return 4 }\n",
        );
        fixture.write(
            "wire/src/net/tcp/framing/record.holo",
            "namespace wire.net.tcp.framing\npublic func bias returns u64 { return 5 }\n",
        );
        fixture.write(
            "telemetry/src/frame/control/state.holo",
            "namespace telemetry.frame.control\npublic func state returns u64 { return 6 }\n",
        );
        fixture.write(
            "telemetry/src/main.holo",
            "using wire.net\nusing wire.net.tcp\nusing telemetry.frame\nusing telemetry.frame.control\nfunc main returns u64 {\n    core.print(net.version())\n    core.print(tcp.code())\n    core.print(control.state())\n    return 0\n}\n",
        );
        fixture.lock();
        let mut run = fixture.command("run");
        assert_eq!(success(run.arg("--locked").output().unwrap()), b"3\n9\n6\n");

        // Removing the entry must still select the path-derived child
        // namespace and bind the split parent without absorbing its files.
        let library = fixture.into_library();
        success(library.metadata_command("check").output().unwrap());
    }

    /// Two paths in one package can end with the same written qualifier.
    /// The compiler distinguishes that collision from a repeated import.
    #[test]
    fn different_nested_paths_cannot_share_one_qualifier() {
        let fixture = Fixture::new();
        fixture.write(
            "telemetry/src/main.holo",
            "using wire.net.tcp\nusing wire.experimental.tcp\nfunc main returns u64 { return 0 }\n",
        );
        assert_terms(
            &rejection(fixture.metadata_command("check").output().unwrap()),
            &[
                "namespace qualifier `tcp` conflicts",
                "wire.net.tcp",
                "wire.experimental.tcp",
            ],
        );
    }

    /// An immediate contributor's filename does not make it a nested namespace.
    #[test]
    fn a_parent_contributor_does_not_satisfy_a_child_import() {
        let fixture = Fixture::new();
        fixture.write(
            "wire/src/net/tcp.holo",
            "namespace wire.net\npublic func code returns u64 { return 2 }\n",
        );
        fixture.write(
            "telemetry/src/main.holo",
            "using wire.net.tcp\nfunc main returns u64 { return 0 }\n",
        );
        assert_terms(
            &rejection(fixture.metadata_command("check").output().unwrap()),
            &["no source files for namespace `wire.net.tcp`"],
        );
    }
}

/// A library must discover cycles even among its otherwise unused namespaces.
#[test]
fn library_check_rejects_unused_namespace_cycles() {
    let fixture = Fixture::new().into_library();
    fixture.write(
        "telemetry/src/outer.holo",
        "namespace telemetry.outer\nusing telemetry.inner\nfunc one returns u64 { return 1 }\n",
    );
    fixture.write(
        "telemetry/src/inner.holo",
        "namespace telemetry.inner\nusing telemetry.outer\nfunc two returns u64 { return 2 }\n",
    );
    assert_terms(
        &rejection(fixture.metadata_command("check").output().unwrap()),
        &[
            "namespace dependency cycle",
            "telemetry.outer",
            "telemetry.inner",
        ],
    );
}

/// Library validation is check-only. Entry actions fail before LLVM discovery
/// and leave output paths untouched when there is no executable entry.
#[test]
fn executable_actions_require_an_entry_in_library_packages() {
    let fixture = Fixture::new().into_library();
    // Both artifact actions reject the manifest before writing the requested path.
    for action in ["build", "emit-llvm"] {
        let path = fixture.path(&format!("{action}.out"));
        let output = fixture
            .metadata_command(action)
            .arg("-o")
            .arg(&path)
            .output()
            .unwrap();
        assert_terms(&rejection(output), &["telemetry", "no entry"]);
        assert!(!path.exists());
    }
    let output = fixture.metadata_command("run").output().unwrap();
    assert_terms(&rejection(output), &["telemetry", "no entry"]);
}

/// Fixed records and destination producers linked from checked package source.
mod fixed_records {
    use super::{Fixture, MANIFEST, WIRE_MANIFEST, assert_terms, cli, rejection, success};
    use holo::driver::NativeToolchain;
    use std::process::Command;

    /// Declare the fixed schema in a different namespace contributor than its constructor.
    const HEADER: &str = "namespace wire.frame
public struct Header { payload_length u64, kind u64 }
";

    /// A rejected kind leaves the receiving append attempt unpublished.
    const MAKE: &str = "namespace wire.frame
public func make with noblock
    arg length u64 as readonly copy
    arg kind u64 as readonly copy
    returns Header
{
    core.reject_if(kind > 2)
    return Header { payload_length: length, kind: kind }
}
";

    /// Export a nominal error family from a different namespace contributor.
    const FRAME_ERROR: &str = "namespace wire.frame
public error FrameError { Oversized(length u64); BadKind }
";

    /// Guard order distinguishes the full-width payload from the next case.
    /// Success initializes both fields directly in the caller's receiving slot.
    const TYPED_MAKE: &str = "namespace wire.frame
public func admit with noblock
    arg length u64 as readonly copy
    arg kind u64 as readonly copy
    returns Result<Header, FrameError>
{
    if length > 4096 { return error FrameError.Oversized(length) }
    if kind > 2 { return error FrameError.BadKind }
    return ok Header:
        payload_length = length
        kind = kind
}
";

    /// Forward the same public aggregate result across a locked dependency.
    /// The called constructor still initializes the caller's receiving slot.
    const TYPED_FORWARD: &str = "namespace wire.frame
public func forward with noblock
    arg length u64 as readonly copy
    arg kind u64 as readonly copy
    returns Result<Header, FrameError>
{
    return admit(length, kind)
}
";

    /// Observe typed recovery, a successful retry, and capacity failure after
    /// the first field write with a retained reference to the published prefix.
    const TYPED_PROGRAM: &str = "using wire.frame
func main returns u64 {
    let memory = core.region(64) else { core.assert(false); return 0 }
    let headers = core.batch(memory, frame.Header)
    let seed = core.append(headers, frame.admit(52, 1))
        on ok then true
        on error then false
    core.assert(seed)
    let first = core.borrow_readonly(headers, 0)
    let original = core.address(first)
    let checkpoint = core.used(memory)
    let rejected = core.append(headers, frame.admit(18446744073709551615, 9))
        on ok then 0
        on Rejected frame.FrameError.Oversized(length) then length
        on error then 1
    core.assert(core.used(memory) == checkpoint)
    core.assert(core.len(headers) == 1)
    let bad_kind = core.append(headers, frame.admit(7, 9))
        on ok then 0
        on Rejected frame.FrameError.BadKind then core.used(memory)
        on error then 1
    core.assert(bad_kind == checkpoint)
    core.assert(core.used(memory) == checkpoint)
    core.assert(core.len(headers) == 1)
    let retry = core.append(headers, frame.admit(18, 2))
        on ok then core.len(headers)
        on error then 0
    core.assert(retry == 2)
    let second = core.borrow_readonly(headers, 1)
    let committed = core.used(memory)
    let full = core.append(headers, frame.admit(8, 0))
        on ok then 0
        on Full then core.used(memory)
        on error then 1
    core.assert(full == committed)
    core.assert(core.used(memory) == committed)
    core.assert(core.len(headers) == 2)
    core.assert(core.address(first) == original)
    core.print(rejected)
    core.print(bad_kind)
    core.print(full)
    core.print(committed)
    core.print(first.payload_length)
    core.print(first.kind)
    core.print(second.payload_length)
    core.print(second.kind)
    core.print(core.sub_wrapping(core.address(second), original))
    return 0
}
";

    /// Fill two final slots, retain an alias through rejection and retry, and
    /// prove exact used bytes, publication count, and address stability.
    const PROGRAM: &str = "using wire.frame
func main returns u64 {
    let memory = core.region(48) else { core.assert(false); return 0 }
    let headers = core.batch(memory, frame.Header)
    let initial = core.append(headers, frame.make(52, 1))
    core.assert(initial == 0)
    {
        let first = core.borrow_readwrite(headers, 0)
        let retained = core.alias_readonly(first)
        let original = core.address(retained)
        let before = core.used(memory)
        let rejected = core.append(headers, frame.make(7, 3))
        core.assert(rejected == 3)
        core.assert(core.used(memory) == before)
        core.assert(core.len(headers) == 1)
        core.assert(core.address(retained) == original)
        core.assert(retained.payload_length == 52)
        let retry = core.append(headers, frame.make(18, 2))
        core.assert(retry == 0)
        core.assert(core.address(retained) == original)
        let second = core.borrow_readonly(headers, 1)
        first.kind = 9
        let full = core.append(headers, frame.make(8, 0))
        core.assert(full == 1)
        core.print(rejected)
        core.print(full)
        core.print(core.len(headers))
        core.print(core.used(memory))
        core.print(retained.payload_length)
        core.print(retained.kind)
        core.print(second.payload_length)
        core.print(second.kind)
        core.print(core.sub_wrapping(core.address(second), original))
    }
    core.reset(headers)
    core.assert(core.used(memory) == 0)
    core.assert(core.len(headers) == 0)
    return 0
}
";

    /// Populate a test-owned locked-package candidate with split record sources.
    /// Callers lock after making any intended dependency changes.
    fn fixture() -> Fixture {
        let fixture = Fixture::new();
        fixture.write("wire/src/frame/header.holo", HEADER);
        fixture.write("wire/src/frame/make.holo", MAKE);
        fixture.write("telemetry/src/main.holo", PROGRAM);
        fixture
    }

    /// Drop namespace/public metadata for the same-file control. Keep the
    /// record, field, and constructor order and all operations unchanged.
    fn flat_source(program: &str) -> String {
        let header = HEADER
            .strip_prefix("namespace wire.frame\npublic ")
            .unwrap();
        let make = MAKE.strip_prefix("namespace wire.frame\npublic ").unwrap();
        let entry = program
            .strip_prefix("using wire.frame\n")
            .unwrap()
            .replace("frame.Header", "Header")
            .replace("frame.make", "make");
        format!("{header}{make}{entry}")
    }

    /// The provider's schema and the caller's destination must become the same
    /// checked record identity without changing layout or adding a call wrapper.
    #[test]
    fn imported_record_preserves_native_layout_failure_and_retained_references() {
        let fixture = fixture();
        fixture.lock();
        assert!(
            success(
                fixture
                    .metadata_command("check")
                    .arg("--locked")
                    .output()
                    .unwrap()
            )
            .is_empty()
        );
        let control = fixture.path("record-control.holo");
        let tools = NativeToolchain::discover().unwrap();

        // Both source forms must use identical checked lowering and publish the
        // hand-computed values under every layout. Size backing for exactly
        // two records so all modes see the same reject, retry, and full cases.
        for (mode, size, header) in [("wide", 24, 8), ("derived", 16, 0), ("compact", 16, 0)] {
            let capacity = 2 * size;
            let source = PROGRAM.replace("core.region(48)", &format!("core.region({capacity})"));
            fixture.write("telemetry/src/main.holo", &source);
            fixture.write("record-control.holo", &flat_source(&source));
            let expected = format!("3\n1\n2\n{capacity}\n52\n9\n18\n2\n{size}\n");

            let description = String::from_utf8(success(
                fixture
                    .metadata_command("inspect-layout")
                    .args(["--locked", "--layout", mode])
                    .output()
                    .unwrap(),
            ))
            .unwrap();
            assert_terms(
                &description,
                &[
                    &format!(
                        "wire.frame.Header: header={header} unit=0 fixed={size} minimum={size}"
                    ),
                    &format!("payload_length: offset={header} bytes; extent=8"),
                    &format!("kind: offset={} bytes; extent=8", header + 8),
                ],
            );

            let packaged = success(
                fixture
                    .command("emit-llvm")
                    .args(["--locked", "--layout", mode])
                    .output()
                    .unwrap(),
            );
            let flat = success(
                cli()
                    .args(["emit-llvm", "--layout", mode])
                    .arg(&control)
                    .output()
                    .unwrap(),
            );
            assert_eq!(packaged, flat, "package record changed {mode} LLVM");

            // Exercise both real optimization modes on both programs. The
            // independent output includes sizes but no absolute addresses.
            for release in [false, true] {
                let mut packaged = fixture.command("run");
                packaged.args(["--locked", "--layout", mode]);
                let mut flat = cli();
                flat.args(["run", "--layout", mode]).arg(&control);
                if release {
                    packaged.arg("--release");
                    flat.arg("--release");
                }
                assert_eq!(success(packaged.output().unwrap()), expected.as_bytes());
                assert_eq!(success(flat.output().unwrap()), expected.as_bytes());
            }

            // Instrument the same emitted module with the ordinary native
            // driver. The failed and successful paths share the output oracle.
            let sanitized = fixture.path(&format!("record-{mode}-asan"));
            let ir = String::from_utf8(packaged).unwrap();
            tools.build_sanitized(&ir, &sanitized).unwrap();
            assert_eq!(
                success(Command::new(sanitized).output().unwrap()),
                expected.as_bytes()
            );
        }
    }

    /// Load the independent dependency contributors into one locked namespace.
    /// The caller may still change its own entry to exercise nominal rejection.
    fn typed_fixture() -> Fixture {
        let fixture = Fixture::new();
        fixture.write("wire/src/frame/header.holo", HEADER);
        fixture.write("wire/src/frame/errors.holo", FRAME_ERROR);
        fixture.write("wire/src/frame/admit.holo", TYPED_MAKE);
        fixture.write("telemetry/src/main.holo", TYPED_PROGRAM);
        fixture
    }

    /// Resolve a public producer and a qualified rejection in a locked package.
    /// The rejection block calls another namespace and retries in the same tail.
    #[test]
    fn package_scoped_construction_resolves_outcomes_and_retry_calls() {
        let fixture = typed_fixture();
        fixture.write("wire/src/frame/forward.holo", TYPED_FORWARD);
        fixture.write(
            "telemetry/src/main.holo",
            "using wire.frame
using wire.octets

func main returns u64 {
    let memory = core.region(64) else { core.assert(false); return 0 }
    core.construct(memory, frame.forward(4097, 9))
        on ok then { core.assert(false) }
        on Rejected frame.FrameError.Oversized(length) then {
            core.assert(core.used(memory) == 0)
            core.print(octets.double(length))
            core.construct(memory, frame.forward(52, 1))
                on ok(header) then {
                    core.print(octets.double(header.payload_length))
                    core.print(header.kind)
                }
                on error then { core.assert(false) }
        }
        on error then { core.assert(false) }
    core.assert(core.used(memory) == 0)
    return 0
}
",
        );
        fixture.lock();
        assert!(
            success(
                fixture
                    .metadata_command("check")
                    .arg("--locked")
                    .output()
                    .unwrap()
            )
            .is_empty()
        );
        let run = fixture
            .command("run")
            .args(["--locked", "--release"])
            .output()
            .unwrap();
        assert_eq!(success(run), b"8194\n104\n1\n");
    }

    /// Select an implicit destination in a locked package. Imported calls in
    /// the budget and both branches retain their ordinary namespace identities.
    /// Reuse the same construction site after a named rejection.
    #[test]
    fn package_implicit_scoped_construction_preserves_qualified_calls() {
        let fixture = typed_fixture();
        fixture.write("wire/src/frame/forward.holo", TYPED_FORWARD);
        let source = r#"using wire.frame
using wire.octets

func main returns u64 {
    with core.region(octets.double(32)) {
        var next_length = 4097
        for attempt in core.range(2) {
            core.construct(frame.forward(next_length, 1))
                on ok(header) then {
                    core.print(octets.double(header.payload_length))
                    core.print(header.kind)
                }
                on Rejected frame.FrameError.Oversized(length) then {
                    core.print(octets.double(length))
                }
                on error then { core.assert(false) }
            next_length = 52
        }
    } else {
        core.print(octets.double(9))
    }
    core.print(99)
    return 0
}
"#;
        // The explicit control supplies the same budget and recovery through
        // a named region in an ordinary brace scope.
        let explicit = source
            .replace(
                "    with core.region(octets.double(32)) {",
                "    {\n        let memory = core.region(octets.double(32)) else {\n            core.print(octets.double(9))\n        }",
            )
            .replace(
                "    } else {\n        core.print(octets.double(9))\n    }",
                "    }",
            )
            .replace("core.construct(frame.forward(", "core.construct(memory, frame.forward(");
        fixture.write("telemetry/src/main.holo", source);
        fixture.lock();
        assert!(
            success(
                fixture
                    .metadata_command("check")
                    .arg("--locked")
                    .output()
                    .unwrap()
            )
            .is_empty()
        );

        // Preserve identical provider layouts and recovery in both forms.
        // Compare raw modules before running the implicit one under both modes.
        for layout in ["wide", "derived", "compact"] {
            fixture.write("telemetry/src/main.holo", &explicit);
            let control = success(
                fixture
                    .command("emit-llvm")
                    .args(["--locked", "--layout", layout])
                    .output()
                    .unwrap(),
            );
            fixture.write("telemetry/src/main.holo", source);
            let automatic = success(
                fixture
                    .command("emit-llvm")
                    .args(["--locked", "--layout", layout])
                    .output()
                    .unwrap(),
            );
            assert_eq!(automatic, control, "implicit backing changed {layout} LLVM");
            // The output witnesses rejection, retry, and the owner continuation.
            for optimization in [None, Some("--release")] {
                assert_eq!(
                    success(
                        fixture
                            .command("run")
                            .args(["--locked", "--layout", layout])
                            .args(optimization)
                            .output()
                            .unwrap(),
                    ),
                    b"8194\n104\n1\n99\n"
                );
            }
        }

        // Even the unexecuted acquisition recovery uses package visibility
        // and reports its original source file on a bad qualified call.
        fixture.write(
            "telemetry/src/main.holo",
            &source.replace("octets.double(9)", "octets.missing(9)"),
        );
        let output = fixture
            .metadata_command("check")
            .arg("--locked")
            .output()
            .unwrap();
        assert_terms(&rejection(output), &["main.holo", "wire.octets", "missing"]);
    }

    /// The package boundary resolves both types and the forwarding callee
    /// without changing final-slot LLVM. The outputs observe data, stride,
    /// named rejections, and exhaustion after a private field write.
    #[test]
    fn package_aggregate_result_preserves_typed_publication() {
        let fixture = typed_fixture();
        fixture.write("wire/src/frame/forward.holo", TYPED_FORWARD);
        let forwarded = TYPED_PROGRAM.replace("frame.admit(", "frame.forward(");
        fixture.write("telemetry/src/main.holo", &forwarded);
        fixture.lock();
        assert!(
            success(
                fixture
                    .metadata_command("check")
                    .arg("--locked")
                    .output()
                    .unwrap()
            )
            .is_empty()
        );
        let tools = NativeToolchain::discover().unwrap();
        let flat_path = fixture.path("typed-flat.holo");
        let flat_definitions = [HEADER, FRAME_ERROR, TYPED_MAKE, TYPED_FORWARD]
            .map(|source| {
                source
                    .strip_prefix("namespace wire.frame\npublic ")
                    .unwrap()
            })
            .concat();
        // Leave room for the third record's header and first field only.
        // That attempt must fail after a private write in every layout.
        for (mode, size, header) in [("wide", 24, 8), ("derived", 16, 0), ("compact", 16, 0)] {
            let committed = 2 * size;
            let capacity = committed + header + 8;
            let source = forwarded.replace("core.region(64)", &format!("core.region({capacity})"));
            fixture.write("telemetry/src/main.holo", &source);
            let flat_entry = source
                .strip_prefix("using wire.frame\n")
                .unwrap()
                .replace("frame.", "");
            fixture.write(
                "typed-flat.holo",
                &format!("{flat_definitions}{flat_entry}"),
            );

            let packaged = success(
                fixture
                    .command("emit-llvm")
                    .args(["--locked", "--layout", mode])
                    .output()
                    .unwrap(),
            );
            let flat = success(
                cli()
                    .args(["emit-llvm", "--layout", mode])
                    .arg(&flat_path)
                    .output()
                    .unwrap(),
            );
            assert_eq!(packaged, flat, "package result changed {mode} LLVM");
            let llvm = String::from_utf8(packaged).unwrap();
            let expected = format!(
                "{}\n{size}\n{committed}\n{committed}\n52\n1\n18\n2\n{size}\n",
                u64::MAX
            );
            // Identical LLVM makes one execution per native mode sufficient
            // for both source forms. ASan checks the exact bounded receiver.
            for optimization in ["o0", "o3", "asan"] {
                let binary = fixture.path(&format!("typed-{mode}-{optimization}"));
                match optimization {
                    "asan" => tools.build_sanitized(&llvm, &binary),
                    "o3" => tools.build(&llvm, &binary, true),
                    _ => tools.build(&llvm, &binary, false),
                }
                .unwrap();
                assert_eq!(
                    success(
                        Command::new(binary)
                            .env("ASAN_OPTIONS", "detect_leaks=0")
                            .output()
                            .unwrap()
                    ),
                    expected.as_bytes(),
                    "{mode} {optimization}"
                );
            }
        }
    }

    /// Even unused public library functions cannot expose either private
    /// type, while a private constructor may name both local declarations.
    #[test]
    fn package_aggregate_result_checks_both_components_in_an_entryless_library() {
        let fixture = typed_fixture().into_library();
        let path = "telemetry/src/admission.holo";
        let declarations = [HEADER, FRAME_ERROR, TYPED_MAKE]
            .map(|source| source.strip_prefix("namespace wire.frame\n").unwrap())
            .concat();
        let source = format!("namespace telemetry.admission\n{declarations}");
        fixture.write(path, &source.replace("public ", ""));
        fixture.lock();
        success(
            fixture
                .metadata_command("check")
                .arg("--locked")
                .output()
                .unwrap(),
        );
        // Only one component changes visibility in each witness. There is
        // no calling entry to incidentally catch the unused leaking export.
        for (visible, private) in [
            ("public struct Header", "private struct `Header`"),
            (
                "public error FrameError",
                "private error family `FrameError`",
            ),
        ] {
            fixture.write(
                path,
                &source.replace(visible, visible.strip_prefix("public ").unwrap()),
            );
            let output = fixture
                .metadata_command("check")
                .arg("--locked")
                .output()
                .unwrap();
            assert_terms(
                &rejection(output),
                &["admission.holo", "public", "admit", private],
            );
        }
    }

    /// Equal layouts and case spellings in a second namespace do not replace
    /// the provider's nominal destination schema or rejection family.
    #[test]
    fn package_aggregate_result_rejects_wrong_nominal_schema_and_family() {
        let fixture = typed_fixture();
        let record = HEADER.strip_prefix("namespace wire.frame\n").unwrap();
        let family = FRAME_ERROR.strip_prefix("namespace wire.frame\n").unwrap();
        fixture.write(
            "wire/src/other.holo",
            &format!("namespace wire.other\n{record}{family}"),
        );
        fixture.lock();
        let source = format!("using wire.other\n{TYPED_PROGRAM}");
        // Mutate only the consumer, so both failures use the same locked provider.
        for (original, mismatch, expected) in [
            (
                "frame.Header",
                "other.Header",
                "constructor result does not match the batch schema",
            ),
            (
                "frame.FrameError",
                "other.FrameError",
                "wire.frame.FrameError",
            ),
        ] {
            fixture.write(
                "telemetry/src/main.holo",
                &source.replace(original, mismatch),
            );
            let output = fixture
                .metadata_command("check")
                .arg("--locked")
                .output()
                .unwrap();
            assert_terms(&rejection(output), &["main.holo", mismatch, expected]);
        }
    }

    /// Equal field shapes do not match a receiving record from another
    /// namespace, with or without a different owning package.
    #[test]
    fn equal_layout_records_keep_distinct_namespace_and_package_identities() {
        // Distinct qualifiers avoid rejecting the fixture during namespace
        // resolution, before reaching the intended nominal append check.
        for (namespace, alternate_path) in [
            ("wire.alternate", "wire/src/alternate.holo"),
            ("radio.alternate", "radio/src/alternate.holo"),
        ] {
            let fixture = fixture();
            // The cross-package case adds a second explicit provider.
            if namespace.starts_with("radio.") {
                fixture.write(
                    "radio/holo.toml",
                    &WIRE_MANIFEST.replace("\"wire\"", "\"radio\""),
                );
                fixture.write(
                    "telemetry/holo.toml",
                    &format!("{MANIFEST}radio = {{ path = \"../radio\" }}\n"),
                );
            }
            let alternate = format!(
                "{HEADER}{}",
                MAKE.strip_prefix("namespace wire.frame\n").unwrap()
            )
            .replace("namespace wire.frame", &format!("namespace {namespace}"));
            fixture.write(alternate_path, &alternate);
            let source = PROGRAM
                .replace(
                    "using wire.frame\n",
                    &format!("using wire.frame\nusing {namespace}\n"),
                )
                .replace("frame.make(52, 1)", "alternate.make(52, 1)");
            fixture.write("telemetry/src/main.holo", &source);
            fixture.lock();
            let actual = format!("{namespace}.Header");
            let diagnostic = ["main.holo", "batch schema", "wire.frame.Header", &actual];
            let output = fixture
                .metadata_command("check")
                .arg("--locked")
                .output()
                .unwrap();
            assert_terms(&rejection(output), &diagnostic);

            // Reject before toolchain discovery and leave the requested LLVM
            // artifact absent on the same nominal mismatch.
            let artifact = fixture.path("wrong-record.ll");
            let emission = fixture
                .metadata_command("emit-llvm")
                .args(["--locked", "-o"])
                .arg(&artifact)
                .output()
                .unwrap();
            assert_terms(&rejection(emission), &diagnostic);
            assert!(!artifact.exists());
        }
    }

    /// A private schema remains visible to sibling files but cannot enter a
    /// public constructor result or another namespace's qualified lookup.
    #[test]
    fn private_records_stay_inside_their_namespace() {
        let fixture = Fixture::new().into_library();
        let private = HEADER
            .replace("wire.frame", "telemetry.records")
            .replace("public struct ", "struct ");
        let constructor = MAKE.replace("wire.frame", "telemetry.records");
        // The schema's filename sorts after its constructor on purpose.
        fixture.write("telemetry/src/records/shape.holo", &private);
        fixture.write(
            "telemetry/src/records/make.holo",
            &constructor.replace("public func ", "func "),
        );
        fixture.lock();
        assert!(
            success(
                fixture
                    .metadata_command("check")
                    .arg("--locked")
                    .output()
                    .unwrap()
            )
            .is_empty()
        );

        // Exporting the same unused constructor is invalid even without an
        // application entry or outside caller to try to name its private result.
        fixture.write("telemetry/src/records/make.holo", &constructor);
        let leak = fixture
            .metadata_command("check")
            .arg("--locked")
            .output()
            .unwrap();
        assert_terms(&rejection(leak), &["make.holo", "Header", "private"]);

        let app = self::fixture();
        app.write(
            "wire/src/frame/header.holo",
            &HEADER.replace("public struct ", "struct "),
        );
        app.write(
            "wire/src/frame/make.holo",
            &MAKE.replace("public func ", "func "),
        );
        // No constructor call can mask a private schema lookup at core.batch.
        app.write(
            "telemetry/src/main.holo",
            "using wire.frame
func main returns u64 {
    let memory = core.region(32) else { return 0 }
    let headers = core.batch(memory, frame.Header)
    return 0
}
",
        );
        app.lock();
        let access = app
            .metadata_command("check")
            .arg("--locked")
            .output()
            .unwrap();
        assert_terms(
            &rejection(access),
            &["main.holo", "wire.frame", "has no public struct `Header`"],
        );
    }

    /// Duplicate records must retain both namespace contributor locations.
    #[test]
    fn duplicate_fixed_record_reports_both_source_files() {
        let fixture = fixture();
        fixture.write("wire/src/frame/other_header.holo", HEADER);
        fixture.lock();
        let output = fixture
            .metadata_command("check")
            .arg("--locked")
            .output()
            .unwrap();
        assert_terms(
            &rejection(output),
            &[
                "duplicate library struct `Header`",
                "header.holo",
                "other_header.holo",
            ],
        );
    }

    /// A public schema supplies compile-time identity, not permission to
    /// write through a readonly reference or reset its live batch.
    #[test]
    fn imported_records_keep_access_and_lifetime_checks() {
        let fixture = fixture();
        fixture.lock();
        // Changing only root source leaves the pinned package unchanged.
        for (operation, reason) in [
            ("retained.kind = 9", "readonly"),
            ("core.reset(headers)", "reference is live"),
        ] {
            fixture.write(
                "telemetry/src/main.holo",
                &PROGRAM.replace("first.kind = 9", operation),
            );
            let output = fixture
                .metadata_command("check")
                .arg("--locked")
                .output()
                .unwrap();
            assert_terms(&rejection(output), &["main.holo", reason, operation]);
        }
    }

    /// The old explicit leaf loader retains a function-only source interface.
    #[test]
    fn public_records_do_not_expand_legacy_leaf_schemas() {
        let fixture = fixture();
        fixture.write(
            "legacy/main.holo",
            "using holo.frame\nfunc main returns u64 { return 0 }\n",
        );
        fixture.write(
            "legacy/library/frame.holo",
            &HEADER.replace("wire.frame", "holo.frame"),
        );
        let output = cli()
            .args(["check", "--library-root"])
            .arg(fixture.path("legacy/library"))
            .arg(fixture.path("legacy/main.holo"))
            .env("HOLO_LLVM_PREFIX", fixture.path("no LLVM installation"))
            .output()
            .unwrap();
        assert_terms(&rejection(output), &["frame.holo", "expected `func`"]);
    }
}

/// Byte-bearing destination results linked from split, locked provider sources.
mod byte_records {
    use super::{Fixture, assert_terms, cli, hosted_stream, rejection, success};
    use holo::{
        driver::NativeToolchain, frontend::parse_experimental_package, layout::LayoutMode, llvm,
        package,
    };
    use std::fs;
    use std::process::Command;

    /// The standalone checked decoder and recovery witness, also used as the
    /// flat LLVM control. Packaging must preserve their existing semantics.
    const FRAME: &str = include_str!("../../fixtures/typed_frame.holo");

    /// The production hosted example supplies the exact checked decoder body.
    const HOSTED_FRAME: &str = include_str!("../../fixtures/typed_refill_frame.holo");

    /// The stream decoder accepts a separately tracked logical filled prefix.
    const STAGED_FRAME: &str = include_str!("../../fixtures/typed_stream_reader.holo");

    /// The same entry handles a native short read or a capacity failure followed
    /// by a complete retry. Its receiver has room for exactly two equal frames.
    const HOSTED_ENTRY: &str = r#"
// One initialized slot receives all wire bytes, leaving a stale suffix on a short read.
struct Slot { wire Bytes }

func wire_slot with noblock returns Slot {
    return Slot { wire: core.bytes(7, 0xEE) }
}

// Observe the logical extent of the native view independently of the decoder.
func received_extent with noalloc, noblock arg input Bytes as readonly reference returns u64 {
    return core.len(input)
}

func main returns u64 {
    let slot_memory = core.region(64) else { core.assert(false); return 0 }
    let slots = core.batch(slot_memory, Slot)
    let initialized = core.append(slots, wire_slot())
    core.assert(initialized == 0)
    let slot = core.borrow_readwrite(slots, 0)

    let stride = @STRIDE@
    let capacity = core.mul_wrapping(2, stride)
    let memory = core.region(capacity) else { core.assert(false); return 0 }
    let frames = core.batch(memory, Received)
    // The first success publishes its record before the selected arm runs.
    refill slot.wire, 6 as input {
        core.assert(received_extent(input) == 6)
        let published = core.append(frames, receive(1, input))
            on ok then core.len(frames)
            on error then 0
        core.assert(published == 1)
    } else status { core.assert(false); return 0 }
    let first = core.borrow_readonly(frames, 0)
    let address = core.address(first)
    core.assert(core.used(memory) == stride)

    // The host may produce four bytes for this request. Otherwise the seven-byte
    // frame is valid but its fourth payload byte exceeds the destination capacity.
    var failure = 0
    refill slot.wire, 7 as input {
        let outcome = core.append(frames, receive(2, input))
            on ok then 0
            on Rejected Truncated(required) then required
            on Full then core.used(memory)
            on error then 0
        failure = outcome
        if failure == 6 {
            core.assert(received_extent(input) == 4)
        } else {
            core.assert(failure == stride)
            core.assert(received_extent(input) == 7)
        }
    } else status { core.assert(false); return 0 }
    core.print(failure)
    core.assert(core.len(frames) == 1)
    core.assert(core.used(memory) == stride)
    core.assert(core.address(first) == address)
    core.assert(first.channel == 1)
    core.assert(first.frame.sequence == 7)
    core.assert(core.len(first.frame.payload) == 3)
    core.assert(first.frame.payload[0] == 0xAA)
    core.assert(first.frame.payload[1] == 0xBB)
    core.assert(first.frame.payload[2] == 0xCC)

    if failure == 6 {
        // The four-byte read overwrote the first payload byte but kept both
        // old tail bytes. Neither tail byte belongs to its fresh view.
        core.assert(slot.wire[3] == 0xDD)
        core.assert(slot.wire[4] == 0xBB)
        core.assert(slot.wire[5] == 0xCC)
    } else {
        // Refill the same slot and publish exactly in the failed record's place.
        refill slot.wire, 6 as input {
            core.assert(received_extent(input) == 6)
            let published = core.append(frames, receive(3, input))
                on ok then core.len(frames)
                on error then 0
            core.assert(published == 2)
        } else status { core.assert(false); return 0 }
        core.assert(core.used(memory) == capacity)
        let second = core.borrow_readonly(frames, 1)
        core.assert(core.sub_wrapping(core.address(second), address) == stride)
        core.assert(second.channel == 3)
        core.assert(core.len(second.frame.payload) == 3)
        core.assert(second.frame.payload[0] == 0x71)
        core.assert(second.frame.payload[1] == 0x72)
        core.assert(second.frame.payload[2] == 0x73)
        core.print(second.frame.sequence)
        core.print(second.frame.payload[0])
    }

    // EOF publishes no view and leaves the earlier payload owned and readable.
    refill slot.wire, 7 as input {
        core.assert(false)
    } else status {
        core.assert(status == 0)
        core.print(status)
    }
    core.assert(core.address(first) == address)
    core.print(first.frame.sequence)
    core.print(first.frame.payload[0])
    core.print(first.frame.payload[1])
    core.print(first.frame.payload[2])
    return 0
}
"#;

    /// Drive the locked decoder with one full and one short native read.
    /// Each scoped success owns its payload until the selected arm ends.
    const HOSTED_SCOPED_ENTRY: &str = r#"
// Initialize the complete receiving slot once so a short refill leaves a stale suffix.
struct Slot { wire Bytes }

// Supply the six initialized backing bytes before either read.
func wire_slot with noblock returns Slot {
    return Slot { wire: core.bytes(6, 0xEE) }
}

// Observe the exact fresh extent independently of the provider's decoder.
func received_extent with noalloc, noblock arg input Bytes as readonly reference returns u64 {
    return core.len(input)
}

// Use real hosted input, then complete only after the nondata EOF outcome.
func main returns u64 {
    let slot_memory = core.region(64) else { core.assert(false); return 0 }
    let slots = core.batch(slot_memory, Slot)
    let initialized = core.append(slots, wire_slot())
    core.assert(initialized == 0)
    let slot = core.borrow_readwrite(slots, 0)
    let memory = core.region(64) else { core.assert(false); return 0 }
    let start = core.used(memory)
    var first_address = 0

    // Construct a full frame directly in the scoped tail. Its copied payload
    // remains readable when the caller overwrites the original wire byte.
    refill slot.wire, 6 as input {
        core.construct(memory, receive(1, input))
            on ok(record) then {
                core.assert(core.used(memory) > start)
                first_address = core.address(record)
                slot.wire[3] = 0xFE
                core.assert(record.channel == 1)
                core.assert(core.len(record.frame.payload) == 3)
                core.print(record.frame.sequence)
                core.print(record.frame.payload[0])
            }
            on error then { core.assert(false) }
    } else status { core.assert(false) }
    core.assert(core.used(memory) == start)

    // The next frame declares three payload bytes but only four bytes arrived.
    // The original tail is still readable through the full initialized slot.
    refill slot.wire, 6 as input {
        core.assert(received_extent(input) == 4)
        core.assert(slot.wire[4] == 0xBB)
        core.assert(slot.wire[5] == 0xCC)
        core.construct(memory, receive(2, input))
            on ok then { core.assert(false) }
            on Rejected Truncated(required) then {
                core.assert(core.used(memory) == start)
                core.print(required)
                // Reinterpret the four fresh bytes as a one-byte preview by
                // changing the live slot's length byte. The readonly view
                // keeps the same extent and observes the caller's mutation.
                slot.wire[2] = 1
                core.construct(memory, receive(3, input))
                    on ok(preview) then {
                        core.assert(core.used(memory) > start)
                        core.assert(core.address(preview) == first_address)
                        core.assert(preview.channel == 3)
                        core.assert(core.len(preview.frame.payload) == 1)
                        core.print(preview.frame.sequence)
                        core.print(preview.frame.payload[0])
                    }
                    on error then { core.assert(false) }
                core.assert(core.used(memory) == start)
            }
            on error then { core.assert(false) }
    } else status { core.assert(false) }
    core.assert(core.used(memory) == start)

    // End exposes no positive view and does not advance the scoped tail.
    refill slot.wire, 6 as input {
        core.assert(false)
    } else status {
        core.assert(status == 0)
        core.assert(core.used(memory) == start)
        core.print(status)
    }
    return 0
}
"#;

    /// Exercise one proof-selected staging site with native wide or two-byte
    /// deliveries. The fixture reuses both slots through rejected appends.
    const STAGED_ENTRY: &str = r#"
struct Slot { wire Bytes }

func staging_slot with noblock returns Slot {
    return Slot { wire: core.bytes(132, 0xEE) }
}

func received_extent with noalloc, noblock arg input Bytes as readonly reference returns u64 {
    return core.len(input)
}

func setting with noalloc, noblock arg input Bytes as readonly reference returns u64 {
    return input[0]
}

func main returns u64 {
    // Both physical extents come from checked construction in distinct backing.
    let read_memory = core.region(140) else { core.assert(false); return 0 }
    let read_slots = core.batch(read_memory, Slot)
    let read_seed = core.append(read_slots, staging_slot())
    core.assert(read_seed == 0)
    let incoming = core.borrow_readwrite(read_slots, 0)
    let pending_memory = core.region(140) else { core.assert(false); return 0 }
    let pending_slots = core.batch(pending_memory, Slot)
    let pending_seed = core.append(pending_slots, staging_slot())
    core.assert(pending_seed == 0)
    let pending = core.borrow_readwrite(pending_slots, 0)

    var read_limit = 0
    refill incoming.wire, 1 as config {
        read_limit = setting(config)
    } else status { core.assert(false); return 0 }
    if read_limit != 2 { core.assert(read_limit == 132) }

    // Derived64 stores channel, sequence, length, and 128 payload bytes.
    let memory = core.region(304) else { core.assert(false); return 0 }
    let frames = core.batch(memory, Received)
    var filled = 0
    var required = 3
    var failure = 0
    var first_address = 0
    loop {
        let remaining = core.sub_wrapping(required, filled)
        var request = read_limit
        if remaining < request { request = remaining }
        refill incoming.wire, request as fresh {
            var position = 0
            let received = received_extent(fresh)
            let room = core.sub_wrapping(required, filled)
            var chunk = received
            if room < chunk { chunk = room }
            core.assert(chunk == received)
            core.assert(chunk <= core.sub_wrapping(132, position))
            // A wide body reaches the outlined leaf; the same site also stages
            // one- and two-byte reads without consulting any stale input bytes.
            for index in core.range(chunk) {
                pending.wire[core.add_wrapping(filled, index)] = incoming.wire[core.add_wrapping(position, index)]
            }
            position = core.add_wrapping(position, chunk)
            filled = core.add_wrapping(filled, chunk)
            core.assert(position == received)

            let count = core.len(frames)
            let marker = core.used(memory)
            let outcome = core.append(frames, receive(pending.wire, filled))
                on ok then 0
                on Rejected Truncated(needed) then needed
                on Full then core.add_wrapping(256, core.used(memory))
                on error then 4096
            var terminal = 0
            if outcome == 0 {
                core.assert(core.len(frames) == core.add_wrapping(count, 1))
                if count == 0 {
                    let published = core.borrow_readonly(frames, 0)
                    first_address = core.address(published)
                }
                terminal = 1
            } else {
                // Every rejected append, including Full, restores publication.
                core.assert(core.len(frames) == count)
                core.assert(core.used(memory) == marker)
                if outcome > 132 {
                    core.assert(outcome == 408)
                    core.assert(failure == 0)
                    failure = core.sub_wrapping(outcome, 256)
                    terminal = 1
                }
            }
            if terminal == 1 {
                filled = 0
                required = 3
            } else {
                core.assert(outcome > filled)
                core.assert(outcome <= 132)
                required = outcome
            }
        } else status {
            core.assert(status == 0)
            let count = core.len(frames)
            if filled != 0 {
                core.assert(required == 131)
                core.assert(failure == 0)
                failure = required
                core.assert(count == 1)
                core.assert(core.used(memory) == 152)
            } else {
                core.assert(failure == 152)
                core.assert(count == 2)
                core.assert(core.used(memory) == 304)
                let retry = core.borrow_readonly(frames, 1)
                core.assert(core.sub_wrapping(core.address(retry), first_address) == 152)
                core.assert(retry.channel == 1)
                core.assert(retry.frame.sequence == 9)
                core.assert(core.len(retry.frame.payload) == 128)
                for index in core.range(128) {
                    core.assert(retry.frame.payload[index] == core.add_wrapping(128, index))
                }
            }
            let first = core.borrow_readonly(frames, 0)
            core.assert(core.address(first) == first_address)
            core.assert(first.channel == 1)
            core.assert(first.frame.sequence == 7)
            core.assert(core.len(first.frame.payload) == 128)
            for index in core.range(128) {
                core.assert(first.frame.payload[index] == core.add_wrapping(1, index))
            }
            core.print(failure)
            core.print(count)
            core.print(first.frame.sequence)
            return 0
        }
    }
}
"#;

    /// Install one decoder's declarations in separate provider source files.
    /// Keep nested decoding private and expose the outer typed constructor.
    fn install_decoder(fixture: &Fixture, error: &str, frame: &str, received: &str, decoder: &str) {
        fixture.write(
            "wire/src/frame/errors.holo",
            &format!("namespace wire.frame\npublic error FrameError{error}"),
        );
        fixture.write(
            "wire/src/frame/frame.holo",
            &format!("namespace wire.frame\npublic struct Frame{frame}"),
        );
        fixture.write(
            "wire/src/frame/received.holo",
            &format!("namespace wire.frame\npublic struct Received{received}"),
        );
        fixture.write(
            "wire/src/frame/decoder.holo",
            &format!(
                "namespace wire.frame\nfunc decode_frame{}",
                decoder.replace("func receive", "public func receive")
            ),
        );
    }

    /// Name the provider's nominal schema, constructor and error cases in the
    /// caller, preserving the standalone control's operations and argument order.
    fn install_caller(fixture: &Fixture, entry: &str) {
        fixture.write(
            "telemetry/src/main.holo",
            &format!(
                "using wire.frame\n{}",
                entry
                    .replace(", Received)", ", frame.Received)")
                    .replace("receive(", "frame.receive(")
                    .replace("on Rejected ", "on Rejected frame.FrameError.")
            ),
        );
    }

    /// Move the existing decoder and public types into one provider namespace.
    /// Keep its standalone recovery witness as the caller and flat control.
    fn install_frame(fixture: &Fixture, source: &str) {
        let (declarations, entry) = source.split_once("func main").unwrap();
        let (declarations, decoder) = declarations.split_once("func decode_frame").unwrap();
        let (prefix, declarations) = declarations.split_once("struct Wire").unwrap();
        let (_, error) = prefix.split_once("error FrameError").unwrap();
        let (wire, declarations) = declarations.split_once("struct Frame").unwrap();
        let (frame, declarations) = declarations.split_once("struct Received").unwrap();
        let (received, empty_wire) = declarations.split_once("func empty_wire").unwrap();
        install_decoder(fixture, error, frame, received, decoder);
        install_caller(
            fixture,
            &format!("struct Wire{wire}func empty_wire{empty_wire}func main{entry}"),
        );
    }

    /// Install the real hosted decoder and return the same unqualified
    /// declarations for the flat control. The test owns the entry and the slot.
    fn install_hosted_decoder(fixture: &Fixture) -> String {
        let (_, declarations) = HOSTED_FRAME.split_once("error FrameError").unwrap();
        let (error, declarations) = declarations.split_once("struct Slot").unwrap();
        let (_, declarations) = declarations.split_once("struct Frame").unwrap();
        let (frame, declarations) = declarations.split_once("struct Received").unwrap();
        let (received, declarations) = declarations.split_once("func wire_slot").unwrap();
        let (_, declarations) = declarations.split_once("func decode_frame").unwrap();
        let (decoder, _) = declarations.split_once("func main").unwrap();
        install_decoder(fixture, error, frame, received, decoder);
        format!(
            "error FrameError{error}struct Frame{frame}struct Received{received}func decode_frame{decoder}"
        )
    }

    /// Install the production staged decoder with its private nested producer.
    /// The test owns only the read loop and independent byte observations.
    fn install_staged_decoder(fixture: &Fixture) {
        let (_, declarations) = STAGED_FRAME.split_once("error FrameError").unwrap();
        let (error, declarations) = declarations.split_once("struct Slot").unwrap();
        let (_, declarations) = declarations.split_once("struct Frame").unwrap();
        let (frame, declarations) = declarations.split_once("struct Received").unwrap();
        let (received, declarations) = declarations.split_once("func make_slot").unwrap();
        let (_, declarations) = declarations.split_once("func decode_frame").unwrap();
        let (decoder, _) = declarations.split_once("func checksum").unwrap();
        install_decoder(fixture, error, frame, received, decoder);
    }

    /// Add a larger admitted frame to the original recovery example. The
    /// bounded receiver can fit the three-byte retry exactly, so the four-byte
    /// payload fails after private parent and sequence writes. Both forms use
    /// the same operations and the independent physical stride for the layout.
    fn bounded_frame_source(stride: u64) -> String {
        let capacity = 2 * stride + 3;
        let (before_full, old_full) = FRAME
            .split_once("    // This capacity fits an empty frame.")
            .unwrap();
        let (_, retry) = old_full.split_once("    // Retry after").unwrap();
        let failed = "
    // Fail after the parent and sequence have reserved bytes. Observe the
    // original marker inside the handler before retrying into the same slot.
    let full = core.append(frames, receive(1, larger.data))
        on ok then 0
        on Full then core.used(memory)
        on error then 0
    core.assert(full == saved)
    core.assert(core.len(frames) == 1)
    core.assert(core.address(first) == address)
";
        let before_full = before_full
            .replace(
                "    let empty = core.borrow_readonly(wires, 3)",
                "    let larger_status = core.append(wires, empty_wire(7))
    core.assert(larger_status == 0)
    let larger = core.borrow_readwrite(wires, 4)
    larger.data[0] = 0x48
    larger.data[1] = 9
    larger.data[2] = 4
    let empty = core.borrow_readonly(wires, 3)",
            )
            .replace(
                "let memory = core.region(256)",
                &format!("let memory = core.region({capacity})"),
            )
            .replace(
                "let saved = core.used(memory)",
                &format!("let saved = core.used(memory)\n    core.assert(saved == {stride})"),
            );
        let retry = retry.replace(
            "let second = core.borrow_readonly(frames, 1)",
            "let second = core.borrow_readonly(frames, 1)
    core.assert(core.sub_wrapping(core.address(second), address) == saved)",
        );
        format!("{before_full}{failed}    // Retry after{retry}")
    }

    /// A real package decoder propagates nested rejection and restores a
    /// partly initialized destination before the caller's selected handler.
    /// Native execution observes the retained record, ownership of copied
    /// bytes after input mutation, and successful exact-slot retry.
    #[test]
    fn package_byte_aggregate_preserves_nested_rollback_and_owned_payload() {
        let fixture = Fixture::new();
        let tools = NativeToolchain::discover().unwrap();
        let flat_path = fixture.path("byte-flat.holo");
        // The empty nested frame's physical stride comes from the respective
        // record headers and length widths. Package and flat IR must match.
        for (mode, stride) in [("wide", 40), ("derived", 24), ("compact", 20)] {
            let source = bounded_frame_source(stride);
            install_frame(&fixture, &source);
            fixture.lock();
            fixture.write("byte-flat.holo", &source);
            let package = success(
                fixture
                    .command("emit-llvm")
                    .args(["--locked", "--layout", mode])
                    .output()
                    .unwrap(),
            );
            let flat = success(
                cli()
                    .args(["emit-llvm", "--layout", mode])
                    .arg(&flat_path)
                    .output()
                    .unwrap(),
            );
            assert_eq!(
                package, flat,
                "{mode}: package byte constructor changed LLVM"
            );
            let llvm = String::from_utf8(package).unwrap();
            // Identical LLVM permits one native execution per variant. The
            // three output scalars are the exact extent, sequence, and last byte.
            for optimization in ["o0", "o3", "asan"] {
                let executable = fixture.path(&format!("byte-{mode}-{optimization}"));
                match optimization {
                    "asan" => tools.build_sanitized(&llvm, &executable),
                    "o3" => tools.build(&llvm, &executable, true),
                    _ => tools.build(&llvm, &executable, false),
                }
                .unwrap();
                let result = Command::new(executable)
                    .env("ASAN_OPTIONS", "detect_leaks=0")
                    .output()
                    .unwrap();
                assert_eq!(success(result), b"6\n8\n204\n", "{mode} {optimization}");
            }
        }
    }

    /// Send real file and stream prefixes through a locked provider's nested
    /// decoder. A short native read exposes fewer bytes than requested. A full
    /// read can still fail capacity and leave the next slot ready for retry.
    #[test]
    fn package_hosted_refill_preserves_published_prefix_and_retries() {
        // Both inputs publish the same seed. One ends during its second frame.
        // The other supplies a too-large complete frame and a valid exact fit.
        const SHORT: &[u8] = &[0x48, 7, 3, 0xAA, 0xBB, 0xCC, 0x48, 8, 3, 0xDD];
        const RETRY: &[u8] = &[
            0x48, 7, 3, 0xAA, 0xBB, 0xCC, 0x48, 8, 4, 0x11, 0x22, 0x33, 0x44, 0x48, 9, 3, 0x71,
            0x72, 0x73,
        ];
        let fixture = Fixture::new();
        let tools = NativeToolchain::discover().unwrap();
        let decoder = install_hosted_decoder(&fixture);
        fixture.lock();
        let flat_path = fixture.path("hosted-flat.holo");
        let short_path = fixture.path("short-wire");
        let retry_path = fixture.path("retry-wire");
        fs::write(&short_path, SHORT).unwrap();
        fs::write(&retry_path, RETRY).unwrap();

        // These strides describe a published three-byte payload at each layout.
        // Changing only the caller leaves the locked provider's source fixed.
        for (mode, stride) in [("wide", 43), ("derived", 27), ("compact", 23)] {
            let entry = HOSTED_ENTRY.replace("@STRIDE@", &stride.to_string());
            install_caller(&fixture, &entry);
            fixture.write("hosted-flat.holo", &format!("{entry}{decoder}"));
            let package = success(
                fixture
                    .command("emit-llvm")
                    .args(["--locked", "--layout", mode])
                    .output()
                    .unwrap(),
            );
            let flat = success(
                cli()
                    .args(["emit-llvm", "--layout", mode])
                    .arg(&flat_path)
                    .output()
                    .unwrap(),
            );
            assert_eq!(package, flat, "{mode}: package hosted refill changed LLVM");
            let llvm = String::from_utf8(package).unwrap();
            assert_eq!(
                llvm.matches("call i64 @holo_refill_experimental(").count(),
                4
            );

            // Hand-derived expected output observes the typed required extent
            // or restored marker, optional retry, EOF, and the first frame.
            let retry_output = format!("{stride}\n9\n113\n0\n7\n170\n187\n204\n");
            let cases = [
                (SHORT, &short_path, b"6\n0\n7\n170\n187\n204\n".as_slice()),
                (RETRY, &retry_path, retry_output.as_bytes()),
            ];
            // Run both input kinds against the same IR in each native mode.
            for optimization in ["o0", "o3", "asan"] {
                let executable = fixture.path(&format!("hosted-{mode}-{optimization}"));
                match optimization {
                    "asan" => tools.build_sanitized(&llvm, &executable),
                    "o3" => tools.build(&llvm, &executable, true),
                    _ => tools.build(&llvm, &executable, false),
                }
                .unwrap();
                for (wire, path, expected) in cases {
                    let file = success(
                        Command::new(&executable)
                            .arg("file")
                            .arg(path)
                            .output()
                            .unwrap(),
                    );
                    assert_eq!(file, expected, "{mode} {optimization} file {wire:02x?}");
                    let stream = hosted_stream::read_stream(&executable, wire, true);
                    assert_eq!(
                        stream.stdout, expected,
                        "{mode} {optimization} stream {wire:02x?}"
                    );
                }
            }
        }
    }

    /// Pass the native prefix through an imported constructor to a private
    /// nested decoder. Package and flat forms must share the checked tail ABI,
    /// including rejection after the parent write and retry from its handler.
    #[test]
    fn package_hosted_scoped_construct_retries_the_fresh_prefix() {
        const WIRE: &[u8] = &[0x48, 7, 3, 0xAA, 0xBB, 0xCC, 0x48, 8, 3, 0xDD];
        // First sequence and owned byte, required extent, preview sequence and
        // owned byte, then EOF. These values come directly from the wire.
        const EXPECTED: &[u8] = b"7\n170\n6\n8\n221\n0\n";
        let fixture = Fixture::new();
        let decoder = install_hosted_decoder(&fixture);
        install_caller(&fixture, HOSTED_SCOPED_ENTRY);
        fixture.lock();
        assert!(
            success(
                fixture
                    .metadata_command("check")
                    .arg("--locked")
                    .output()
                    .unwrap()
            )
            .is_empty()
        );
        let flat = fixture.path("hosted-scoped-flat.holo");
        fixture.write(
            "hosted-scoped-flat.holo",
            &format!("{HOSTED_SCOPED_ENTRY}{decoder}"),
        );
        let input = fixture.path("hosted-scoped-wire");
        fs::write(&input, WIRE).unwrap();
        let tools = NativeToolchain::discover().unwrap();

        // Cover the marker-based and cursor-based producer paths without
        // changing the provider, lock, entry, or expected behavior.
        for mode in ["wide", "derived", "compact"] {
            let package = success(
                fixture
                    .command("emit-llvm")
                    .args(["--locked", "--layout", mode])
                    .output()
                    .unwrap(),
            );
            let flat_ir = success(
                cli()
                    .args(["emit-llvm", "--layout", mode])
                    .arg(&flat)
                    .output()
                    .unwrap(),
            );
            assert!(
                package == flat_ir,
                "{mode}: scoped refill changed package LLVM"
            );
            let llvm = String::from_utf8(package).unwrap();

            // Both descriptor kinds execute the same checked LLVM. ASan
            // checks the caller's bounded slot and each constructed record.
            for optimization in ["o0", "o3", "asan"] {
                let executable = fixture.path(&format!("hosted-scoped-{mode}-{optimization}"));
                match optimization {
                    "asan" => tools.build_sanitized(&llvm, &executable),
                    "o3" => tools.build(&llvm, &executable, true),
                    _ => tools.build(&llvm, &executable, false),
                }
                .unwrap();
                let file = success(
                    Command::new(&executable)
                        .arg("file")
                        .arg(&input)
                        .output()
                        .unwrap(),
                );
                assert_eq!(file, EXPECTED, "{mode} {optimization} file");
                let stream = hosted_stream::read_stream(&executable, WIRE, true);
                assert_eq!(stream.stdout, EXPECTED, "{mode} {optimization} stream");
            }
        }
    }

    /// Verify the outlined staging leaf in a locked application, then run real
    /// wide and two-byte deliveries through its nested typed decoder. The same
    /// witness checks rollback on truncation and Full, plus exact-slot retry.
    #[test]
    fn package_outlined_staging_preserves_hosted_rollback_and_reuse() {
        let fixture = Fixture::new();
        install_staged_decoder(&fixture);
        install_caller(&fixture, STAGED_ENTRY);
        fixture.lock();
        let graph = package::load(&fixture.path("telemetry/holo.toml"), true).unwrap();
        let project = parse_experimental_package(&graph).unwrap();
        let checked = project
            .program()
            .check()
            .unwrap_or_else(|error| panic!("{}", project.render(&error)));
        // The selected proof and its private call are required even on a host
        // that cannot execute this macOS arm64-specific lowering.
        let selected = llvm::emit_with_outlined_bulk_staging(
            &checked,
            "arm64-apple-macosx14.0.0",
            LayoutMode::Derived64,
        )
        .unwrap();
        assert_eq!(
            selected
                .matches("Holo constructed slot cursor proof")
                .count(),
            1
        );
        assert_eq!(
            selected
                .matches("call preserve_mostcc void @holo.stage.bulk.transfer(")
                .count(),
            1
        );
        assert_eq!(
            selected
                .matches("define internal preserve_mostcc void @holo.stage.bulk.transfer(")
                .count(),
            1
        );

        let tools = NativeToolchain::discover().unwrap();
        let candidate = llvm::emit_with_outlined_bulk_staging(
            &checked,
            tools.target_triple(),
            LayoutMode::Derived64,
        )
        .unwrap();
        let native = success(
            fixture
                .command("emit-llvm")
                .args(["--locked", "--layout", "derived"])
                .output()
                .unwrap(),
        );
        assert!(native == candidate.as_bytes(), "locked CLI staging differs");
        let native = String::from_utf8(native).unwrap();
        let mut cases = Vec::new();
        // The first setting byte caps native reads. Wide bodies reach the bulk
        // call and the other mode consumes the same frames in tiny fragments.
        for limit in [2, 132] {
            let mut first = vec![limit, 0x48, 7, 128];
            first.extend(1..=128);
            let mut short = first.clone();
            short.extend([0x48, 8, 128]);
            short.extend([0xEE; 70]);
            let mut retry = first;
            retry.extend([0x48, 8, 129]);
            retry.extend([0xEE; 129]);
            retry.extend([0x48, 9, 128]);
            retry.extend(128..=255);
            // The hand-written outcomes encode typed required extent or the
            // restored marker, then the published count and first sequence.
            for (name, wire, expected) in [
                ("short", short, b"131\n1\n7\n".as_slice()),
                ("retry", retry, b"152\n2\n7\n".as_slice()),
            ] {
                let path = fixture.path(&format!("{name}-{limit}-wire"));
                fs::write(&path, &wire).unwrap();
                cases.push((path, wire, expected));
            }
        }

        // Three independent native builds use the same checked package LLVM.
        for optimization in ["o0", "o3", "asan"] {
            let executable = fixture.path(&format!("staged-hosted-{optimization}"));
            match optimization {
                "asan" => tools.build_sanitized(&native, &executable),
                "o3" => tools.build(&native, &executable, true),
                _ => tools.build(&native, &executable, false),
            }
            .unwrap();
            // Each transport begins with an independent copy of the same input.
            for (path, wire, expected) in &cases {
                let file = Command::new(&executable)
                    .arg("file")
                    .arg(path)
                    .output()
                    .unwrap();
                assert!(file.stderr.is_empty(), "{optimization} file {path:?}");
                assert_eq!(success(file), *expected, "{optimization} file {path:?}");
                let stream = hosted_stream::read_stream(&executable, wire, true);
                assert_eq!(stream.stdout, *expected, "{optimization} stream {path:?}");
                assert!(stream.stderr.is_empty(), "{optimization} stream {path:?}");
            }
        }
    }

    /// Entryless checks resolve later declarations and reject private or
    /// unresolved nested types before any constructor or native build.
    #[test]
    fn package_inline_fields_require_local_types_and_respect_visibility() {
        let fixture = Fixture::new().into_library();
        fixture.write(
            "telemetry/src/message/body.holo",
            "namespace telemetry.message\npublic struct Body { payload Bytes }\n",
        );
        let envelope = "namespace telemetry.message\npublic struct Envelope { body Body }\n";
        fixture.write("telemetry/src/message/a_envelope.holo", envelope);
        fixture.lock();
        success(
            fixture
                .metadata_command("check")
                .arg("--locked")
                .output()
                .unwrap(),
        );
        // A different namespace cannot supply a bare field name even when
        // both namespaces participate in the same flattened library check.
        fixture.write(
            "telemetry/src/other.holo",
            "namespace telemetry.other\npublic struct Missing { payload Bytes }\n",
        );
        fixture.write(
            "telemetry/src/message/a_envelope.holo",
            &envelope.replace("body Body", "body Missing"),
        );
        let missing = fixture
            .metadata_command("check")
            .arg("--locked")
            .output()
            .unwrap();
        assert_terms(
            &rejection(missing),
            &["a_envelope.holo", "namespace-local struct", "Missing"],
        );
        fixture.write("telemetry/src/message/a_envelope.holo", envelope);
        fixture.write(
            "telemetry/src/message/body.holo",
            "namespace telemetry.message\nstruct Body { payload Bytes }\n",
        );
        let leak = fixture
            .metadata_command("check")
            .arg("--locked")
            .output()
            .unwrap();
        assert_terms(
            &rejection(leak),
            &[
                "a_envelope.holo",
                "public struct `Envelope`",
                "private struct `Body`",
            ],
        );
        // A private envelope can use its own private inline schema.
        fixture.write(
            "telemetry/src/message/a_envelope.holo",
            &envelope.replace("public struct", "struct"),
        );
        success(
            fixture
                .metadata_command("check")
                .arg("--locked")
                .output()
                .unwrap(),
        );
    }
}
