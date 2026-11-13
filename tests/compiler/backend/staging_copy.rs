//! Differential coverage for the opt-in checked byte-field staging transfer.
//!
//! The same checked program supplies the scalar oracle and candidate LLVM.
//! Cases cover exact subranges, empty loops, forward-visible overlap, and
//! partial writes before the original fatal guard. Only the diagnostic build
//! adds output around an otherwise unobservable trap. The wide outlined
//! transfer also checks every byte of a multi-block copy and its partial tail.

#[path = "../../support/model/mod.rs"]
mod model;

use std::{fmt::Write, os::unix::process::ExitStatusExt, process::Command};

use holo::{
    driver::{NativeToolchain, TempDir},
    frontend::parse_experimental,
    layout::LayoutMode,
    llvm,
};

/// Physical modes whose metadata must never be copied as payload.
const LAYOUTS: [LayoutMode; 3] = [
    LayoutMode::Wide,
    LayoutMode::Derived64,
    LayoutMode::Compact32,
];

/// Standalone source with inline nesting and separately owned source/destination.
const DEFINITION: &str = r#"
struct Payload { data Bytes }
struct Record { prefix Bytes, payload Payload }

func make_payload with noblock arg length u64 as readonly copy arg seed u64 as readonly copy returns Payload {
    return Payload { data: core.bytes(length, core.add_wrapping(seed, core.index())) }
}
func make with noblock arg length u64 as readonly copy arg seed u64 as readonly copy returns Record {
    return Record { prefix: core.bytes(3, 71), payload: make_payload(length, seed) }
}
"#;

/// Instantiate two independent bounded records and one readonly destination alias.
/// The source owns no state except the supplied staging body and fixed byte seeds.
fn fixture(source_length: u64, destination_length: u64, body: &str) -> String {
    format!(
        r#"{DEFINITION}
func main returns u64 {{
    let a = core.region(512) else {{ core.assert(false); return 0 }}
    let b = core.region(512) else {{ core.assert(false); return 0 }}
    let input = core.batch(a, Record)
    let output = core.batch(b, Record)
    let status = core.append(input, make({source_length}, 9))
    core.assert(status == 0)
    let status_out = core.append(output, make({destination_length}, 100))
    core.assert(status_out == 0)
    let src = core.borrow_readonly(input, 0)
    let dst = core.borrow_readwrite(output, 0)
    let alias = core.alias_readonly(dst)
{body}
    return 0
}}
"#
    )
}

/// Full-range probes preserve output and alias behavior in every physical layout.
#[test]
fn staging_copy_preserves_disjoint_and_overlapping_forward_semantics() {
    let source = fixture(
        129,
        129,
        r#"
    let one = 1
    let three = 3
    // Exercise the short path with both operand orders and nonzero starts.
    for index in core.range(64) {
        dst.payload.data[core.add_wrapping(one, index)] = src.payload.data[core.add_wrapping(index, three)]
    }
    // The bulk path ends exactly at each field's extent.
    for index in core.range(65) {
        dst.payload.data[core.add_wrapping(64, index)] = src.payload.data[core.add_wrapping(index, 64)]
    }
    core.print(dst.prefix[2])
    core.print(dst.payload.data[0])
    core.print(dst.payload.data[1])
    core.print(dst.payload.data[63])
    core.print(dst.payload.data[64])
    core.print(dst.payload.data[128])
    // Empty loops must skip even out-of-bounds starts and source reads.
    for index in core.range(0) {
        dst.payload.data[core.add_wrapping(18446744073709551615, index)] = src.payload.data[core.add_wrapping(18446744073709551615, index)]
    }
    // Forward overlap propagates the first byte, unlike a snapshot copy.
    for index in core.range(5) {
        dst.payload.data[core.add_wrapping(1, index)] = alias.payload.data[index]
    }
    // Disjoint intervals may share a backing and even the same field.
    for index in core.range(5) {
        dst.payload.data[index] = alias.payload.data[core.add_wrapping(6, index)]
    }
    // Overlap in the other direction still reads the live byte on each step.
    for index in core.range(5) {
        dst.payload.data[index] = alias.payload.data[core.add_wrapping(1, index)]
    }
    for index in core.range(8) {
        core.print(dst.payload.data[index])
    }
    // A tiny transfer uses the scalar loop even though its shape matches.
    let tiny = 2
    for index in core.range(tiny) {
        dst.payload.data[core.add_wrapping(1, index)] = alias.payload.data[index]
    }
    core.print(dst.payload.data[1])
    core.print(dst.payload.data[2])
"#,
    );
    let parsed = parse_experimental(&source).unwrap();
    let checked = parsed.check().unwrap();
    let expected = [
        71, 100, 12, 74, 73, 137, 18, 19, 20, 21, 100, 100, 17, 18, 18, 18,
    ];
    let expected_text = expected.iter().fold(String::new(), |mut text, value| {
        writeln!(text, "{value}").unwrap();
        text
    });
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();

    // The actual checked target has an opt-in. Other targets emit the same
    // module as the ordinary compiler even when the experiment is requested.
    let unsupported = "aarch64-unknown-linux-musl";
    assert_eq!(
        llvm::emit_with_staging_copy(&checked, unsupported, LayoutMode::Derived64).unwrap(),
        llvm::emit_with_layout(&checked, unsupported, LayoutMode::Derived64).unwrap(),
    );

    // The oracle validates the original loop semantics independently, including
    // exactly how forward overlap changes each later source read.
    for layout in LAYOUTS {
        let oracle = match layout {
            LayoutMode::Wide => model::run(parsed.program()),
            _ => model::run_with_layout(parsed.program(), layout),
        };
        assert_eq!(oracle.unwrap(), expected);
        let baseline = llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        let candidate =
            llvm::emit_with_staging_copy(&checked, tools.target_triple(), layout).unwrap();
        assert!(!baseline.contains("; Holo checked staging copy"));
        // Top-level copies have no enclosing descriptor proof. In the release
        // candidate their lowering, including overlap, matches ordinary LLVM.
        assert_eq!(
            llvm::emit_with_proved_staging_slots(&checked, tools.target_triple(), layout).unwrap(),
            baseline
        );
        // Native witnesses run everywhere. The experimental path is admitted
        // only on the supported macOS arm64 target.
        if tools.target_triple().starts_with("arm64-apple-macosx") {
            assert_eq!(candidate.matches("; Holo checked staging copy").count(), 7);
            assert!(candidate.contains("llvm.memcpy.inline"));
            assert!(!candidate.contains("noalias"));
        }
        // Build each optimized or instrumented binary independently from the same IR.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch.path().join(format!("valid-{layout:?}-{mode}"));
            match mode {
                "asan" => tools.build_sanitized(&candidate, &binary),
                _ => tools.build(&candidate, &binary, mode == "o3"),
            }
            .unwrap();
            let output = Command::new(binary)
                .env("ASAN_OPTIONS", "detect_leaks=0")
                .output()
                .unwrap();
            assert!(output.status.success(), "{layout:?} {mode}: {output:?}");
            assert_eq!(output.stdout, expected_text.as_bytes());
        }
    }
}

/// Stable descriptors cover nested tiny copies while zero trips skip the body.
/// Each layout has different metadata widths and nested payload offsets.
#[test]
fn stable_staging_slots_preserve_tiny_copy_and_empty_outer_loop() {
    let source = fixture(
        4,
        5,
        r#"
    // A zero-trip wrapper must bypass the staged fields and invalid indices.
    for unused in core.range(0) {
        for index in core.range(1) {
            dst.payload.data[core.add_wrapping(99, index)] = src.payload.data[core.add_wrapping(99, index)]
        }
    }
    // Exercise each tiny geometry using one pair of stable published fields.
    for fragment in core.range(4) {
        let count = core.add_wrapping(fragment, 1)
        for index in core.range(count) {
            dst.payload.data[index] = src.payload.data[index]
        }
    }
    // The last destination byte is outside the copied interval.
    for index in core.range(5) {
        core.print(dst.payload.data[index])
    }
"#,
    );
    let parsed = parse_experimental(&source).unwrap();
    let checked = parsed.check().unwrap();
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let unsupported = "aarch64-unknown-linux-musl";
    assert_eq!(
        llvm::emit_with_stable_staging_slots(&checked, unsupported, LayoutMode::Derived64).unwrap(),
        llvm::emit_with_layout(&checked, unsupported, LayoutMode::Derived64).unwrap(),
    );
    assert_eq!(
        llvm::emit_with_proved_staging_slots(&checked, unsupported, LayoutMode::Derived64).unwrap(),
        llvm::emit_with_layout(&checked, unsupported, LayoutMode::Derived64).unwrap(),
    );

    // Compare the checked scalar model and candidate at every physical layout.
    for layout in LAYOUTS {
        let oracle = match layout {
            LayoutMode::Wide => model::run(parsed.program()),
            _ => model::run_with_layout(parsed.program(), layout),
        };
        assert_eq!(oracle.unwrap(), [9, 10, 11, 12, 104]);
        let target = tools.target_triple();
        let candidate = llvm::emit_with_proved_staging_slots(&checked, target, layout).unwrap();
        // Both policies use the same proved path when all staging sites qualify.
        assert_eq!(
            candidate,
            llvm::emit_with_stable_staging_slots(&checked, target, layout).unwrap()
        );
        // The preheader must guard the complete zero-trip body before capture.
        if target.starts_with("arm64-apple-macosx") {
            assert_eq!(candidate.matches("; Holo stable staging slots").count(), 2);
            let (prefix, _) = candidate.split_once("; Holo stable staging slots").unwrap();
            assert!(
                prefix.ends_with(":\n  ")
                    && prefix
                        .lines()
                        .rev()
                        .take(4)
                        .any(|line| { line.contains("icmp ne i64 0, 0") })
            );
            assert!(!candidate.contains("noalias"));
        }
        // Compile and run the exact module at both optimization modes and ASan.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch.path().join(format!("stable-{layout:?}-{mode}"));
            match mode {
                "asan" => tools.build_sanitized(&candidate, &binary),
                _ => tools.build(&candidate, &binary, mode == "o3"),
            }
            .unwrap();
            let output = Command::new(binary)
                .env("ASAN_OPTIONS", "detect_leaks=0")
                .output()
                .unwrap();
            assert!(output.status.success(), "{layout:?} {mode}: {output:?}");
            assert_eq!(output.stdout, b"9\n10\n11\n12\n104\n");
        }
    }
}

/// A cursor shadow or reference assignment can select a different source.
/// The stable-slot mode must retain the guarded loop when a saved error
/// selects the assignment.
#[test]
fn stable_staging_slots_reject_cursor_shadow_and_rebind() {
    let cursor = fixture(
        8,
        8,
        r#"
    for pass in core.range(1) {
        // This cursor shadows the distinct-region source with the receiver.
        for readonly src in output {
            for index in core.range(6) {
                dst.payload.data[core.add_wrapping(1, index)] = src.payload.data[index]
            }
        }
    }
    core.print(dst.payload.data[6])
"#,
    );
    let rebound = fixture(
        8,
        8,
        r#"
    let added = core.append(input, make(8, 40))
    core.assert(added == 0)
    let next = core.borrow_readonly(input, 1)
    for pass in core.range(1) {
        // A same-root rebind still changes the observed payload.
        src = next
        for index in core.range(6) {
            dst.payload.data[core.add_wrapping(1, index)] = src.payload.data[index]
        }
    }
    core.print(dst.payload.data[6])
"#,
    )
    .replace("let src =", "var src =");
    let saved_error = format!(
        "error LookupError {{ Changed(seed u64) }}\n\
         func selected returns Result<u64, LookupError> {{ return error LookupError.Changed(40) }}\n{}",
        rebound.replace(
            "src = next",
            "let choice = selected()\n        choice\n            on ok then {}\n            on Changed(seed) then { if seed == 40 { src = next } }",
        )
    );
    // Each case uses the existing per-transfer fallback, even on the trial's target.
    for (source, expected) in [(&cursor, 100), (&rebound, 45), (&saved_error, 45)] {
        let parsed = parse_experimental(source).unwrap();
        let checked = parsed.check().unwrap();
        assert_eq!(model::run(parsed.program()).unwrap(), [expected]);
        let target = "arm64-apple-macosx11.0.0";
        let guarded =
            llvm::emit_with_staging_copy(&checked, target, LayoutMode::Derived64).unwrap();
        let candidate =
            llvm::emit_with_stable_staging_slots(&checked, target, LayoutMode::Derived64).unwrap();
        assert!(!candidate.contains("; Holo stable staging slots"));
        assert_eq!(candidate, guarded);
        // Rebinding and lexical shadowing leave the proved-only candidate
        // byte-identical to ordinary emission, with no per-transfer guards.
        assert_eq!(
            llvm::emit_with_proved_staging_slots(&checked, target, LayoutMode::Derived64).unwrap(),
            llvm::emit_with_layout(&checked, target, LayoutMode::Derived64).unwrap()
        );
    }
}

/// An enclosing proof covers its selected pair while a same-slot forward copy
/// under the very same preheader retains scalar semantics.
#[test]
fn proved_staging_slots_leave_unrelated_transfer_scalar() {
    let source = fixture(
        8,
        8,
        r#"
    for pass in core.range(2) {
        // Separate live regions admit a cached descriptor pair.
        for index in core.range(6) {
            dst.payload.data[index] = src.payload.data[index]
        }
        // Same backing propagates the first byte through the copied prefix.
        for index in core.range(6) {
            dst.payload.data[core.add_wrapping(1, index)] = alias.payload.data[index]
        }
    }
    core.print(dst.payload.data[0])
    core.print(dst.payload.data[6])
    core.print(dst.payload.data[7])
"#,
    );
    let parsed = parse_experimental(&source).unwrap();
    let checked = parsed.check().unwrap();
    let tools = NativeToolchain::discover().unwrap();
    let target = tools.target_triple();
    let scratch = TempDir::new().unwrap();
    // Cover packed layouts whose byte payload offsets differ.
    for layout in LAYOUTS {
        assert_eq!(
            model::run_with_layout(parsed.program(), layout).unwrap(),
            [9, 9, 107]
        );
        let candidate = llvm::emit_with_proved_staging_slots(&checked, target, layout).unwrap();
        // The nested same-backing copy must emit no guarded transfer even
        // though the enclosing loop holds the selected pair's descriptors.
        if target.starts_with("arm64-apple-macosx") {
            assert_eq!(candidate.matches("; Holo stable staging slots").count(), 1);
            assert_eq!(candidate.matches("; Holo checked staging copy").count(), 1);
        }
        let binary = scratch.path().join(format!("proved-only-{layout:?}"));
        tools.build_sanitized(&candidate, &binary).unwrap();
        let output = Command::new(binary)
            .env("ASAN_OPTIONS", "detect_leaks=0")
            .output()
            .unwrap();
        assert!(output.status.success(), "{layout:?}: {output:?}");
        assert_eq!(output.stdout, b"9\n9\n107\n");
    }
}

/// Make only the emitted fallback's three guards and one store observable.
/// This diagnostic records committed bytes immediately with `write` and gives
/// destination and source traps different exit codes. Ordinary code uses BRK.
fn observe_scalar_fallback(candidate: &str) -> String {
    let (prefix, suffix) = candidate
        .split_once("; Holo checked staging copy\n")
        .unwrap();
    let mut result = prefix.to_owned();
    let mut assertions = 0;
    let mut stores = 0;
    // Exactly one assignment follows the marker, with two destination guards
    // and then one source guard. Do not instrument constructor or setup work.
    for line in suffix.lines() {
        if assertions < 3
            && let Some(guard) = line.trim().strip_prefix("call void @holo.assert(i1 ")
        {
            let condition = guard.strip_suffix(')').unwrap();
            let exit = if assertions < 2 { 85 } else { 86 };
            writeln!(
                result,
                "  call void @holo.staging_guard(i1 {condition}, i32 {exit})"
            )
            .unwrap();
            assertions += 1;
        } else {
            writeln!(result, "{line}").unwrap();
            // The diagnostic exposes exactly the byte which this iteration
            // stored. No read or write from the failed iteration is invented.
            if line.trim().starts_with("store i8 ") {
                let (_, pointer) = line.split_once(", ptr ").unwrap();
                let pointer = pointer.strip_suffix(", align 1").unwrap();
                writeln!(result, "  call i64 @write(i32 1, ptr {pointer}, i64 1)").unwrap();
                stores += 1;
            }
        }
    }
    assert_eq!((assertions, stores), (3, 1));
    result.push_str(
        r#"
declare i64 @write(i32, ptr, i64)
define internal void @holo.staging_guard(i1 %ok, i32 %code) {
entry:
  br i1 %ok, label %valid, label %invalid
valid:
  ret void
invalid:
  call void @exit(i32 %code)
  unreachable
}
"#,
    );
    result
}

/// Invalid fast-path intervals replay earlier writes and original trap precedence.
#[test]
fn staging_copy_fallback_preserves_late_failure_and_trap_order() {
    let tools = NativeToolchain::discover().unwrap();
    // The production Linux emitter has no experimental path to compare here.
    if !tools.target_triple().starts_with("arm64-apple-macosx") {
        return;
    }
    let scratch = TempDir::new().unwrap();
    let body = r#"
    for index in core.range(6) {
        dst.payload.data[index] = src.payload.data[index]
    }
"#;
    // A short source fails at the sixth read. A short destination fails first
    // at that iteration even when its source index would also be invalid.
    for (destination_len, exit_code) in [(6, 86), (5, 85)] {
        // The wrapper admits stable descriptors. The scalar fallback must still
        // perform five valid writes before the original check rejects byte six.
        let wrapped = format!("for pass in core.range(1) {{\n{body}\n}}");
        let source = fixture(5, destination_len, &wrapped);
        let parsed = parse_experimental(&source).unwrap();
        let checked = parsed.check().unwrap();
        for layout in LAYOUTS {
            let candidate =
                llvm::emit_with_proved_staging_slots(&checked, tools.target_triple(), layout)
                    .unwrap();
            assert_eq!(candidate.matches("; Holo stable staging slots").count(), 1);
            assert_eq!(candidate.matches("; Holo checked staging copy").count(), 1);
            let observed = observe_scalar_fallback(&candidate);
            // Check untouched candidate trap under ASan and observed ordering
            // at O0 and O3. Observed runs only replace termination and logging.
            for mode in ["asan", "o0", "o3"] {
                let binary = scratch
                    .path()
                    .join(format!("invalid-{destination_len}-{layout:?}-{mode}"));
                match mode {
                    "asan" => tools.build_sanitized(&candidate, &binary),
                    _ => tools.build(&observed, &binary, mode == "o3"),
                }
                .unwrap();
                let output = Command::new(binary)
                    .env("ASAN_OPTIONS", "detect_leaks=0")
                    .output()
                    .unwrap();
                if mode == "asan" {
                    assert_eq!(output.status.signal(), Some(5), "{output:?}");
                    assert!(!String::from_utf8_lossy(&output.stderr).contains("buffer-overflow"));
                } else {
                    assert_eq!(output.status.code(), Some(exit_code), "{output:?}");
                    assert_eq!(output.stdout, [9, 10, 11, 12, 13]);
                }
            }
        }
    }
}

/// Two exact initialized slots and a min-bounded receiving cursor. Source
/// mutation witnesses alter one required induction obligation at a time.
const CURSOR_SLOTS: &str = r#"
struct Slot { wire Bytes }
func make_slot with noblock arg length u64 as readonly copy returns Slot {
    return Slot { wire: core.bytes(length, core.add_wrapping(10, core.index())) }
}
func main returns u64 {
    let size = core.add_wrapping(4, 4)
    let a = core.region(64) else { core.assert(false); return 0 }
    let source = core.batch(a, Slot)
    let source_status = core.append(source, make_slot(size))
    core.assert(source_status == 0)
    let src = core.borrow_readonly(source, 0)
    let b = core.region(64) else { core.assert(false); return 0 }
    let destination = core.batch(b, Slot)
    let destination_status = core.append(destination, make_slot(8))
    core.assert(destination_status == 0)
    let dst = core.borrow_readwrite(destination, 0)
    var position = 0
    var filled = 0
    var required = 2
    for attempt in core.range(4) {
        let room = core.sub_wrapping(required, filled)
        var chunk = 2
        if room < chunk {
            chunk = room
        }
        core.assert(chunk > 0)
        core.assert(chunk <= core.sub_wrapping(size, position))
        for index in core.range(chunk) {
            dst.wire[core.add_wrapping(filled, index)] = src.wire[core.add_wrapping(position, index)]
        }
        position = core.add_wrapping(position, chunk)
        filled = core.add_wrapping(filled, chunk)
        if filled == 2 {
            required = 8
        }
    }
    core.print(position)
    core.print(dst.wire[0])
    core.print(dst.wire[7])
    return 0
}
"#;

/// The exact range proof selects the original copy and preserves its bytes
/// with checked publication and cursor updates through all native modes.
#[test]
fn constructed_slots_and_cursor_induction_preserve_exact_copy() {
    let parsed = parse_experimental(CURSOR_SLOTS).unwrap();
    let checked = parsed.check().unwrap();
    let tools = NativeToolchain::discover().unwrap();
    let candidate =
        llvm::emit_with_layout(&checked, tools.target_triple(), LayoutMode::Derived64).unwrap();
    if tools.target_triple().starts_with("arm64-apple-macosx") {
        assert_eq!(
            candidate
                .matches("Holo constructed slot cursor proof")
                .count(),
            1
        );
    }
    let scratch = TempDir::new().unwrap();
    // Every native build is independently compared against the checked model.
    let expected = model::run_with_layout(parsed.program(), LayoutMode::Derived64).unwrap();
    assert_eq!(expected, [8, 10, 17]);
    for mode in ["o0", "o3", "asan"] {
        let binary = scratch.path().join(mode);
        match mode {
            "asan" => tools.build_sanitized(&candidate, &binary),
            _ => tools.build(&candidate, &binary, mode == "o3"),
        }
        .unwrap();
        let output = Command::new(binary)
            .env("ASAN_OPTIONS", "detect_leaks=0")
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output.stderr);
        assert_eq!(output.stdout, b"8\n10\n17\n");
    }
}

/// Check the private wide leaf against every model byte, including the
/// exact two-byte arm and a 257-byte copy whose end block adds one new byte.
#[test]
fn outlined_cursor_staging_preserves_wide_tail_and_tiny_copy() {
    let source = CURSOR_SLOTS
        .replace("core.add_wrapping(4, 4)", "core.add_wrapping(256, 3)")
        .replace("core.region(64)", "core.region(272)")
        .replace("make_slot(8)", "make_slot(259)")
        .replace(
            "for attempt in core.range(4)",
            "for attempt in core.range(2)",
        )
        .replace("var chunk = 2", "var chunk = 257")
        .replace("required = 8", "required = 259")
        .replace(
            "    core.print(dst.wire[0])\n    core.print(dst.wire[7])",
            "    for byte in core.range(size) {\n        core.print(dst.wire[byte])\n    }",
        );
    let parsed = parse_experimental(&source).unwrap();
    let checked = parsed.check().unwrap();
    let target = "arm64-apple-macosx11.0.0";
    let ordinary = llvm::emit_with_layout(&checked, target, LayoutMode::Derived64).unwrap();
    let outlined =
        llvm::emit_with_outlined_bulk_staging(&checked, target, LayoutMode::Derived64).unwrap();
    assert_eq!(
        outlined
            .matches("Holo constructed slot cursor proof")
            .count(),
        1
    );
    assert_eq!(
        outlined
            .matches("define internal preserve_mostcc void @holo.stage.bulk.transfer")
            .count(),
        1
    );
    assert_eq!(
        outlined
            .matches("call preserve_mostcc void @holo.stage.bulk.transfer")
            .count(),
        1
    );
    assert!(!ordinary.contains("@holo.stage.bulk.transfer"));
    // The bulk intrinsic is the only replaced line inside entry. The fixed
    // two-byte arm, ordinary guards, and producer code retain exact LLVM.
    let (outlined_entry, _) = outlined
        .split_once("\ndefine internal preserve_mostcc void @holo.stage.bulk.transfer")
        .unwrap();
    let call = outlined_entry
        .lines()
        .find(|line| line.contains("call preserve_mostcc void @holo.stage.bulk.transfer"))
        .unwrap();
    let bulk = ordinary
        .lines()
        .find(|line| line.contains("call void @llvm.memcpy.p0.p0.i64"))
        .unwrap();
    assert_eq!(outlined_entry.replacen(call, bulk, 1), ordinary);

    let expected = model::run_with_layout(parsed.program(), LayoutMode::Derived64).unwrap();
    assert_eq!(expected.len(), 260);
    assert_eq!(expected[0], 259);
    assert!(
        expected[1..]
            .iter()
            .copied()
            .eq((10..269).map(|byte| byte % 256))
    );
    let mut expected_output = String::new();
    for byte in expected {
        writeln!(expected_output, "{byte}").unwrap();
    }
    let tools = NativeToolchain::discover().unwrap();
    let native = llvm::emit_with_outlined_bulk_staging(
        &checked,
        tools.target_triple(),
        LayoutMode::Derived64,
    )
    .unwrap();
    let scratch = TempDir::new().unwrap();
    // The host runs the opted-in leaf where supported and the unchanged
    // checked path elsewhere. O0 and ASan exercise the emitted loop directly.
    for mode in ["o0", "o3", "asan"] {
        let binary = scratch.path().join(mode);
        match mode {
            "asan" => tools.build_sanitized(&native, &binary),
            _ => tools.build(&native, &binary, mode == "o3"),
        }
        .unwrap();
        let output = Command::new(binary)
            .env("ASAN_OPTIONS", "detect_leaks=0")
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output.stderr);
        assert_eq!(output.stdout, expected_output.as_bytes());
    }
}

/// The ordinary emitter uses the proved initial request to qualify a wide
/// copy in a larger slot. Adjacent short and larger bounds retain the
/// ordinary transfer. No extra runtime size check is needed.
#[test]
fn ordinary_cursor_staging_outlines_only_the_qualified_maximum() {
    let fixture = CURSOR_SLOTS
        .replace("core.add_wrapping(4, 4)", "core.add_wrapping(256, 256)")
        .replace("core.region(64)", "core.region(528)")
        .replace("make_slot(8)", "make_slot(512)")
        .replace(
            "for attempt in core.range(4)",
            "for attempt in core.range(2)",
        )
        .replace("required = 8", "required = 512");
    let target = "arm64-apple-macosx11.0.0";
    for (maximum, outlined) in [(64, false), (65, true), (256, true), (257, false)] {
        let source = fixture.replace("var chunk = 2", &format!("var chunk = {maximum}"));
        let parsed = parse_experimental(&source).unwrap();
        let checked = parsed.check().unwrap();
        let ordinary = llvm::emit_with_layout(&checked, target, LayoutMode::Derived64).unwrap();
        // Every case retains the cursor proof. Only its wide edge can change.
        assert_eq!(
            ordinary
                .matches("Holo constructed slot cursor proof")
                .count(),
            1
        );
        assert_eq!(
            ordinary.contains("@holo.stage.bulk.transfer"),
            outlined,
            "{maximum}"
        );
        if outlined {
            let experiment =
                llvm::emit_with_outlined_bulk_staging(&checked, target, LayoutMode::Derived64)
                    .unwrap();
            assert_eq!(ordinary, experiment);
        } else {
            assert!(ordinary.contains("call void @llvm.memcpy.p0.p0.i64"));
        }
    }
}

/// The opt-in retains exact ordinary LLVM on layouts and targets outside the
/// cursor proof's qualification, including intrinsic declarations and helpers.
#[test]
fn outlined_cursor_staging_leaves_other_layouts_and_targets_exact() {
    let parsed = parse_experimental(CURSOR_SLOTS).unwrap();
    let checked = parsed.check().unwrap();
    for (target, layout) in [
        ("arm64-apple-macosx11.0.0", LayoutMode::Wide),
        ("arm64-apple-macosx11.0.0", LayoutMode::Compact32),
        ("aarch64-unknown-linux-musl", LayoutMode::Derived64),
        ("x86_64-unknown-linux-musl", LayoutMode::Derived64),
    ] {
        assert_eq!(
            llvm::emit_with_outlined_bulk_staging(&checked, target, layout).unwrap(),
            llvm::emit_with_layout(&checked, target, layout).unwrap(),
        );
    }
}

/// Changed cursor lifetimes, changing bound captures, absent publication
/// checks, and altered updates all retain the original checked copy.
#[test]
fn cursor_induction_requires_stable_lifetimes_and_unmutated_captures() {
    let shrinking_extent = CURSOR_SLOTS
        .replace(
            "    let size = core.add_wrapping(4, 4)",
            "    var position = 0\n    for frame in core.range(2) {\n    let size = core.sub_wrapping(8, core.mul_wrapping(frame, 7))",
        )
        .replace("    var position = 0\n    var filled", "    var filled")
        .replace("    core.print(position)", "    }\n    core.print(position)")
        .replace("    core.print(dst.wire[0])\n    core.print(dst.wire[7])\n", "");
    let independently_reset_required = CURSOR_SLOTS
        .replace(
            "    var required = 2",
            "    for frame in core.range(2) {\n    var required = 2",
        )
        .replace(
            "for attempt in core.range(4)",
            "for attempt in core.range(3)",
        )
        .replace(
            "    core.print(position)",
            "    }\n    core.print(position)",
        );
    let changed_capture = CURSOR_SLOTS.replace(
        "        core.assert(chunk <= core.sub_wrapping(size, position))",
        "        var checked_count = chunk\n        if position == 0 { checked_count = 0 }\n        core.assert(checked_count <= core.sub_wrapping(size, position))",
    );
    let cases = [
        shrinking_extent,
        independently_reset_required,
        changed_capture,
        CURSOR_SLOTS.replace("    core.assert(source_status == 0)\n", ""),
        CURSOR_SLOTS.replace(
            "position = core.add_wrapping(position, chunk)",
            "position = core.add_wrapping(position, 1)",
        ),
        CURSOR_SLOTS.replace("required = 8", "required = 9"),
    ];
    // A fixed target makes the rejection checks independent of host support.
    for source in cases {
        let parsed = parse_experimental(&source).unwrap();
        let checked = parsed.check().unwrap();
        let ordinary =
            llvm::emit_with_layout(&checked, "arm64-apple-macosx11.0.0", LayoutMode::Derived64)
                .unwrap();
        assert!(
            !ordinary.contains("Holo constructed slot cursor proof"),
            "{source}"
        );
        assert_eq!(
            llvm::emit_with_outlined_bulk_staging(
                &checked,
                "arm64-apple-macosx11.0.0",
                LayoutMode::Derived64,
            )
            .unwrap(),
            ordinary,
            "{source}"
        );
    }
}
