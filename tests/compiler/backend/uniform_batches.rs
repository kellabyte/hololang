//! Differential checks for the opt-in uniform published batch extent proof.
//!
//! The test uses the same checked records and append failures under both policies.
//! Uniform cursors may skip length metadata reads, but their bounds, captured
//! end, rollback and reset must still agree with the ordinary emitter.

use std::{fs, os::unix::process::ExitStatusExt, path::Path, process::Command};

use holo::{
    check::{self, CheckedProgram},
    driver::{NativeToolchain, TempDir},
    frontend::parse_experimental,
    ir::{
        Access, Argument, Build, Expr, Field, FieldKind, Op, Parameter, Producer, Program, Record,
    },
    layout::LayoutMode,
    llvm,
};

/// The trial target used for portable LLVM selection tests.
const TARGET: &str = "arm64-apple-macosx14.0.0";

/// A homogeneous slot producer with an early typed rejection and scalar seed.
/// The entry checks captured traversal, marker rollback and reuse in the same
/// lexical owner. Observable byte values distinguish all live records.
const SOURCE: &str = r#"
struct Slot { wire Bytes }
error SlotError { Refused }

func slot with noblock arg length u64 as readonly copy arg seed u64 as readonly copy returns Result<Slot, SlotError> {
    if seed == 99 { return error SlotError.Refused }
    return ok Slot:
        wire = core.bytes(length, seed)
}

func main returns u64 {
    let memory = core.region(20) else { core.assert(false); return 0 }
    let rows = core.batch(memory, Slot)
    let first = core.append(rows, slot(2, 11))
        on ok then true
        on error then false
    core.assert(first)
    let rejected = core.append(rows, slot(2, 99))
        on ok then false
        on Rejected SlotError.Refused then true
        on error then false
    core.assert(rejected)
    core.assert(core.len(rows) == 1)
    core.assert(core.used(memory) == 10)
    var visited = 0
    for readwrite row in rows {
        let later = core.append(rows, slot(2, 33))
            on ok then true
            on error then false
        core.assert(later)
        core.assert(core.len(row.wire) == 2)
        row.wire[1] = 77
        visited = core.add_wrapping(visited, 1)
    }
    core.assert(visited == 1)
    let full = core.append(rows, slot(2, 44))
        on ok then false
        on Full then true
        on error then false
    core.assert(full)
    core.assert(core.len(rows) == 2)
    core.assert(core.used(memory) == 20)
    for readonly row in rows {
        core.print(row.wire[0])
        core.print(row.wire[1])
    }
    core.reset(rows)
    core.assert(core.len(rows) == 0)
    core.assert(core.used(memory) == 0)
    let reused = core.append(rows, slot(2, 55))
        on ok then true
        on error then false
    core.assert(reused)
    for readonly row in rows { core.print(row.wire[1]) }
    return 0
}
"#;

/// Both branch bodies publish. A zero-length payload still occupies one
/// length header, so a proved cursor must progress twice and skip an empty batch.
const ZERO_LENGTH: &str = r#"
struct Slot { wire Bytes }
func slot with noblock arg length u64 as readonly copy returns Slot {
    return Slot { wire: core.bytes(length, 41) }
}
func main returns u64 {
    let memory = core.region(18) else { core.assert(false); return 0 }
    let rows = core.batch(memory, Slot)
    for readonly empty in rows { core.assert(false) }
    for pass in core.range(2) {
        if pass == 0 {
            let first = core.append(rows, slot(0))
            core.assert(first == 0)
        } else {
            let second = core.append(rows, slot(0))
            core.assert(second == 0)
        }
    }
    var visited = 0
    for readonly row in rows {
        visited = core.add_wrapping(visited, 1)
        core.assert(visited <= 2)
        core.print(core.len(row.wire))
    }
    core.assert(visited == 2)
    core.print(core.used(memory))
    return 0
}
"#;

/// A separate proved refill buffer feeds two positive file reads followed by
/// EOF. Output records can remain uniform or use a longer EOF payload.
const HOSTED: &str = r#"
struct Buffer { wire Bytes }
struct Slot { wire Bytes }
func buffer with noblock returns Buffer { return Buffer { wire: core.bytes(4, 99) } }
func copy_slot with noblock arg length u64 as readonly copy arg input Bytes as readonly reference returns Slot {
    return Slot { wire: core.bytes(length, input[core.index()]) }
}
func eof_slot with noblock arg length u64 as readonly copy returns Slot {
    return Slot { wire: core.bytes(length, 42) }
}
func main returns u64 {
    let scratch_memory = core.region(12) else { core.assert(false); return 0 }
    let scratch = core.batch(scratch_memory, Buffer)
    let seeded = core.append(scratch, buffer())
    core.assert(seeded == 0)
    let memory = core.region(28) else { core.assert(false); return 0 }
    let rows = core.batch(memory, Slot)
    for readwrite pending in scratch {
        for attempt in core.range(3) {
            refill pending.wire, 1 as fresh {
                let copied = core.append(rows, copy_slot(1, fresh))
                core.assert(copied == 0)
            } else status {
                core.assert(status == 0)
                let ended = core.append(rows, eof_slot(1))
                core.assert(ended == 0)
            }
        }
    }
    for readonly row in rows {
        core.print(core.len(row.wire))
        core.print(row.wire[0])
    }
    core.print(core.used(memory))
    return 0
}
"#;

/// Emit both policies from the same standalone checked source and target.
/// No native tools or execution are needed to inspect the emitted LLVM.
fn emissions(source: &str, target: &str, layout: LayoutMode) -> (String, String) {
    let parsed = parse_experimental(source).unwrap();
    let checked = parsed.check().unwrap();
    checked_emissions(&checked, target, layout)
}

/// Emit both policies through the same verified representation and layout.
/// The resulting LLVM is owned by the caller and may be inspected or built.
fn checked_emissions(
    checked: &CheckedProgram<'_>,
    target: &str,
    layout: LayoutMode,
) -> (String, String) {
    (
        llvm::emit_with_layout(checked, target, layout).unwrap(),
        llvm::emit_with_uniform_batch_extents(checked, target, layout).unwrap(),
    )
}

/// Native behaviors observed after a checked fixture is built.
enum Outcome<'a> {
    /// Exit successfully with the exact decimal observation stream.
    Output(&'a [u8]),
    /// Reject an invalid index with the target trap before accessing memory.
    Trap,
}

/// Find LLVM on the only host where this experiment can execute its fast path.
/// Unsupported hosts still execute the portable LLVM emission assertions.
fn native_tools() -> Option<NativeToolchain> {
    // The trial selects only the Darwin arm64 Derived64 profile.
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(NativeToolchain::discover().unwrap())
    } else {
        None
    }
}

/// Run ordinary O3 and candidate O0, O3, and ASan with the same input.
/// Each build owns fresh temporary output. The optional regular file is opened
/// by the production hosted driver, and fixture observations use an exact oracle.
fn check_native(
    tools: &NativeToolchain,
    ordinary: &str,
    candidate: &str,
    file: Option<&Path>,
    outcome: Outcome<'_>,
) {
    let scratch = TempDir::new().unwrap();
    // The control and each candidate mode get independent native builds.
    for (name, ir, optimized) in [
        ("ordinary-o3", ordinary, true),
        ("uniform-o0", candidate, false),
        ("uniform-o3", candidate, true),
        ("uniform-asan", candidate, true),
    ] {
        let binary = scratch.path().join(name);
        // Keep ASan instrumentation out of the plain native builds.
        if name == "uniform-asan" {
            tools.build_sanitized(ir, &binary).unwrap();
        } else {
            tools.build(ir, &binary, optimized).unwrap();
        }
        let mut command = Command::new(binary);
        // Standalone fixtures have no hosted adapter or arguments.
        if let Some(file) = file {
            command.arg("file").arg(file);
        }
        let output = command
            .env("ASAN_OPTIONS", "detect_leaks=0")
            .output()
            .unwrap();
        assert!(output.stderr.is_empty(), "{name}: {output:?}");
        // Checked failures take the target trap. Valid runs must match every byte.
        match outcome {
            Outcome::Output(expected) => {
                assert!(output.status.success(), "{name}: {output:?}");
                assert_eq!(output.stdout, expected, "{name}");
            }
            Outcome::Trap => {
                assert_eq!(output.status.signal(), Some(5), "{name}: {output:?}");
                assert!(output.stdout.is_empty(), "{name}: {output:?}");
            }
        }
    }
}

/// Homogeneous sites remove the variable hop. An unknown length or a
/// differently sized append, even inside a branch, keeps the exact baseline.
#[test]
fn proof_requires_every_possible_publication_to_match() {
    let (ordinary, candidate) = emissions(SOURCE, TARGET, LayoutMode::Derived64);
    // The indexed lookup helper retains its generic hop; only three cursor
    // traversals stop loading physical record sizes.
    let hop = "call i64 @holo.record_size.0(";
    assert_eq!(ordinary.matches(hop).count(), 4);
    assert_eq!(candidate.matches(hop).count(), 1);
    let mixed = SOURCE.replace(
        "    core.assert(first)",
        "    core.assert(first)\n    if core.len(rows) == 0 {\n        let ignored = core.append(rows, slot(3, 0))\n            on ok then true\n            on error then false\n        core.assert(ignored)\n    }",
    );
    let mixed_result = SOURCE
        .replace(
            "func main returns u64",
            "error ChoiceError { Refused }\nfunc choice returns Result<u64, ChoiceError> { return ok 0 }\nfunc main returns u64",
        )
        .replace(
            "    core.assert(first)",
            "    core.assert(first)\n    choice()\n        on ok then {}\n        on Refused then {\n            let ignored = core.append(rows, slot(3, 0))\n                on ok then true\n                on error then false\n            core.assert(ignored)\n        }",
        );
    let unknown = SOURCE.replace("slot(2, 11)", "slot(core.len(rows), 11)");
    // The ordinary policy is also exact for the unsupported target/layouts.
    for (source, target, layout) in [
        (mixed.as_str(), TARGET, LayoutMode::Derived64),
        (mixed_result.as_str(), TARGET, LayoutMode::Derived64),
        (unknown.as_str(), TARGET, LayoutMode::Derived64),
        (SOURCE, TARGET, LayoutMode::Wide),
        (SOURCE, TARGET, LayoutMode::Compact32),
        (SOURCE, "aarch64-unknown-linux-musl", LayoutMode::Derived64),
    ] {
        let (ordinary, candidate) = emissions(source, target, layout);
        assert_eq!(candidate, ordinary);
    }
}

/// Native qualification preserves payloads, suffix rollback and snapshot
/// iteration. A read just outside the proved extent still traps before access.
#[test]
fn uniform_cursor_preserves_snapshot_rollback_and_byte_bounds() {
    // Other hosts cover emitted policy selection in the portable test above.
    let Some(tools) = native_tools() else {
        return;
    };
    let out_of_bounds = SOURCE.replace("row.wire[1] = 77", "core.print(row.wire[2])");
    for (source, outcome) in [
        (SOURCE, Outcome::Output(b"11\n77\n33\n33\n55\n")),
        (out_of_bounds.as_str(), Outcome::Trap),
    ] {
        let (ordinary, candidate) = emissions(source, tools.target_triple(), LayoutMode::Derived64);
        check_native(&tools, &ordinary, &candidate, None, outcome);
    }
}

/// Empty records still step by their length header. Both live if arms count
/// toward the proof, so one differing arm must retain variable traversal.
#[test]
fn zero_length_progresses_and_live_mixed_branches_fall_back() {
    let hop = "call i64 @holo.record_size.0(";
    let (ordinary, candidate) = emissions(ZERO_LENGTH, TARGET, LayoutMode::Derived64);
    assert_eq!(ordinary.matches(hop).count(), 3);
    assert_eq!(candidate.matches(hop).count(), 1);
    let mixed = ZERO_LENGTH.replace(
        "let second = core.append(rows, slot(0))",
        "let second = core.append(rows, slot(2))",
    );
    let (ordinary, candidate) = emissions(&mixed, TARGET, LayoutMode::Derived64);
    assert_eq!(ordinary, candidate);

    // The host check executes both live branches with an exact count and footprint.
    let Some(tools) = native_tools() else {
        return;
    };
    for (source, expected) in [
        (ZERO_LENGTH, b"0\n0\n16\n".as_slice()),
        (mixed.as_str(), b"0\n2\n18\n".as_slice()),
    ] {
        let (ordinary, candidate) = emissions(source, tools.target_triple(), LayoutMode::Derived64);
        check_native(
            &tools,
            &ordinary,
            &candidate,
            None,
            Outcome::Output(expected),
        );
    }
}

/// Both refill continuations constrain the destination batch. A differing EOF
/// extent keeps the output cursor dynamic while the separate input specializes.
#[test]
fn hosted_refill_keeps_published_bytes_and_mixed_shapes() {
    let input_hop = "call i64 @holo.record_size.0(";
    let output_hop = "call i64 @holo.record_size.1(";
    let mixed = HOSTED.replace("eof_slot(1)", "eof_slot(2)");
    for (source, output_hops) in [(HOSTED, 1), (mixed.as_str(), 2)] {
        let (ordinary, candidate) = emissions(source, TARGET, LayoutMode::Derived64);
        // Each generic indexed helper retains one size call. The refill
        // cursor has its own proof even when output publication is mixed.
        assert_eq!(ordinary.matches(input_hop).count(), 2);
        assert_eq!(ordinary.matches(output_hop).count(), 2);
        assert_eq!(candidate.matches(input_hop).count(), 1);
        assert_eq!(candidate.matches(output_hop).count(), output_hops);
    }

    // The real adapter reuses one buffer twice, then observes file EOF.
    let Some(tools) = native_tools() else {
        return;
    };
    let scratch = TempDir::new().unwrap();
    let file = scratch.path().join("input.bin");
    fs::write(&file, [7, 8]).unwrap();
    for (source, expected) in [
        (HOSTED, b"1\n7\n1\n8\n1\n42\n27\n".as_slice()),
        (mixed.as_str(), b"1\n7\n1\n8\n2\n42\n28\n".as_slice()),
    ] {
        let (ordinary, candidate) = emissions(source, tools.target_triple(), LayoutMode::Derived64);
        check_native(
            &tools,
            &ordinary,
            &candidate,
            Some(&file),
            Outcome::Output(expected),
        );
    }
}

/// Construct checked IR that rebinds the current cursor reference to a second
/// record in the same batch. Parsed source keeps its iterator name immutable.
fn rebind_cursor() -> Program {
    // Both literal initializers publish equal shape with distinguishable bytes.
    let append = |seed, status: &str| Op::Append {
        batch: "rows".into(),
        producer: "slot".into(),
        args: vec![Argument::Scalar(Expr::Const(seed))],
        status: status.into(),
    };
    // Read the referenced payload through the ordinary checked projection.
    let read = |reference: &str| {
        Op::Print(Expr::Byte {
            reference: reference.into(),
            path: vec!["wire".into()],
            index: Box::new(Expr::Const(0)),
        })
    };
    Program {
        records: vec![Record {
            name: "Slot".into(),
            fields: vec![Field {
                name: "wire".into(),
                kind: FieldKind::Bytes,
            }],
        }],
        producers: vec![Producer {
            name: "slot".into(),
            record: "Slot".into(),
            error: None,
            params: vec![Parameter::U64],
            body: vec![Build::Bytes {
                field: "wire".into(),
                len: Expr::Const(1),
                value: Expr::Param(0),
            }],
        }],
        body: vec![Op::Region {
            name: "memory".into(),
            capacity: Expr::Const(18),
            fail_acquire: false,
            body: vec![Op::Batch {
                name: "rows".into(),
                region: "memory".into(),
                record: "Slot".into(),
                body: vec![
                    append(11, "first_status"),
                    append(22, "second_status"),
                    Op::Borrow {
                        name: "last".into(),
                        batch: "rows".into(),
                        index: Expr::Const(1),
                        access: Access::Readonly,
                    },
                    Op::ForEach {
                        batch: "rows".into(),
                        reference: "row".into(),
                        access: Access::Readonly,
                        body: vec![
                            Op::Alias {
                                name: "saved".into(),
                                source: "row".into(),
                                access: Access::Readonly,
                            },
                            read("row"),
                            Op::AssignRef {
                                name: "row".into(),
                                source: "last".into(),
                            },
                            read("row"),
                            read("saved"),
                        ],
                    },
                ],
            }],
            on_failure: vec![Op::Print(Expr::Const(999))],
        }],
    }
}

/// A rebind changes the live cursor reference, while its saved alias and
/// physical advance remain tied to the original element.
#[test]
fn cursor_rebinding_keeps_aliases_and_snapshot_order() {
    let program = rebind_cursor();
    let checked = check::check(&program).unwrap();
    let (ordinary, candidate) = checked_emissions(&checked, TARGET, LayoutMode::Derived64);
    let hop = "call i64 @holo.record_size.0(";
    assert_eq!(ordinary.matches(hop).count(), 2);
    assert_eq!(candidate.matches(hop).count(), 1);

    // Native observations distinguish the rebinding from the independent hop.
    let Some(tools) = native_tools() else {
        return;
    };
    let (ordinary, candidate) =
        checked_emissions(&checked, tools.target_triple(), LayoutMode::Derived64);
    check_native(
        &tools,
        &ordinary,
        &candidate,
        None,
        Outcome::Output(b"11\n22\n11\n22\n22\n22\n"),
    );
}
