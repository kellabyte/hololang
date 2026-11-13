//! Parsed-source conformance for synchronous borrowed byte inputs and owned output.
//!
//! Hand expectations and an independent safe model validate the same source under
//! all physical layouts and O0/O3/ASan. Native guard traps use a distinctive test-only
//! exit, so a segfault cannot masquerade as a bounds check. The harness implements
//! no record algorithm and makes no timing or dynamic-copy-volume claim.

#[path = "../../support/byte_input_fixtures.rs"]
mod fixtures;
#[path = "../../support/model/mod.rs"]
mod model;

use fixtures::{Fixture, Outcome};
use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::parse_experimental;
use holo::layout::LayoutMode;
use std::fs::{self, File};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Return a fixed order for the explicit supported representation configurations.
fn layouts() -> [LayoutMode; 3] {
    [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ]
}

/// Every fixture checks and independently models its exact success or bounds-failure outcome.
/// This test requires no LLVM executable and performs no native allocation measurements.
#[test]
fn parsed_byte_inputs_match_hand_values_and_independent_model() {
    // Complete sources are retained, rather than relying on a random seed or a compiler oracle.
    for fixture in fixtures::fixtures() {
        let input = parse_experimental(&fixture.source)
            .unwrap_or_else(|error| panic!("{}", error.render(fixture.label, &fixture.source)));
        input
            .check()
            .unwrap_or_else(|error| panic!("{}", error.render(fixture.label, &fixture.source)));
        // Independent model storage accounting varies with the explicit metadata widths.
        for layout in layouts() {
            validate_model(&fixture, input.program(), layout);
        }
    }
}

/// All admitted source paths execute in real native binaries under optimizers and ASan.
/// Children run sequentially in this test; this is correctness coverage, not a benchmark.
#[test]
fn parsed_byte_inputs_execute_with_checked_native_reads() {
    let tools = NativeToolchain::discover().expect("byte-input native tests require matched LLVM");
    // Each case owns its artifacts, expectations, and independent semantic result.
    for fixture in fixtures::fixtures() {
        compare(&fixture, &tools);
    }
}

/// Require the independent model to match hand values or the specified bounds trap.
/// `program` is unchanged; expected traps must not match unrelated oracle errors.
fn validate_model(fixture: &Fixture, program: &holo::ir::Program, layout: LayoutMode) {
    // Exercise both public model entry points while preserving explicit alternate layouts.
    let actual = if layout == LayoutMode::Wide {
        model::run(program)
    } else {
        model::run_with_layout(program, layout)
    };
    match fixture.outcome(layout) {
        Outcome::Values(expected) => {
            assert_eq!(actual.unwrap(), expected, "{}/{layout:?}", fixture.label);
        }
        Outcome::BoundsTrap { model_message, .. } => {
            let error = actual.expect_err("invalid source index must fail in the model");
            assert_eq!(error.message, model_message, "{}: {error}", fixture.label);
        }
        Outcome::OffsetBoundsTrap => {
            let error = actual.expect_err("the second range byte exceeds the source extent");
            assert_eq!(error.message, "input byte read bounds trap");
        }
    }
}

/// Check, emit, compile, and execute one unchanged source under the finite native matrix.
/// Only guard termination is instrumented for trap cases; source byte operations stay emitted.
fn compare(fixture: &Fixture, tools: &NativeToolchain) {
    let input = parse_experimental(&fixture.source)
        .unwrap_or_else(|error| panic!("{}", error.render(fixture.label, &fixture.source)));
    let checked = input
        .check()
        .unwrap_or_else(|error| panic!("{}", error.render(fixture.label, &fixture.source)));
    let scratch = TempDir::new().unwrap();
    fs::write(scratch.path().join("source.holo"), &fixture.source).unwrap();
    fs::write(
        scratch.path().join("input.ir"),
        format!("{:#?}", input.program()),
    )
    .unwrap();
    fs::write(scratch.path().join("toolchain.txt"), &tools.identity).unwrap();
    // Keep original and observation-instrumented LLVM separate for honest replay.
    for layout in layouts() {
        validate_model(fixture, input.program(), layout);
        let original =
            holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        let (llvm, expected_status, expected_text) = match fixture.outcome(layout) {
            Outcome::Values(values) => (
                original.clone(),
                0,
                values
                    .iter()
                    .map(|value| format!("{value}\n"))
                    .collect::<String>(),
            ),
            Outcome::BoundsTrap { index, .. } => {
                // Observe only the intended byte-read guard; setup assertions retain the target trap.
                (observe_guard(&original, index), 86, String::new())
            }
            Outcome::OffsetBoundsTrap => {
                // The late invalid read uses the original generated target trap.
                let signal = match std::env::consts::ARCH {
                    "aarch64" => 5,
                    "x86_64" => 4,
                    architecture => panic!("expected trap signal for {architecture}"),
                };
                (original.clone(), -signal, String::new())
            }
        };
        fs::write(
            scratch.path().join(format!("{layout:?}-original.ll")),
            &original,
        )
        .unwrap();
        fs::write(
            scratch.path().join(format!("{layout:?}-observed.ll")),
            &llvm,
        )
        .unwrap();
        fs::write(
            scratch.path().join(format!("{layout:?}-expected.txt")),
            format!("status: {expected_status}\n{expected_text}"),
        )
        .unwrap();
        // O0 checks direct lowering; O3 and ASan expose optimization and invalid-access defects.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch.path().join(format!("{layout:?}-{mode}"));
            let build = match mode {
                "asan" => tools.build_sanitized(&llvm, &binary),
                "o3" => tools.build(&llvm, &binary, true),
                _ => tools.build(&llvm, &binary, false),
            }
            .unwrap_or_else(|error| fail(scratch.path(), fixture.label, &error));
            fs::write(
                binary.with_extension("commands"),
                format!("{:#?}", build.commands),
            )
            .unwrap();
            let output = run_native(&binary)
                .unwrap_or_else(|error| fail(scratch.path(), fixture.label, &error));
            // ASan diagnostics, unexpected output, other signals, and guard omissions all fail.
            let actual_status = output
                .status
                .code()
                .or_else(|| output.status.signal().map(|signal| -signal));
            if actual_status != Some(expected_status)
                || output.stdout != expected_text.as_bytes()
                || !output.stderr.is_empty()
            {
                fail(
                    scratch.path(),
                    fixture.label,
                    &format!(
                        "{layout:?}/{mode}: status {} expected {expected_status}\nstdout: {}\nstderr: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    ),
                );
            }
        }
    }
}

/// Redirect exactly one specified byte-index guard to a distinctive termination observer.
/// `index` is the fixture's deliberately invalid constant. The unoptimized emitter
/// places its icmp immediately before holo.assert; fail if that provenance changes
/// instead of guessing a site. Other assertions and all byte loads stay unchanged.
fn observe_guard(llvm: &str, index: u64) -> String {
    let pattern = format!(" = icmp ult i64 {index}, ");
    let mut condition = None;
    let mut matched = 0;
    let mut observed = String::with_capacity(llvm.len() + GUARD_OBSERVER.len());
    // Inspect a finite module; only an adjacent use of the selected SSA condition qualifies.
    for line in llvm.lines() {
        if let Some(value) = condition.take() {
            assert_eq!(line.trim(), format!("call void @holo.assert(i1 {value})"));
            observed.push_str(&line.replace("@holo.assert(", "@byte.input.guard("));
            matched += 1;
        } else {
            // The complete constant operand excludes setup index0 and dynamic loop guards.
            if let Some((value, _)) = line.trim().split_once(&pattern) {
                condition = Some(value.to_owned());
            }
            observed.push_str(line);
        }
        observed.push('\n');
    }
    assert!(condition.is_none(), "incomplete expected byte guard");
    assert_eq!(matched, 1, "fixture must identify exactly one input guard");
    observed.push_str(GUARD_OBSERVER);
    observed
}

/// Execute one owned child with file streams and a ten-second wall-clock bound.
/// Timeout kills/reaps only that child; available streams remain for failure evidence.
fn run_native(binary: &Path) -> Result<Output, String> {
    let stdout = binary.with_extension("stdout");
    let stderr = binary.with_extension("stderr");
    let mut child = Command::new(binary)
        .env("ASAN_OPTIONS", "detect_leaks=0:handle_sigill=0")
        .stdout(File::create_new(&stdout).map_err(|error| error.to_string())?)
        .stderr(File::create_new(&stderr).map_err(|error| error.to_string())?)
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(10);
    // A broken loop must terminate the test with evidence rather than hang the suite.
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().map_err(|error| error.to_string())?;
            child.wait().map_err(|error| error.to_string())?;
            return Err("byte-input child exceeded ten seconds".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(Output {
        status,
        stdout: fs::read(stdout).map_err(|error| error.to_string())?,
        stderr: fs::read(stderr).map_err(|error| error.to_string())?,
    })
}

/// Save complete current source/test/compiler/native inputs, then fail without hiding its cause.
/// Successful validation does not pay the source-snapshot cost; evidence I/O errors stay visible.
fn fail(scratch: &Path, label: &str, detail: &str) -> ! {
    let save = || -> std::io::Result<PathBuf> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CLI package must live directly under the workspace root");
        let target = manifest
            .join("build/failures")
            .join(format!("{label}-{}-{stamp}", std::process::id()));
        snapshot(scratch, &target)?;
        // Workspace-relative trees retain package manifests and external native
        // support and Cargo configuration without archived/generated replay inputs.
        for component in ["compiler", "cli", "runtime", "tests", "examples", ".cargo"] {
            snapshot(&manifest.join(component), &target.join(component))?;
        }
        // Preserve dirty build inputs rather than relying on a version label alone.
        for name in ["Cargo.toml", "Cargo.lock"] {
            fs::copy(manifest.join(name), target.join(name))?;
        }
        fs::write(target.join("failure.txt"), detail)?;
        Ok(target)
    };
    panic!("{label}: {detail}\nevidence: {:?}", save());
}

/// Copy only finite owned regular source/artifact trees; reject symlinks and special files.
/// `source` stays unchanged, `destination` is newly created, and filesystem errors propagate.
fn snapshot(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    // Recursion visits maintained inputs or this test's scratch directory, never external links.
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            snapshot(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(std::io::Error::other(
                "non-regular file in byte-input replay tree",
            ));
        }
    }
    Ok(())
}

/// Test-only termination observer executes solely after the compiler's real guard fails.
/// The explicit status distinguishes specified rejection from a signal or ASan failure.
const GUARD_OBSERVER: &str = r#"
declare void @_Exit(i32) noreturn
define internal void @byte.input.guard(i1 %condition) {
entry:
  br i1 %condition, label %valid, label %invalid
valid:
  ret void
invalid:
  call void @_Exit(i32 86)
  unreachable
}
"#;
