//! Parsed-source native checks for independent nested storage-owner lifetimes.
//!
//! The independent model and hand-derived fixture outputs observe the same source
//! program. Test-only LLVM wrappers observe real backing acquisition/free order,
//! and optionally return NULL at an actual malloc boundary; they implement no
//! record operations. Bounded native children run each layout at O0/O3/ASan.

#[path = "../support/model/mod.rs"]
mod model;
#[path = "../support/owner_fixtures.rs"]
mod owner_fixtures;
#[path = "../support/recovery_fixtures.rs"]
mod recovery_fixtures;

use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::{SourceProgram, parse_experimental};
use holo::ir::Op;
use holo::layout::LayoutMode;
use owner_fixtures::Fixture;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Collect ten bounded source paths, including recoverable acquisition branches.
/// Every program uses the same checked model and real allocator-observation path.
fn fixtures() -> [Fixture; 10] {
    [
        owner_fixtures::continuation(),
        owner_fixtures::append_exit(true),
        owner_fixtures::append_exit(false),
        owner_fixtures::acquire_exit(),
        recovery_fixtures::acquire_retry(false),
        recovery_fixtures::acquire_retry(true),
        recovery_fixtures::acquire_branch(true),
        recovery_fixtures::acquire_branch(false),
        recovery_fixtures::maintained_recovery(),
        recovery_fixtures::acquire_retry_failure(),
    ]
}

/// Every source fixture checks and matches hand-derived output in the safe model.
/// This check is runnable without invoking the native LLVM toolchain.
#[test]
fn parsed_owner_lifecycles_match_independent_model() {
    // Each fixture uses the same source frontend and model path as native execution below.
    for fixture in fixtures() {
        let input = parse(&fixture);
        input
            .check()
            .unwrap_or_else(|error| panic!("{}", error.render(fixture.label, &fixture.source)));
        // Representation changes cannot change these logical outputs or within-run identities.
        for layout in layouts() {
            check_model(&fixture, &input, layout);
        }
    }
}

/// All source paths execute under real allocation and reverse-order ownership guards.
/// Each child is sequential within this matrix; failures retain exact reproductions.
#[test]
fn parsed_owner_lifecycles_match_native_in_all_layouts() {
    let tools = NativeToolchain::discover().expect("owner lifecycle tests require matched LLVM");
    // Measured competitors are not involved: these are bounded correctness fixtures only.
    for fixture in fixtures() {
        compare(&fixture, &tools);
    }
}

/// Return the three supported experimental representations in a fixed order.
fn layouts() -> [LayoutMode; 3] {
    [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ]
}

/// Parse actual source; diagnostics retain source spans on any unsupported construct.
fn parse(fixture: &Fixture) -> SourceProgram {
    parse_experimental(&fixture.source)
        .unwrap_or_else(|error| panic!("{}", error.render(fixture.label, &fixture.source)))
}

/// Compare a parsed program against independent values, injecting only an acquisition fault.
/// Model injection does not replace or alter the checked source used by native codegen.
fn check_model(fixture: &Fixture, input: &SourceProgram, layout: LayoutMode) {
    let mut program = input.program().clone();
    assert_eq!(
        fixture.failed_capacity.is_some(),
        fixture.expected_faults != 0,
        "fault metadata must explicitly select injection and its expected count"
    );
    // The source has no fault-injection syntax: the model mirrors the allocator wrapper only.
    if fixture.failed_capacity.is_some() {
        assert_eq!(
            u64::try_from(inject_acquire_failure(&mut program.body))
                .expect("finite test fault count must fit u64"),
            fixture.expected_faults
        );
    }
    // Retain coverage of the model's default API as well as explicit alternate layouts.
    let output = if layout == LayoutMode::Wide {
        model::run(&program)
    } else {
        model::run_with_layout(&program, layout)
    }
    .unwrap();
    assert_eq!(output, fixture.expected, "{}/{layout:?}", fixture.label);
}

/// Toggle precisely designated test owners in a model-only clone; return match count.
/// Structured recursion visits both failure and success paths without guessing layout sizes.
fn inject_acquire_failure(operations: &mut [Op]) -> usize {
    let mut matches = 0;
    // Every nested lexical/control scope can contain the named source declaration.
    for operation in operations {
        // Only Region has an acquisition flag; other structured operations recurse into bodies.
        match operation {
            Op::Region {
                name,
                fail_acquire,
                body,
                on_failure,
                ..
            } => {
                // The adapter appends an unspellable `.$<id>` to each source name.
                // Match the two deliberate source hints, not unrelated owners
                // whose capacities or name prefixes happen to resemble them.
                if name.starts_with("unavailable.$") || name.starts_with("unavailable_retry.$") {
                    *fail_acquire = true;
                    matches += 1;
                }
                matches += inject_acquire_failure(body);
                matches += inject_acquire_failure(on_failure);
            }
            Op::Batch { body, .. }
            | Op::Scope(body)
            | Op::Repeat { body, .. }
            | Op::ForEach { body, .. } => matches += inject_acquire_failure(body),
            Op::If {
                then_body,
                else_body,
                ..
            } => {
                matches += inject_acquire_failure(then_body);
                matches += inject_acquire_failure(else_body);
            }
            _ => {}
        }
    }
    matches
}

/// Run one source fixture through checking, all layouts, and O0/O3/ASan code generation.
/// Temporary output belongs to this test; failed builds/runs copy it into build/failures.
fn compare(fixture: &Fixture, tools: &NativeToolchain) {
    let input = parse(fixture);
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
    let expected = fixture
        .expected
        .iter()
        .map(|value| format!("{value}\n"))
        .collect::<String>();
    fs::write(scratch.path().join("expected.txt"), &expected).unwrap();
    // Each layout retains its original compiler output separately from test instrumentation.
    for layout in layouts() {
        check_model(fixture, &input, layout);
        let original =
            holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        let guarded = track_owners(&original, fixture);
        fs::write(
            scratch.path().join(format!("{layout:?}-original.ll")),
            original,
        )
        .unwrap();
        fs::write(
            scratch.path().join(format!("{layout:?}-guarded.ll")),
            &guarded,
        )
        .unwrap();
        // O0 tests direct lowering, O3 exposes optimizer mistakes, ASan checks real memory access.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch.path().join(format!("{layout:?}-{mode}"));
            // Select actual driver modes while preserving source, representation, and guards.
            let build = match mode {
                "asan" => tools.build_sanitized(&guarded, &binary),
                "o3" => tools.build(&guarded, &binary, true),
                _ => tools.build(&guarded, &binary, false),
            }
            .unwrap_or_else(|error| failure(scratch.path(), fixture.label, &error));
            fs::write(
                binary.with_extension("commands"),
                format!("{:#?}", build.commands),
            )
            .unwrap();
            let output = run_native(&binary)
                .unwrap_or_else(|error| failure(scratch.path(), fixture.label, &error));
            // Traps, ASan reports, leaked owners, extra output, and mismatches are all failures.
            if !output.status.success()
                || output.stdout != expected.as_bytes()
                || !output.stderr.is_empty()
            {
                failure(
                    scratch.path(),
                    fixture.label,
                    &format!(
                        "{layout:?}/{mode}: status {}\nstdout: {}\nstderr: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    ),
                );
            }
        }
    }
}

/// Run this test's child with file streams and a ten-second wall-clock deadline.
/// Only the owned child is killed on timeout; output files survive for evidence copying.
fn run_native(binary: &Path) -> Result<Output, String> {
    let stdout_path = binary.with_extension("stdout");
    let stderr_path = binary.with_extension("stderr");
    let mut child = Command::new(binary)
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .stdout(File::create_new(&stdout_path).map_err(|error| error.to_string())?)
        .stderr(File::create_new(&stderr_path).map_err(|error| error.to_string())?)
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(10);
    // Polling terminates on actual completion or timeout without pipe backpressure.
    let status = loop {
        // Completion preserves the actual native exit status and avoids an unnecessary kill.
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        // A broken cursor or cleanup cycle must fail rather than hang the complete suite.
        if Instant::now() >= deadline {
            child.kill().map_err(|error| error.to_string())?;
            child.wait().map_err(|error| error.to_string())?;
            return Err("owner lifecycle child exceeded ten seconds".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(Output {
        status,
        stdout: fs::read(stdout_path).map_err(|error| error.to_string())?,
        stderr: fs::read(stderr_path).map_err(|error| error.to_string())?,
    })
}

/// Preserve existing complete source/IR/LLVM/command/output files, then fail this case.
/// Evidence I/O failure is reported alongside the original failure instead of hiding it.
fn failure(scratch: &Path, label: &str, detail: &str) -> ! {
    let save = || -> Result<PathBuf, std::io::Error> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let target = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CLI package must live directly under the workspace root")
            .join("build/failures")
            .join(format!("{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&target)?;
        // NativeToolchain writes flat artifacts; copy only regular scratch files, never follow dirs.
        for entry in fs::read_dir(scratch)? {
            let entry = entry?;
            // Reject unexpected subdirectories rather than silently omitting evidence.
            if !entry.file_type()?.is_file() {
                return Err(std::io::Error::other(
                    "non-file in owned native scratch directory",
                ));
            }
            fs::copy(entry.path(), target.join(entry.file_name()))?;
        }
        fs::write(target.join("failure.txt"), detail)?;
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CLI package must live directly under the workspace root");
        // Retain workspace-relative component trees, including both package
        // manifests, native support and Cargo configuration for exact source replay.
        for component in ["compiler", "cli", "runtime", "tests", "examples", ".cargo"] {
            snapshot_tree(&manifest.join(component), &target.join(component))?;
        }
        // Versions alone cannot identify dirty compiler/test inputs needed for source replay.
        for name in ["Cargo.toml", "Cargo.lock"] {
            fs::copy(manifest.join(name), target.join(name))?;
        }
        Ok(target)
    };
    panic!("{label}: {detail}\nevidence: {:?}", save());
}

/// Snapshot maintained source/test inputs into an owned failure-artifact directory.
/// Both paths are borrowed; only the destination changes. Recursion follows real
/// directories and copies regular files, rejecting symlinks/special files with an
/// I/O error. This failure-only cost does not affect successful native execution.
fn snapshot_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir(destination)?;
    // Visit the finite maintained tree, never generated build output or external links.
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        // Preserve source module boundaries without following anything outside this tree.
        if kind.is_dir() {
            snapshot_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(std::io::Error::other(format!(
                "cannot snapshot non-regular source entry {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

/// Add test-only real allocator guards and optional NULL injection to emitted LLVM.
/// A sixteen-slot owner stack enforces LIFO order and exact total acquisitions/frees.
/// Volatile fault configuration plus noinline wrapper preserve the real NULL branch at O3.
fn track_owners(llvm: &str, fixture: &Fixture) -> String {
    let mut guarded = llvm
        .replace("call ptr @malloc(", "call ptr @owner.test.malloc(")
        .replace("call void @free(", "call void @owner.test.free(")
        .replace("call void @exit(", "call void @owner.test.exit(");
    let instrumentation = OWNER_GUARDS
        .replace("EXPECTED_ACQUISITIONS", &fixture.acquisitions.to_string())
        .replace(
            "FAILED_CAPACITY",
            &fixture.failed_capacity.unwrap_or(0).to_string(),
        )
        .replace("EXPECTED_FAULTS", &fixture.expected_faults.to_string());
    guarded.push_str(&instrumentation);
    guarded
}

/// LLVM instrumentation observes backing only; source record operations remain unchanged.
/// Final checks run both before explicit exit and from a destructor after normal main return.
const OWNER_GUARDS: &str = r#"
; Test-only owner state has fixed capacity independent of records or payload sizes.
@owner.test.stack = internal global [16 x ptr] zeroinitializer
@owner.test.count = internal global i64 0
@owner.test.acquisitions = internal global i64 0
@owner.test.frees = internal global i64 0
@owner.test.faults = internal global i64 0
@owner.test.failed.capacity = internal global i64 FAILED_CAPACITY
@llvm.global_dtors = appending global [1 x { i32, ptr, ptr }] [{ i32, ptr, ptr } { i32 65535, ptr @owner.test.finish, ptr null }]

; Return actual allocation results except for the one externally opaque fault request.
define internal ptr @owner.test.malloc(i64 %size) noinline {
entry:
  %failed.size = load volatile i64, ptr @owner.test.failed.capacity
  %enabled = icmp ne i64 %failed.size, 0
  %matches = icmp eq i64 %failed.size, %size
  %inject = and i1 %enabled, %matches
  ; Injection returns NULL into the original acquisition failure branch, without an owner.
  br i1 %inject, label %fault, label %allocate
fault:
  %old.faults = load i64, ptr @owner.test.faults
  %new.faults = add i64 %old.faults, 1
  store i64 %new.faults, ptr @owner.test.faults
  ret ptr null
allocate:
  %base = call ptr @malloc(i64 %size)
  %present = icmp ne ptr %base, null
  ; A genuine host allocation failure also never enters the active-owner stack.
  br i1 %present, label %acquired, label %done
acquired:
  %count = load i64, ptr @owner.test.count
  %fits = icmp ult i64 %count, 16
  call void @holo.assert(i1 %fits)
  %slot = getelementptr [16 x ptr], ptr @owner.test.stack, i64 0, i64 %count
  store ptr %base, ptr %slot
  %next = add i64 %count, 1
  store i64 %next, ptr @owner.test.count
  %old.total = load i64, ptr @owner.test.acquisitions
  %new.total = add i64 %old.total, 1
  store i64 %new.total, ptr @owner.test.acquisitions
  br label %done
done:
  ret ptr %base
}

; Release only the most recently acquired live owner, then invoke the actual allocator free.
define internal void @owner.test.free(ptr %base) noinline {
entry:
  %count = load i64, ptr @owner.test.count
  %nonempty = icmp ugt i64 %count, 0
  call void @holo.assert(i1 %nonempty)
  %next = sub i64 %count, 1
  %slot = getelementptr [16 x ptr], ptr @owner.test.stack, i64 0, i64 %next
  %expected = load ptr, ptr %slot
  %matches = icmp eq ptr %base, %expected
  call void @holo.assert(i1 %matches)
  store i64 %next, ptr @owner.test.count
  %old.total = load i64, ptr @owner.test.frees
  %new.total = add i64 %old.total, 1
  store i64 %new.total, ptr @owner.test.frees
  call void @free(ptr %base)
  ret void
}

; Both normal completion and source early return require all and only acquired owners freed.
define internal void @owner.test.finish() noinline {
entry:
  %count = load i64, ptr @owner.test.count
  %empty = icmp eq i64 %count, 0
  call void @holo.assert(i1 %empty)
  %acquired = load i64, ptr @owner.test.acquisitions
  %all.acquired = icmp eq i64 %acquired, EXPECTED_ACQUISITIONS
  call void @holo.assert(i1 %all.acquired)
  %freed = load i64, ptr @owner.test.frees
  %all.freed = icmp eq i64 %freed, EXPECTED_ACQUISITIONS
  call void @holo.assert(i1 %all.freed)
  %faults = load i64, ptr @owner.test.faults
  %all.faults = icmp eq i64 %faults, EXPECTED_FAULTS
  call void @holo.assert(i1 %all.faults)
  ret void
}

; Preserve ordinary exit flushing after checking active ownership before process shutdown.
define internal void @owner.test.exit(i32 %status) noinline noreturn {
entry:
  call void @owner.test.finish()
  call void @exit(i32 %status)
  unreachable
}
"#;
