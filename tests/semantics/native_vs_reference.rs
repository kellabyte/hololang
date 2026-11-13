//! Differential execution of bounded generated IR against an independent model.
//!
//! Real LLVM executables run at O0 and O3 under all three explicit layout modes.
//! Expected values come from safe test-only
//! semantics, never checker/emitter helpers. Failure artifacts retain complete IR,
//! emitted LLVM, flags, tool identity, expected outputs, and native observations.
//! Native address observations are checked inside each run rather than compared
//! as host-dependent printed integers. These are correctness tests, not timings.

#[path = "../support/edges.rs"]
mod edges;
#[path = "../support/generator.rs"]
mod generator;
#[path = "../support/model/mod.rs"]
mod model;

use holo::driver::{NativeToolchain, TempDir};
use holo::ir::Program;
use holo::layout::LayoutMode;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Compare multiple complete generated inputs/layouts at both optimization levels.
#[test]
fn generated_programs_match_independent_model_in_native_builds() {
    let toolchain =
        NativeToolchain::discover().expect("model/native tests require matched LLVM tools");
    // Seeds vary values, finite capacities, and failure ordering. Complete input
    // is retained on failure because a seed cannot identify a future generator.
    for seed in [0, 1, 7, 19, 41, 255, 65_537, u64::MAX] {
        compare(
            &toolchain,
            &format!("seed-{seed}"),
            &generator::program(seed),
        );
    }
}

/// Explicit nested failures preserve published values, marker, and references.
#[test]
fn construction_boundaries_match_independent_model() {
    let toolchain = NativeToolchain::discover().expect("boundary tests require matched LLVM tools");
    compare(
        &toolchain,
        "construction-boundaries",
        &generator::boundaries(),
    );
}

/// Empty values, exact capacities, cursor rebinding, and captured prefixes agree.
#[test]
fn empty_records_and_prefix_iteration_match_independent_model() {
    let toolchain = NativeToolchain::discover().expect("edge tests require matched LLVM tools");
    compare(&toolchain, "empty-and-prefix", &edges::program());
}

/// Compile and run one input at both levels, checking deterministic scalar output.
///
/// `label` identifies replay artifacts. Every error stores complete context before
/// panicking; a missing tool or failed native build is not silently skipped.
fn compare(toolchain: &NativeToolchain, label: &str, program: &Program) {
    // Explicit modes share logical operations/checking while the independent oracle
    // derives each mode's own sizes, failure points, and stable record offsets.
    for mode in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        compare_layout(toolchain, &format!("{label}-{mode:?}"), program, mode);
    }
}

/// Compare one explicit layout at both optimization levels without sharing its plan.
/// Complete layout-labeled replay evidence is retained by the existing failure path.
fn compare_layout(toolchain: &NativeToolchain, label: &str, program: &Program, mode: LayoutMode) {
    let model_result = if mode == LayoutMode::Wide {
        model::run(program)
    } else {
        model::run_with_layout(program, mode)
    };
    let expected = model_result.unwrap_or_else(|error| {
        fail(
            label,
            program,
            "",
            toolchain,
            false,
            &[],
            &format!("model failed: {error}"),
        )
    });
    let checked = holo::check::check(program).unwrap_or_else(|error| {
        fail(
            label,
            program,
            "",
            toolchain,
            false,
            &expected,
            &format!("valid generated input rejected: {error}"),
        )
    });
    let llvm = holo::llvm::emit_with_layout(&checked, toolchain.target_triple(), mode)
        .unwrap_or_else(|error| {
            fail(
                label,
                program,
                "",
                toolchain,
                false,
                &expected,
                &format!("layout failed: {error}"),
            )
        });
    // The input and expectations are identical across optimization levels; a
    // backend-dependent numeric or alias interpretation is a correctness failure.
    for optimized in [false, true] {
        let scratch = TempDir::new().expect("create native model-test scratch directory");
        let executable = scratch.path().join("model native executable");
        toolchain
            .build(&llvm, &executable, optimized)
            .unwrap_or_else(|error| {
                fail(
                    label,
                    program,
                    &llvm,
                    toolchain,
                    optimized,
                    &expected,
                    &format!("native build failed: {error}"),
                )
            });
        let output = Command::new(&executable).output().unwrap_or_else(|error| {
            fail(
                label,
                program,
                &llvm,
                toolchain,
                optimized,
                &expected,
                &format!("native launch failed: {error}"),
            )
        });
        let observations = format!(
            "status: {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // A trap cannot be mistaken for a successful empty/matching output. The
        // generated positive fixtures are defined and expected to finish normally.
        if !output.status.success() {
            fail(
                label,
                program,
                &llvm,
                toolchain,
                optimized,
                &expected,
                &observations,
            );
        }
        let actual = parse_output(&output.stdout).unwrap_or_else(|error| {
            fail(
                label,
                program,
                &llvm,
                toolchain,
                optimized,
                &expected,
                &format!("{error}\n{observations}"),
            )
        });
        // Exact ordered values expose status, mutation, byte decoding, and scan
        // mismatches that a single final checksum could accidentally conceal.
        if actual != expected {
            fail(
                label,
                program,
                &llvm,
                toolchain,
                optimized,
                &expected,
                &observations,
            );
        }
    }
}

/// Parse only unsigned ASCII decimal lines produced by native Print operations.
fn parse_output(bytes: &[u8]) -> Result<Vec<u64>, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("non-UTF8 native output: {error}"))?;
    // A Print operation includes its newline; incomplete output must not compare
    // equal after a parser silently accepts a truncated final record.
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err("native output ended in an incomplete decimal line".into());
    }
    let mut values = Vec::new();
    // Reject malformed records rather than filtering unexpected diagnostics or
    // converting signed/overflowed values into apparently valid expected output.
    for line in text.split_terminator('\n') {
        // Each print is exactly one unsigned decimal value; whitespace, signs,
        // and blank lines would conceal a different native-output contract.
        if line.is_empty() || !line.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("malformed native output line: {line:?}"));
        }
        values.push(
            line.parse::<u64>()
                .map_err(|error| format!("native value out of range: {error}"))?,
        );
    }
    Ok(values)
}

/// Preserve replay evidence for a failing case, then stop the test explicitly.
///
/// Artifact I/O is test-only and failure paths never modify the original input.
/// A failed artifact write is reported rather than pretending evidence was saved.
fn fail(
    label: &str,
    program: &Program,
    llvm: &str,
    toolchain: &NativeToolchain,
    optimized: bool,
    expected: &[u64],
    detail: &str,
) -> ! {
    let saved = save_failure(label, program, llvm, toolchain, optimized, expected, detail);
    // Keep the original discrepancy in the panic even when the filesystem cannot
    // retain evidence; otherwise an I/O error would mask the compiler/model bug.
    match saved {
        Ok(path) => panic!("{label}: {detail}\nreplay evidence: {}", path.display()),
        Err(error) => panic!("{label}: {detail}\nfailed to save replay evidence: {error}"),
    }
}

/// Write a unique generated-artifact directory containing full replay inputs.
fn save_failure(
    label: &str,
    program: &Program,
    llvm: &str,
    toolchain: &NativeToolchain,
    optimized: bool,
    expected: &[u64],
    detail: &str,
) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CLI package must live directly under the workspace root")
        .to_path_buf()
        .join("build/failures")
        .join(format!("{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    fs::write(path.join("input.debug"), format!("{program:#?}\n"))
        .map_err(|error| error.to_string())?;
    fs::write(path.join("module.ll"), llvm).map_err(|error| error.to_string())?;
    fs::write(path.join("expected.txt"), format!("{expected:?}\n"))
        .map_err(|error| error.to_string())?;
    fs::write(path.join("observed.txt"), detail).map_err(|error| error.to_string())?;
    fs::write(
        path.join("toolchain.txt"),
        format!(
            "crate: {} {}\noptimized: {optimized}\ntarget: {}\nopt: {}\nclang: {}\n{}\n",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            toolchain.target_triple(),
            toolchain.opt.display(),
            toolchain.clang.display(),
            toolchain.identity
        ),
    )
    .map_err(|error| error.to_string())?;
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CLI package must live directly under the workspace root")
        .to_path_buf();
    // Preserve actual workspace-relative ownership so both manifests and their
    // embedded runtime/example inputs and Cargo configuration can be replayed.
    for component in ["compiler", "cli", "runtime", "tests", "examples", ".cargo"] {
        copy_tree(&manifest.join(component), &path.join(component))?;
    }
    // Version labels cannot identify dirty compiler code; retain build inputs
    // alongside complete source snapshots for a self-contained failing case.
    for name in ["Cargo.toml", "Cargo.lock"] {
        fs::copy(manifest.join(name), path.join(name)).map_err(|error| error.to_string())?;
    }
    Ok(path)
}

/// Copy maintained source bytes into a generated replay directory on failure.
///
/// Input directories are maintained workspace components, never build output.
/// Symlinks and special files are rejected rather than traversed implicitly.
fn copy_tree(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    fs::create_dir(destination).map_err(|error| error.to_string())?;
    // Snapshot complete source only for a failing case; normal validation and
    // native execution carry no filesystem-copy or provenance overhead.
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        // Preserve module boundaries and READMEs, copying only regular files.
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
        } else {
            return Err(format!(
                "cannot snapshot non-regular source entry {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}
