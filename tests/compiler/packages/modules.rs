//! Source-level checks for experimental local libraries.
//!
//! Fresh files exercise the real CLI resolver, checker and native driver. An
//! equivalent single-file program controls for packaging-specific lowering.
//! Compilation and execution check behavior and code shape without timing them.

use holo::driver::{NativeToolchain, TempDir};
use holo::format::format;
use holo::frontend::{parse_experimental, parse_experimental_project};
use holo::layout::LayoutMode;
use holo::llvm;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Borrow one initialized byte, call across the module boundary and print its result.
const ENTRY: &str = "using holo.bytes
struct Slot { data Bytes }
func seed returns Slot { return Slot { data: core.bytes(2, 7) } }
func main returns u64 {
    let memory = core.region(256) else { return 0 }
    let slots = core.batch(memory, Slot)
    let status = core.append(slots, seed())
    core.assert(status == 0)
    let slot = core.borrow_readonly(slots, 0)
    core.print(bytes.first(slot.data))
    return 0
}
";

/// The namespace matches the `using` directive. The public helper uses a private helper
/// and the ordinary checked byte load.
const LIBRARY: &str = "namespace holo.bytes

public func first with noalloc, noblock
    arg input Bytes as readonly reference
    returns u64
{
    return twice(input[0])
}
func twice arg value u64 as readonly copy returns u64 {
    return core.add_wrapping(value, value)
}
";

/// Create an isolated entry/root pair and its UTF-8 library, retaining their owner.
/// Fixture failures abort the test. The small input buffers are host-owned.
fn fixture(entry: &str, library: &str) -> (TempDir, PathBuf, PathBuf) {
    let scratch = TempDir::new().unwrap();
    let path = scratch.path().join("entry.holo");
    let root = scratch.path().join("library root");
    fs::create_dir(&root).unwrap();
    fs::write(&path, entry).unwrap();
    fs::write(root.join("bytes.holo"), library).unwrap();
    (scratch, path, root)
}

/// Prepare a direct experimental CLI invocation with an explicit library root.
/// Launching stays with the caller, which may add mode, environment, or output.
fn command(action: &str, entry: &Path, root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_holo"));
    command
        .args([action, "--library-root"])
        .arg(root)
        .arg(entry);
    command
}

/// Require successful native/CLI completion and decode its UTF-8 stdout.
/// A failing status preserves stderr in the assertion.
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "status {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Require a failed check and return its original diagnostic for path assertions.
fn failure(output: Output) -> String {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    String::from_utf8(output.stderr).unwrap()
}

/// Verify LLVM-free checking and both unoptimized and optimized execution.
/// `build` additionally exercises the persistent artifact through the same resolver.
#[test]
fn local_library_runs_through_standard_commands() {
    let (scratch, entry, root) = fixture(ENTRY, LIBRARY);
    let no_root = Command::new(env!("CARGO_BIN_EXE_holo"))
        .args(["check"])
        .arg(&entry)
        .output()
        .unwrap();
    assert!(failure(no_root).contains("expected `struct` or `func`"));
    let output = command("check", &entry, &root)
        .env("HOLO_LLVM_PREFIX", scratch.path().join("absent"))
        .output()
        .unwrap();
    assert_eq!(success(output), "");

    let debug = command("run", &entry, &root).output().unwrap();
    assert_eq!(success(debug), "14\n");
    let binary = scratch.path().join("worker");
    let release = command("build", &entry, &root)
        .args(["--release", "-o"])
        .arg(&binary)
        .output()
        .unwrap();
    success(release);
    assert_eq!(success(Command::new(binary).output().unwrap()), "14\n");
}

/// Exercise the maintained helper's endian order and explicit low-bit contract
/// through the real native library root, including maximum-width scalar inputs.
#[test]
fn public_octet_join_has_total_low_bit_semantics() {
    let scratch = TempDir::new().unwrap();
    let entry = scratch.path().join("octets.holo");
    fs::write(
        &entry,
        "using holo.bytes
func main returns u64 {
    core.print(bytes.join_low_octets_le(52, 18))
    core.print(bytes.join_low_octets_le(18446744073709551615, 256))
    core.print(bytes.join_low_octets_le(256, 18446744073709551615))
    return 0
}",
    )
    .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../lib/holo");
    let output = command("run", &entry, &root)
        .arg("--release")
        .output()
        .unwrap();
    assert_eq!(success(output), "4660\n255\n65280\n");
}

/// Fail empty or option-like root arguments, including when no directive would
/// otherwise touch the supplied path.
#[test]
fn root_cannot_consume_the_next_option() {
    let (scratch, entry, _) = fixture("func main returns u64 { return 0 }", LIBRARY);
    // Both spellings need an explicit directory value.
    for invalid in ["", "--release"] {
        let output = Command::new(env!("CARGO_BIN_EXE_holo"))
            .args(["check", "--library-root", invalid])
            .arg(&entry)
            .output()
            .unwrap();
        assert!(failure(output).contains("--library-root requires a directory path"));
    }
    fs::create_dir(scratch.path().join("-lib")).unwrap();
    let output = command("check", &entry, Path::new("./-lib"))
        .current_dir(scratch.path())
        .output()
        .unwrap();
    assert_eq!(success(output), "");
}

/// Check that direct source linking emits exactly the same code as placing the
/// same implementations in the entry file, using each supported layout mode.
#[test]
fn library_linking_adds_no_runtime_ir() {
    let (_scratch, entry, root) = fixture(ENTRY, LIBRARY);
    let imported = parse_experimental_project(&entry, ENTRY, &root).unwrap();
    let combined = format!(
        "{}{}",
        ENTRY
            .replace("using holo.bytes\n", "")
            .replace("bytes.first(", "first("),
        LIBRARY
            .strip_prefix("namespace holo.bytes\n")
            .unwrap()
            .replace("public func ", "func ")
    );
    let inline = parse_experimental(&combined).unwrap();
    let imported = imported.program().check().unwrap();
    let inline = inline.check().unwrap();
    let target = NativeToolchain::host_target_triple().unwrap();
    // Packaging must not change byte layouts or introduce lookup/call wrappers.
    for mode in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let module_ir = llvm::emit_with_layout(&imported, target, mode).unwrap();
        let inline_ir = llvm::emit_with_layout(&inline, target, mode).unwrap();
        assert_eq!(module_ir, inline_ir);
    }
}

/// Format module syntax without reading a root or separating the visibility marker
/// from its declaration. Reformatting both files must preserve their newlines.
#[test]
fn source_formatting_preserves_local_module_syntax() {
    let project = format(ENTRY).unwrap();
    assert!(project.starts_with("using holo.bytes\n"));
    assert_eq!(format(&project).unwrap(), project);
    // The directive keeps its path and header boundary when whitespace is normalized.
    let unformatted = ENTRY.replacen("using holo.bytes", "using  holo . bytes", 1);
    assert_eq!(format(&unformatted).unwrap(), project);
    let library = format(LIBRARY).unwrap();
    assert!(library.starts_with("namespace holo.bytes\n"));
    assert!(library.contains("\npublic func first\n"));
    assert_eq!(format(&library).unwrap(), library);

    // A namespace alone or followed only by private functions still identifies
    // the library grammar. No project or public declaration is needed to format.
    assert_eq!(
        format("namespace  holo . bytes\n").unwrap(),
        "namespace holo.bytes\n"
    );
    let private = format("namespace holo.bytes\nfunc hidden returns u64 {return 0}").unwrap();
    assert!(private.contains("\nfunc hidden\n"));
    assert_eq!(format(&private).unwrap(), private);

    // The maintained worker nests the imported call and new outcome scopes.
    let worker = include_str!("../../fixtures/telemetry_worker/src/main.holo");
    let formatted = format(worker).unwrap();
    assert!(formatted.contains("using telemetry.frame\n"));
    assert!(formatted.contains("refill slot.wire, 4096 as chunk {"));
    assert_eq!(format(&formatted).unwrap(), formatted);
}

/// Reject unrelated bindings, nonpublic functions, duplicate directives, and invalid
/// path syntax before native generation. Each case gets an independent root.
#[test]
fn module_visibility_and_resolution_are_checked() {
    let entry = "func main returns u64 { return 0 }";
    let cases = [
        (
            ENTRY.replace("bytes.first", "bytes.twice"),
            "library `holo.bytes` has no public function `twice`",
        ),
        (ENTRY.replace("bytes.first", "first"), "unknown"),
        (
            ENTRY.replace("let slot =", "let bytes ="),
            "shadows an imported namespace",
        ),
        (format!("using holo.missing\n{entry}"), "read local library"),
        (
            format!("using holo.missing\nusing holo.missing\n{entry}"),
            "duplicate `using` directive",
        ),
        (format!("using holo...\n{entry}"), "expected an identifier"),
        (
            format!("using holo.core\n{entry}"),
            "expected a lowercase library leaf name",
        ),
        (
            format!("using holo.true\n{entry}"),
            "expected a lowercase library leaf name",
        ),
        (
            format!("{entry}\nusing holo.bytes"),
            "expected `struct` or `func`",
        ),
    ];
    // Each rejected source retains the specific intended failure in the diagnostic.
    for (source, expected) in cases {
        let (_scratch, entry, root) = fixture(&source, LIBRARY);
        let diagnostic = failure(command("check", &entry, &root).output().unwrap());
        assert!(diagnostic.contains(expected), "{expected}: {diagnostic}");
    }
}

/// Library names are lexically isolated and errors name the original library line.
#[test]
fn library_errors_preserve_their_source_and_scope() {
    let entry_source = "using holo.bytes
func caller returns u64 { return 7 }
func main returns u64 { return 0 }
";
    let library_source = "// first line\npublic func first returns u64 { return caller() }\n";
    let (_scratch, entry, root) = fixture(entry_source, library_source);
    let diagnostic = failure(command("check", &entry, &root).output().unwrap());
    assert!(diagnostic.contains("bytes.holo:2:"), "{diagnostic}");
    assert!(diagnostic.contains("unknown local library function `caller`"));

    // A failure produced after call binding still points at its actual declaration.
    let library_source = "// first line\npublic func first returns u64 { return first() }\n";
    fs::write(root.join("bytes.holo"), library_source).unwrap();
    let diagnostic = failure(command("check", &entry, &root).output().unwrap());
    assert!(diagnostic.contains("bytes.holo:2:"), "{diagnostic}");
    assert!(diagnostic.contains("recursive function graph at `holo.bytes.first`"));
}

/// Reject an identity mismatch at the library header, with both paths visible.
/// The explicit library root and all source files are private to the test.
#[test]
fn library_namespace_must_match_its_using_directive() {
    let library = LIBRARY.replacen("holo.bytes", "holo.other", 1);
    let library = format!("// Library source\n{library}");
    let (_scratch, entry, root) = fixture(ENTRY, &library);
    let diagnostic = failure(command("check", &entry, &root).output().unwrap());
    assert!(diagnostic.contains("bytes.holo:2:"), "{diagnostic}");
    assert!(
        diagnostic
            .contains("namespace `holo.other` does not match `using` directive for `holo.bytes`"),
        "{diagnostic}"
    );
}

/// Reject malformed or misplaced headers in the checker and formatter.
/// Each checker failure must point into its own library at the offending line.
#[test]
fn library_namespace_headers_follow_the_leaf_grammar() {
    let separator = "expected a newline or `;` after the statement";
    let position = "namespace declaration must appear once before library functions";
    let external_namespace = "// Library source\nnamespace other.bytes\n";
    // The formatter recognizes the package grammar without choosing a loader.
    // The explicit legacy leaf resolver still requires the `holo` prefix.
    let (_scratch, entry, root) = fixture(ENTRY, external_namespace);
    let diagnostic = failure(command("check", &entry, &root).output().unwrap());
    assert!(diagnostic.contains("bytes.holo:2:"), "{diagnostic}");
    assert!(diagnostic.contains("expected `holo`"), "{diagnostic}");
    assert_eq!(format(external_namespace).unwrap(), external_namespace);

    // A deeper package namespace is formattable, while the explicitly
    // rooted checker still selects exactly one leaf.
    let nested_namespace = "// Library source\nnamespace holo.bytes.decode\n";
    let (_scratch, entry, root) = fixture(ENTRY, nested_namespace);
    let diagnostic = failure(command("check", &entry, &root).output().unwrap());
    assert!(diagnostic.contains("bytes.holo:2:"), "{diagnostic}");
    assert!(diagnostic.contains(separator), "{diagnostic}");
    assert_eq!(format(nested_namespace).unwrap(), nested_namespace);

    let cases = [
        (
            "namespace holo.bytes func hidden returns u64 { return 0 }\n",
            2,
            separator,
        ),
        ("namespace holo.bytes\nnamespace holo.bytes\n", 3, position),
        (
            "func hidden returns u64 { return 0 }\nnamespace holo.bytes\n",
            3,
            position,
        ),
    ];
    // Each case uses fresh source paths so the diagnostic cannot refer to a
    // previously loaded library, and the formatter resolves no dependencies.
    for (source, line, expected) in cases {
        let library = format!("// Library source\n{source}");
        let (_scratch, entry, root) = fixture(ENTRY, &library);
        let diagnostic = failure(command("check", &entry, &root).output().unwrap());
        assert!(
            diagnostic.contains(&format!("bytes.holo:{line}:")),
            "{diagnostic}"
        );
        assert!(diagnostic.contains(expected), "{diagnostic}");
        assert!(format(&library).is_err(), "{library}");
    }
}

/// Check contextual words as function and local names in a headerless library.
/// Source checking also verifies that retiring the old directive leaves the
/// same word available as a local name, without LLVM discovery.
#[test]
fn module_words_are_contextual() {
    let entry_source = "using holo.bytes
func main returns u64 {
    let public = bytes.namespace(9)
    let namespace = bytes.public(public)
    let import = namespace
    let using = import
    core.print(using)
    return 0
}";
    let library_source =
        "public func public arg value u64 as readonly copy returns u64 { return value }
public func namespace arg value u64 as readonly copy returns u64 { return value }";
    let (scratch, entry, root) = fixture(entry_source, library_source);
    let output = command("check", &entry, &root)
        .env("HOLO_LLVM_PREFIX", scratch.path().join("absent"))
        .output()
        .unwrap();
    assert_eq!(success(output), "");
    let formatted = format(library_source).unwrap();
    assert!(formatted.starts_with("public func public\n"));
    assert!(formatted.contains("\npublic func namespace\n"));
    assert_eq!(format(&formatted).unwrap(), formatted);
}

/// Reject the old namespace directive before resolving files, including in the
/// formatter. The CLI reports the migration at the original directive line.
#[test]
fn retired_import_directive_is_rejected() {
    let source = ENTRY.replacen("using holo.bytes", "import holo.bytes", 1);
    let (_scratch, entry, root) = fixture(&source, LIBRARY);
    let message = "replace `import` with `using` for namespace directives";
    let diagnostic = failure(command("check", &entry, &root).output().unwrap());
    assert!(diagnostic.contains("entry.holo:1:"), "{diagnostic}");
    assert!(diagnostic.contains(message), "{diagnostic}");
    assert!(format(&source).unwrap_err().message.contains(message));
}

/// The retired declaration spelling must fail in both checking and formatting.
/// The CLI diagnostic stays on the original library declaration line.
#[test]
fn retired_visibility_marker_is_rejected() {
    let library = LIBRARY.replacen("public func ", "export func ", 1);
    let (_scratch, entry, root) = fixture(ENTRY, &library);
    let diagnostic = failure(command("check", &entry, &root).output().unwrap());
    assert!(diagnostic.contains("bytes.holo:3:"), "{diagnostic}");
    assert!(diagnostic.contains("export func first"), "{diagnostic}");
    assert!(format(&library).is_err());
}

/// Reject a symbolic leaf before opening the target, even when it is valid Holo.
#[cfg(unix)]
#[test]
fn library_leaf_cannot_be_a_symbolic_link() {
    let (scratch, entry, root) = fixture(ENTRY, LIBRARY);
    let source = root.join("bytes.holo");
    let target = scratch.path().join("outside.holo");
    fs::rename(&source, &target).unwrap();
    std::os::unix::fs::symlink(target, source).unwrap();
    let diagnostic = failure(command("check", &entry, &root).output().unwrap());
    assert!(diagnostic.contains("expected a regular file, excluding symbolic links"));
}
