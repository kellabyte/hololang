//! Regression checks for matching and executing checked contiguous byte sums.
//!
//! Actual source exercises owned and borrowed spans, unaligned payloads, poison
//! after a prefix, and wrapping seeds. Host arithmetic supplies independent
//! expected values. Native runs validate correctness without claiming speed.

#[path = "../../support/byte_sum_differential.rs"]
mod differential;
#[path = "../../support/large_byte_sum.rs"]
mod large_byte_sum;
#[path = "../../support/static_byte_sum.rs"]
mod static_byte_sum;

use std::process::Command;

use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::parse_experimental;
use holo::layout::LayoutMode;

/// Include every 16-byte remainder, multiple blocks, and a page-scale extent.
const LENGTHS: [u64; 35] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32, 65, 4095,
];
/// A seed that overflows on the first byte in every nonempty witness.
const SEED: u64 = u64::MAX - 17;
/// Production LLVM symbol used only for this matched form.
const SUM_CALL: &str = "call i64 @holo.byte.sum(";

/// Actual parsed source for nested field reads and prefix-borrowed helper reads.
/// The leading one-byte field deliberately misaligns the later payload. The
/// suffix after each requested prefix is initialized but must not be included.
/// A readonly alias observes a mutation made through the original reference.
const SOURCE: &str = r#"
struct Payload { bytes Bytes }
struct Slot {
    leading Bytes
    payload Payload
}
func fill
    arg length u64 as readonly copy
    returns Slot
{
    return Slot {
        leading: core.bytes(1, 203),
        payload: Payload { bytes: core.bytes(length, core.add_wrapping(211, core.mul_wrapping(13, core.index()))) }
    }
}
func borrowed
    arg input Bytes as readonly reference
    arg seed u64 as readonly copy
    returns u64
{
    var sum = seed
    // The reversed add operands still sum exactly one indexed byte per iteration.
    for j in core.range(core.len(input)) {
        sum = core.add_wrapping(input[j], sum)
    }
    return sum
}
func main returns u64 {
    let owner = core.region(65536) else { return 0 }
    let items = core.batch(owner, Slot)
    // The fixture inserts complete records before the single published traversal.
    INSERT_RECORDS
    for readwrite writer in items {
        let row = core.alias_readonly(writer)
        if core.len(row.payload.bytes) > 0 {
            writer.payload.bytes[0] = 227
        }
        var sum = INITIAL_SEED
        for i in core.range(core.len(row.payload.bytes)) {
            sum = core.add_wrapping(sum, row.payload.bytes[i])
        }
        core.print(sum)
        let len = core.len(row.payload.bytes)
        var prefix = 0
        if len > 0 {
            prefix = core.sub_wrapping(len, 1)
        }
        core.print(borrowed(core.prefix(row.payload.bytes, prefix), INITIAL_SEED))
    }
    return 0
}
"#;

/// Instantiate a checked source program with all payload/tail lengths.
/// The returned text owns test data only and never influences native dispatch.
fn source() -> String {
    let mut records = String::new();
    // Each append has its own lexical status; successful construction is asserted.
    for (position, length) in LENGTHS.into_iter().enumerate() {
        records.push_str(&format!(
            "let status{position} = core.append(items, fill({length}))\ncore.assert(status{position} == 0)\n"
        ));
    }
    SOURCE
        .replace("INSERT_RECORDS", &records)
        .replace("INITIAL_SEED", &SEED.to_string())
}

/// Compute the exact decimal output for the full field and borrowed prefix.
/// The host loop uses each byte's independently computed modulo-256 value and
/// carries a full u64 wrapping total, with no access to compiler or layout facts.
fn oracle() -> String {
    let mut output = String::new();
    // Per-record outputs appear as a full length followed by a narrowed prefix.
    for len in LENGTHS {
        for width in [len, len.saturating_sub(1)] {
            let value = (0..width).fold(SEED, |sum, i| {
                // The first initialized byte was written through the other alias.
                let byte = if i == 0 { 227 } else { (211 + 13 * i) % 256 };
                sum.wrapping_add(byte)
            });
            output.push_str(&format!("{value}\n"));
        }
    }
    output
}

/// Verify two positive shapes and intentionally close legal nonmatches.
/// This uses explicit target triples without executing or measuring their code.
#[test]
fn selects_only_private_wrapping_sums_over_the_same_checked_extent() {
    let source = source();
    let target = "arm64-apple-macosx11.0.0";
    let emit = |source: &str, target| {
        let parsed = parse_experimental(source).unwrap_or_else(|e| panic!("{e:?}"));
        holo::llvm::emit(&parsed.check().unwrap(), target)
    };
    let native = emit(&source, target);
    assert_eq!(native.matches(SUM_CALL).count(), 2);
    assert_eq!(
        native
            .matches("define internal i64 @holo.byte.sum(")
            .count(),
        1
    );
    assert!(!emit(&source, "aarch64-unknown-linux-gnu").contains("@holo.byte.sum("));

    // Changing extent, index, arithmetic, or effects must leave the field loop
    // scalar. The borrowed helper remains a positive witness in all variations.
    for (before, after) in [
        (
            "core.range(core.len(row.payload.bytes))",
            "core.range(core.len(row.leading))",
        ),
        (
            "row.payload.bytes[i]",
            "row.payload.bytes[core.add_wrapping(i, 1)]",
        ),
        (
            "core.add_wrapping(sum, row.payload.bytes[i])",
            "core.xor(sum, row.payload.bytes[i])",
        ),
        (
            "sum = core.add_wrapping(sum, row.payload.bytes[i])",
            "sum = core.add_wrapping(sum, row.payload.bytes[i])\ncore.print(sum)",
        ),
    ] {
        let altered = source.replace(before, after);
        assert_ne!(altered, source);
        assert_eq!(
            emit(&altered, target).matches(SUM_CALL).count(),
            1,
            "{after}"
        );
    }
}

/// Execute real emitted code at O0/O3 and with AddressSanitizer in all layouts.
/// Compilation and all native output stay in temporary test storage.
#[test]
fn matches_wrapping_oracle_for_owned_and_borrowed_byte_ranges() {
    let source = source();
    let expected = oracle();
    let parsed = parse_experimental(&source).unwrap_or_else(|e| panic!("{e:?}"));
    let checked = parsed.check().unwrap();
    let toolchain =
        NativeToolchain::discover().expect("byte-sum native tests require matched LLVM");
    let scratch = TempDir::new().unwrap();
    // Frozen offsets and byte lengths differ across the physical layouts.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let llvm =
            holo::llvm::emit_with_layout(&checked, toolchain.target_triple(), layout).unwrap();
        // Keep the unoptimized and instrumented loads observable independently.
        for profile in ["O0", "O3", "ASan"] {
            let binary = scratch.path().join(format!("{}-{profile}", layout.name()));
            // Sanitizer requires its own instrumented compilation path.
            if profile == "ASan" {
                toolchain.build_sanitized(&llvm, &binary).unwrap();
            } else {
                toolchain.build(&llvm, &binary, profile == "O3").unwrap();
            }
            let run = Command::new(&binary).output().unwrap();
            assert!(run.status.success(), "{layout:?}/{profile}: {run:?}");
            assert!(run.stderr.is_empty(), "{layout:?}/{profile}: {run:?}");
            assert_eq!(run.stdout, expected.as_bytes(), "{layout:?}/{profile}");
        }
    }
}
