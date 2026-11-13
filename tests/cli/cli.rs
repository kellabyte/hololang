//! Command-level coverage for the experimental source compiler.
//!
//! Fresh temporary source files exercise option validation, UTF-8/source diagnostics,
//! LLVM-free checking, artifact ownership, native execution, and opt-in timings.
//! Native checks require real installed LLVM tools and never replace compilation
//! with host kernels. These tests measure correctness, not compiler performance.

use holo::driver::TempDir;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

/// Minimal source with visible native output and explicit successful entry return.
const SOURCE: &str = "// CLI fixture: source is compiled through the checked IR.\n\
func main\n\
    returns u64\n\
{\n\
    core.print(42)\n\
    return 0\n\
}\n";

/// Valid source with one nonconforming binding and two ordinary uses.
const NAMING_SOURCE: &str = "func main returns u64 {
    let packetCount = 1
    core.print(packetCount)
    core.print(packetCount)
    return 0
}
";

/// Own a temporary UTF-8 source file with spaces in its path for one test.
/// Returns its directory owner and path; dropping the owner removes its artifacts.
fn fixture(source: &str) -> (TempDir, PathBuf) {
    let scratch = TempDir::new().unwrap();
    let path = scratch.path().join("source program.holo");
    fs::write(&path, source).unwrap();
    (scratch, path)
}

/// Return a direct invocation of the Cargo-built executable with no shell parsing.
/// Environment inheritance is unchanged unless a test explicitly overrides a key.
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_holo"))
}

/// Require successful command completion and return its UTF-8 stdout for checking.
/// Retains stderr in assertion failures; does not hide launch failures or warnings.
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "status {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Source checking works by default and with the legacy opt-in used by scripts.
#[test]
fn source_check_accepts_the_legacy_experimental_flag() {
    let (_scratch, path) = fixture(SOURCE);
    // Both forms check the same program without producing output or warnings.
    for flags in [None, Some("--experimental")] {
        let output = cli().arg("check").args(flags).arg(&path).output().unwrap();
        assert!(output.stderr.is_empty());
        assert_eq!(success(output), "");
    }
}

/// Check succeeds with an authoritative nonexistent LLVM installation override.
#[test]
fn source_check_is_independent_of_llvm_and_keeps_timings_on_stderr() {
    let (scratch, path) = fixture(SOURCE);
    let missing = scratch.path().join("no LLVM installation");
    let output = cli()
        .args(["check", "--timings"])
        .arg(path)
        .env("HOLO_LLVM_PREFIX", missing)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("timing parse_lower:"));
    assert!(stderr.contains("timing check:"));
    assert!(!stderr.contains("llvm_lower"));
    assert!(!stderr.contains("command:"));
    assert_eq!(success(output), "");
}

/// Layout inspection needs no LLVM and exposes width/eligibility instead of hidden fallback.
#[test]
fn layout_inspection_is_explicit_and_independent_of_llvm() {
    let source = "// Inline schema for check-only physical inspection.\n\
        struct Entry { id u64, payload Payload }\n\
        struct Payload { data Bytes }\n\
        func main returns u64 { return 0 }\n";
    let (scratch, path) = fixture(source);
    // The same logical source has an inspectable 32/16/12-byte zero-payload extent.
    for (mode, bytes, bits) in [("wide", 32, 64), ("derived", 16, 64), ("compact", 12, 32)] {
        let output = cli()
            .args(["inspect-layout", "--layout", mode])
            .arg(&path)
            .env("HOLO_LLVM_PREFIX", scratch.path().join("absent"))
            .output()
            .unwrap();
        let description = success(output);
        assert!(description.contains(&format!("byte lengths {bits} bits")));
        assert!(
            description.lines().any(
                |line| line.starts_with("Entry:") && line.contains(&format!("minimum={bytes}"))
            )
        );
    }
    // The ordinary source default elides eligible headers without narrowing u64 lengths.
    let description = success(
        cli()
            .args(["inspect-layout"])
            .arg(&path)
            .env("HOLO_LLVM_PREFIX", scratch.path().join("absent"))
            .output()
            .unwrap(),
    );
    assert!(description.starts_with("layout derived: byte lengths 64 bits"));
}

/// Copy inspection exposes the actual host policy without needing LLVM tools.
#[test]
fn copy_inspection_describes_guarded_and_ordinary_initializers() {
    let source = "// One ordinary initializer and one bounded-copy candidate.\n\
        struct Payload { data Bytes }\n\
        func seed returns Payload { return Payload { data: core.bytes(4, 17) } }\n\
        func copy_tail arg input Bytes as readonly reference arg length u64 as readonly copy\n\
        returns Payload { return Payload { data: core.bytes(length, input[core.add_wrapping(8, core.index())]) } }\n\
        func main returns u64 { return 0 }\n";
    let (scratch, path) = fixture(source);
    let description = success(
        cli()
            .args(["inspect-copies"])
            .arg(path)
            .env("HOLO_LLVM_PREFIX", scratch.path().join("absent"))
            .output()
            .unwrap(),
    );
    assert!(description.contains("seed"));
    assert!(description.contains("copy_tail"));
    assert!(description.contains("checked byte loop"));
    // Only the measured target policy can select the guarded path.
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        assert!(description.contains("guarded inline through 64 bytes"));
        assert!(description.contains("offset 8"));
    } else {
        assert!(description.contains("scoped to macOS arm64"));
    }
}

/// Unknown/repeated layout options and ineffective inspection flags never get ignored.
#[test]
fn invalid_layout_invocations_are_rejected() {
    let (_scratch, path) = fixture(SOURCE);
    let variants: &[&[&str]] = &[
        &["check", "--layout", "unknown"],
        &["check", "--layout", "wide", "--layout", "compact"],
        &["inspect-layout", "--release"],
    ];
    // Every invalid invocation fails before tool discovery or any output artifact creation.
    for arguments in variants {
        let output = cli().args(*arguments).arg(&path).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

/// Native layout selection changes actual storage while compiling the same checked source.
#[test]
fn explicit_layouts_reach_native_storage() {
    let source = "// Native nested storage observation, independent of driver implementation.\n\
        struct Payload { data Bytes }\n\
        struct Entry { id u64, payload Payload }\n\
        func make returns Entry { return Entry { id: 7, payload: Payload { data: core.bytes(3, 9) } } }\n\
        func main returns u64 {\n\
            let memory = core.region(64) else { return 0 }\n\
            let entries = core.batch(memory, Entry)\n\
            let status = core.append(entries, make())\n\
            core.assert(status == 0)\n\
            core.print(core.used(memory))\n\
            return 0\n\
        }\n";
    let (_scratch, path) = fixture(source);
    // Native byte counts include exactly three initialized payload bytes in every representation.
    for (mode, bytes) in [("wide", 35), ("derived", 19), ("compact", 15)] {
        let output = cli()
            .args(["run", "--release", "--layout", mode])
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(success(output), format!("{bytes}\n"));
    }
    // Default source execution uses the measured full-width derived representation.
    assert_eq!(
        success(
            cli()
                .args(["run", "--release"])
                .arg(&path)
                .output()
                .unwrap()
        ),
        "19\n"
    );
}

/// Invalid source reports its exact file location before any native-tool lookup.
#[test]
fn source_diagnostics_are_rendered_and_return_failure() {
    let (scratch, path) = fixture("func main\nreturns u64 {\n@\n}\n");
    let output = cli()
        .args(["check"])
        .arg(&path)
        .env("HOLO_LLVM_PREFIX", scratch.path().join("absent"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains(&format!("{}:3:1:", path.display())),
        "{stderr}"
    );
    assert!(stderr.contains("error:"));
    assert!(stderr.contains('^'));
    assert!(!stderr.contains("LLVM opt/clang"));
}

/// Naming policy marks the written declaration once and leaves references alone.
#[test]
fn naming_diagnostics_point_once_to_the_declaration() {
    let (scratch, path) = fixture(NAMING_SOURCE);
    let absent = scratch.path().join("no LLVM installation");
    // Both modes check the same valid source without discovering LLVM.
    for (flag, severity, status) in [(None, "warning", 0), (Some("--strict-names"), "error", 1)] {
        let output = cli()
            .args(["check"])
            .args(flag)
            .arg(&path)
            .env("HOLO_LLVM_PREFIX", &absent)
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(status), "{stderr}");
        assert!(output.stdout.is_empty(), "{stderr}");
        assert_eq!(
            stderr.matches(": warning:").count(),
            usize::from(status == 0)
        );
        assert_eq!(stderr.matches(": error:").count(), usize::from(status != 0));
        assert!(
            stderr.contains(&format!("{}:2:9: {severity}:", path.display())),
            "{stderr}"
        );
        assert!(
            stderr.contains("\n2 |     let packetCount = 1\n  |         ^^^^^^^^^^^\n"),
            "{stderr}"
        );
        assert!(stderr.contains("snake_case"), "{stderr}");
        assert!(stderr.contains("`packet_count`"), "{stderr}");
    }
}

/// Strict naming rejection occurs before discovery or native artifact creation.
#[test]
fn strict_naming_build_stops_before_native_discovery() {
    let (scratch, path) = fixture(NAMING_SOURCE);
    let artifact = scratch.path().join("rejected native program");
    let absent = scratch.path().join("no LLVM installation");
    let output = cli()
        .args(["build", "--strict-names"])
        .arg(&path)
        .arg("-o")
        .arg(&artifact)
        .env("HOLO_LLVM_PREFIX", absent)
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!output.status.success(), "{stderr}");
    assert!(output.stdout.is_empty(), "{stderr}");
    assert!(
        stderr.contains(&format!("{}:2:9: error:", path.display())),
        "{stderr}"
    );
    assert!(!stderr.contains("HOLO_LLVM_PREFIX"), "{stderr}");
    assert!(!artifact.exists());
}

/// An invalid checked program reports its access error before naming policy.
#[test]
fn naming_diagnostics_wait_for_semantic_validity() {
    let source = "struct Row { id u64 }
func row returns Row { return Row { id: 7 } }
func main returns u64 {
    let storage = core.region(64) else { return 0 }
    let rows = core.batch(storage, Row)
    let status = core.append(rows, row())
    core.assert(status == 0)
    let rowView = core.borrow_readonly(rows, 0)
    rowView.id = 9
    return 0
}
";
    let (_scratch, path) = fixture(source);
    // The write lowers to checked IR and must fail independently of style policy.
    for flag in [None, Some("--strict-names")] {
        let output = cli()
            .args(["check"])
            .args(flag)
            .arg(&path)
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!output.status.success(), "{stderr}");
        assert!(output.stdout.is_empty(), "{stderr}");
        assert!(stderr.contains("readonly"), "{stderr}");
        assert!(!stderr.contains("warning:"), "{stderr}");
        assert!(
            !stderr.contains(&format!("{}:8:9:", path.display())),
            "{stderr}"
        );
    }
}

/// Formatting repairs whitespace and preserves the original identifier spelling.
#[test]
fn formatting_preserves_nonconforming_names() {
    let source = NAMING_SOURCE.replace("packetCount = 1", "packetCount=1");
    let (_scratch, path) = fixture(&source);
    let output = cli().arg("fmt").arg(&path).output().unwrap();
    assert!(output.stderr.is_empty(), "{output:?}");
    let formatted = success(output);
    assert!(formatted.contains("let packetCount = 1"), "{formatted}");
    assert_eq!(formatted.matches("packetCount").count(), 3, "{formatted}");
}

/// Helper parameters respect the same explicit core namespace as entry locals.
#[test]
fn experimental_core_namespace_cannot_be_shadowed_by_helper_arguments() {
    let source = format!(
        "// Namespace regression: even unused helpers must be validated.\n\
         func helper\n\
             arg core u64 as readonly copy\n\
             returns u64\n\
         {{ return core }}\n\
         {SOURCE}",
    );
    let (scratch, path) = fixture(&source);
    let output = cli()
        .args(["check"])
        .arg(path)
        .env("HOLO_LLVM_PREFIX", scratch.path().join("absent"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("argument `core`"));
    assert!(stderr.contains("reserved experimental core namespace"));
}

/// Required commands/arguments and meaningful option combinations are enforced.
#[test]
fn invocation_errors_are_explicit_and_help_needs_no_source() {
    // Every case must fail at the argument boundary without attempting file I/O.
    for arguments in [
        vec![],
        vec!["unknown"],
        vec!["check"],
        vec!["build", "missing.holo"],
        vec!["check", "missing.holo", "--release"],
        vec!["run", "missing.holo", "-o", "output"],
        vec!["check", "missing.holo", "--mystery"],
    ] {
        let output = cli().args(arguments).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    let output = cli().arg("--help").output().unwrap();
    assert!(output.stderr.is_empty());
    let help = success(output);
    assert!(help.contains("<source.holo>"));
    assert!(!help.contains("--experimental"));
}

/// UTF-8 source/path limits fail clearly instead of panicking or lossy replacement.
#[test]
fn non_utf8_source_is_a_contextual_read_error() {
    let (_scratch, path) = fixture(SOURCE);
    fs::write(&path, [0xff]).unwrap();
    let output = cli().args(["check"]).arg(&path).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("UTF-8"));
    assert!(stderr.contains(&path.display().to_string()));
}

/// A non-UTF-8 invocation path is rejected at the CLI boundary on supported hosts.
#[cfg(unix)]
#[test]
fn non_utf8_path_is_rejected_before_source_io() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let output = cli()
        .args(["check"])
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("paths must be valid UTF-8"));
}

/// Source LLVM emission produces checked text on stdout and new output files.
#[test]
fn source_emits_native_llvm_text_without_stdout_timing_noise() {
    let (scratch, path) = fixture(SOURCE);
    let output = cli().args(["emit-llvm"]).arg(&path).output().unwrap();
    assert!(output.stderr.is_empty());
    let stdout = success(output);
    assert!(stdout.contains("define i32 @main()"));
    assert!(stdout.contains("call void @exit(i32 0)"));
    let output_path = scratch.path().join("checked output.ll");
    assert_eq!(
        success(
            cli()
                .args(["emit-llvm"])
                .arg(&path)
                .arg("-o")
                .arg(&output_path)
                .output()
                .unwrap()
        ),
        "",
    );
    assert_eq!(fs::read_to_string(output_path).unwrap(), stdout);
}

/// Build/run execute actual parsed source at default O0 and explicit release O3.
#[test]
fn source_build_and_run_preserve_output_and_successful_return() {
    let (scratch, path) = fixture(SOURCE);
    // These are correctness comparisons over identical source, not timing samples.
    for release in [false, true] {
        let executable = scratch.path().join(format!("native program {release}"));
        let mut command = cli();
        command
            .args(["build"])
            .arg(&path)
            .arg("-o")
            .arg(&executable);
        if release {
            command.arg("--release");
        }
        let output = command.output().unwrap();
        assert!(output.stderr.is_empty());
        assert_eq!(success(output), "");
        assert_eq!(success(Command::new(executable).output().unwrap()), "42\n");
        let mut command = cli();
        command.args(["run", "--timings"]).arg(&path);
        if release {
            command.arg("--release");
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("timing llvm_lower:"));
        assert!(stderr.contains("timing native_driver:"));
        assert!(stderr.contains("timing native_verification:"));
        assert!(stderr.contains("timing native_codegen_link:"));
        assert!(stderr.contains("command:"));
        assert_eq!(success(output), "42\n");
    }
}

/// Compile the public greeting and verify literal bytes alongside numeric output.
/// All artifacts are temporary, and native execution supplies the exact stdout.
#[test]
fn text_print_preserves_utf8_escapes_and_numeric_output() {
    let hello = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/hello_world/main.holo");
    let output = cli().args(["run"]).arg(hello).output().unwrap();
    assert_eq!(success(output), "Hello, world!\n");

    let (_scratch, path) = fixture(
        r#"func main returns u64 {
    core.print(42)
    core.print("%s: \"café\"\\\tend\r\nnext")
    core.print("")
    return 0
}
"#,
    );
    let output = cli().args(["run", "--release"]).arg(path).output().unwrap();
    assert_eq!(success(output), "42\n%s: \"café\"\\\tend\r\nnext\n\n");
}

/// Reject malformed literals and uses outside entry printing before native I/O.
#[test]
fn text_literals_are_checked_at_the_print_boundary() {
    // Each fixture violates one independent constraint of the literal-only operation.
    for source in [
        r#"func main returns u64 { core.print("bad\q"); return 0 }"#,
        "func main returns u64 { core.print(\"zero\0byte\"); return 0 }",
        r#"func main returns u64 { let text = "hello"; return 0 }"#,
        r#"func helper returns u64 { core.print("hello"); return 0 }
func main returns u64 { return 0 }"#,
    ] {
        let (_scratch, path) = fixture(source);
        let output = cli().args(["check"]).arg(path).output().unwrap();
        assert!(!output.status.success(), "{source}");
        assert!(output.stdout.is_empty(), "{source}");
        assert!(!output.stderr.is_empty(), "{source}");
    }
}

/// A native assertion trap must remain unsuccessful through the CLI run boundary.
#[test]
fn native_trap_is_reported_on_stderr_with_unsuccessful_status() {
    let source = SOURCE.replace("core.print(42)", "core.assert(false)");
    let (_scratch, path) = fixture(&source);
    let output = cli().args(["run"]).arg(path).output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("native program"));
}

/// Neither output command can replace its own source or an existing artifact.
#[test]
fn output_creation_refuses_to_overwrite_source_or_existing_artifacts() {
    let (scratch, path) = fixture(SOURCE);
    let artifact = scratch.path().join("existing artifact");
    fs::write(&artifact, "preserved").unwrap();
    // The source-path case protects against accidentally replacing compiler input;
    // the independent file case exercises the general exclusive-output guarantee.
    for command in ["emit-llvm", "build"] {
        for target in [&path, &artifact] {
            let output = cli()
                .args([command])
                .arg(&path)
                .arg("-o")
                .arg(target)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(!output.stderr.is_empty());
            assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
            assert_eq!(fs::read_to_string(&artifact).unwrap(), "preserved");
        }
    }
}
