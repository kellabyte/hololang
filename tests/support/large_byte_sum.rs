//! Checked lower-bound witnesses for the outlined 64-byte reduction.
//!
//! The two helpers share one unaligned, initialized backing with a poison
//! suffix. Native runs check exact tails and modulo arithmetic. LLVM checks
//! pin literal selection and preserve fallback for dynamic extents.

use std::process::Command;

use holo::{
    driver::{NativeToolchain, TempDir},
    frontend::parse_experimental,
    layout::LayoutMode,
};

/// Call symbol unique to the outlined 64-byte routine.
const WIDE_CALL: &str = "call i64 @holo.byte.sum.large(";
/// Seed that wraps during the first nonempty reduction.
const SEED: u64 = u64::MAX - 9;

/// Literal prefixes around the cutoff, unaligned large tails and dynamic
/// prefixes. The latter preserve the general narrow reduction.
const SOURCE: &str = r#"
struct Item {
    pad Bytes
    data Bytes
}
func make returns Item {
    return Item {
        pad: core.bytes(1, 101),
        data: core.bytes(329, core.add_wrapping(core.mul_wrapping(core.index(), 7), 219))
    }
}
func sum arg input Bytes as readonly reference arg seed u64 as readonly copy returns u64 {
    var total = seed
    for index in core.range(core.len(input)) {
        total = core.add_wrapping(total, input[index])
    }
    return total
}
func main returns u64 {
    let region = core.region(1024) else { return 0 }
    let batch = core.batch(region, Item)
    let status = core.append(batch, make())
    core.assert(status == 0)
    let item = core.borrow_readonly(batch, 0)
    core.opaque(region)
    let seed = 18446744073709551606
    core.print(sum(core.prefix(item.data, 129), seed))
    core.print(sum(core.prefix(item.data, 192), seed))
    core.print(sum(core.prefix(item.data, 255), seed))
    core.print(sum(core.prefix(item.data, 256), seed))
    core.print(sum(core.prefix(item.data, 257), seed))
    core.print(sum(core.prefix(item.data, 319), seed))
    core.print(sum(core.prefix(item.data, 320), seed))
    core.print(sum(core.prefix(item.data, 321), seed))
    let dynamic = core.len(item.pad)
    core.print(sum(core.prefix(item.data, dynamic), seed))
    return 0
}
"#;

/// Check the real source and emit IR for the requested layout and target.
/// Compiler-owned text is the only output and the checked input remains unchanged.
fn emit(source: &str, target: &str, layout: LayoutMode) -> String {
    let parsed = parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("large-byte-sum.holo", source)));
    holo::llvm::emit_with_layout(&parsed.check().unwrap(), target, layout).unwrap()
}

/// Emit the same checked fixture with the three-block compiler policy enabled.
fn emit_exact(source: &str, target: &str, layout: LayoutMode) -> String {
    let parsed = parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("exact-byte-sum.holo", source)));
    holo::llvm::emit_with_exact_three_block_sums(&parsed.check().unwrap(), target, layout).unwrap()
}

/// Count wide calls to catch width mistakes or accidental selection for
/// dynamic extents and callees with additional scalar work.
#[test]
fn large_sums_require_a_literal_extent_and_a_pure_callee() {
    let target = "arm64-apple-macosx11.0.0";
    let layout = LayoutMode::Compact32;
    let llvm = emit(SOURCE, target, layout);
    assert_eq!(llvm.matches(WIDE_CALL).count(), 5);
    assert_eq!(llvm.matches("load <64 x i8>").count(), 1);
    assert_eq!(llvm.matches("call i64 @holo.function.").count(), 4);
    assert_eq!(llvm.matches("call i64 @holo.byte.sum(").count(), 2);
    let computed_seed =
        SOURCE.replace("var total = seed", "var total = core.add_wrapping(seed, 1)");
    assert!(!emit(&computed_seed, target, layout).contains(WIDE_CALL));
    assert!(!emit(SOURCE, "aarch64-unknown-linux-gnu", layout).contains(WIDE_CALL));
}

/// The explicit policy changes one literal 192-byte call in Wide. The
/// Derived64 default uses that same selection while preserving other call edges.
#[test]
fn exact_three_block_selection_reuses_the_checked_call_site() {
    let target = "arm64-apple-macosx11.0.0";
    let layout = LayoutMode::Wide;
    let source = SOURCE
        .replace(
            "core.print(sum(core.prefix(item.data, 192), seed))",
            "core.print(sum(core.prefix(item.data, 191), seed))\n    core.print(sum(core.prefix(item.data, 192), seed))\n    core.print(sum(core.prefix(item.data, 193), seed))",
        )
        .replace("let dynamic = core.len(item.pad)", "let dynamic = core.mul_wrapping(core.len(item.pad), 192)");
    let ordinary = emit(&source, target, layout);
    let candidate = emit_exact(&source, target, layout);
    assert_eq!(ordinary.lines().count(), candidate.lines().count());
    let changes: Vec<_> = ordinary
        .lines()
        .zip(candidate.lines())
        .filter(|(before, after)| before != after)
        .collect();
    // The other literal calls already supply the shared helper definition.
    let [(before, after)] = changes.as_slice() else {
        panic!("expected one exact call-edge change: {changes:?}");
    };
    assert!(before.contains("call i64 @holo.function."), "{before}");
    assert!(
        after.contains(WIDE_CALL) && after.contains(", i64 192,"),
        "{after}"
    );
    assert_eq!(
        before.split_once('(').unwrap().1,
        after.split_once('(').unwrap().1
    );

    // A module with no admitted literal keeps the complete ordinary LLVM.
    let without_literal = source.replace("prefix(item.data, 192)", "prefix(item.data, 190)");
    assert_eq!(
        emit(&without_literal, target, layout),
        emit_exact(&without_literal, target, layout)
    );

    // Only the qualified default layout adds the 192-byte call to the five
    // existing large literals. The explicit emitter can still exercise it elsewhere.
    let derived = LayoutMode::Derived64;
    let selected = emit(&source, target, derived);
    assert_eq!(selected, emit_exact(&source, target, derived));
    assert_eq!(selected.matches(WIDE_CALL).count(), 6);
    assert!(
        selected
            .lines()
            .any(|line| line.contains(WIDE_CALL) && line.contains(", i64 192,"))
    );
    assert_eq!(
        emit(&source, target, LayoutMode::Compact32)
            .matches(WIDE_CALL)
            .count(),
        5
    );
    // An unrecognized callee cannot reach the helper on the default layout.
    let computed_seed =
        source.replace("var total = seed", "var total = core.add_wrapping(seed, 1)");
    assert!(!emit(&computed_seed, target, derived).contains(WIDE_CALL));

    // The current policy is specific to the supported macOS arm64 target.
    for other in ["aarch64-unknown-linux-musl", "x86_64-apple-macosx11.0.0"] {
        let fallback = emit(&source, other, derived);
        assert!(!fallback.contains(WIDE_CALL));
        assert_eq!(fallback, emit_exact(&source, other, derived));
    }
}

/// Validate exact unaligned loads, nonempty remainder, and wrap at u64.
/// The independent oracle counts each original byte once per source reduction.
#[test]
fn proved_large_sums_match_native_oracle_and_keep_short_prefix_failure() {
    let tool = NativeToolchain::discover().expect("large byte-sum witness requires LLVM");
    let temp = TempDir::new().unwrap();
    let expected = [129, 192, 255, 256, 257, 319, 320, 321, 1]
        .map(|len| {
            let sum: u64 = (0..len).map(|index| (219 + index * 7) % 256).sum();
            SEED.wrapping_add(sum).to_string()
        })
        .join("\n")
        + "\n";
    // Each layout and build profile exercises both policies and the poison suffix.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        for (policy, llvm) in [
            ("default", emit(SOURCE, tool.target_triple(), layout)),
            (
                "three-block",
                emit_exact(SOURCE, tool.target_triple(), layout),
            ),
        ] {
            for profile in ["O0", "O3", "ASan"] {
                let binary = temp.path().join(format!("{layout:?}-{policy}-{profile}"));
                if profile == "ASan" {
                    tool.build_sanitized(&llvm, &binary).unwrap();
                } else {
                    tool.build(&llvm, &binary, profile == "O3").unwrap();
                }
                let run = Command::new(binary).output().unwrap();
                assert!(
                    run.status.success(),
                    "{layout:?}/{policy}/{profile}: {run:?}"
                );
                assert!(
                    run.stderr.is_empty(),
                    "{layout:?}/{policy}/{profile}: {run:?}"
                );
                assert_eq!(
                    run.stdout,
                    expected.as_bytes(),
                    "{layout:?}/{policy}/{profile}"
                );
            }
        }
    }
    // A literal does not bypass the eager prefix bounds check.
    let source = SOURCE.replace("prefix(item.data, 319)", "prefix(item.data, 330)");
    let llvm = emit(&source, tool.target_triple(), LayoutMode::Derived64);
    let binary = temp.path().join("out-of-range");
    tool.build(&llvm, &binary, true).unwrap();
    assert!(!Command::new(binary).output().unwrap().status.success());

    // A selected 192-byte literal must still trap on a 191-byte backing.
    let source = SOURCE.replace("core.bytes(329,", "core.bytes(191,");
    let llvm = emit(&source, tool.target_triple(), LayoutMode::Derived64);
    let binary = temp.path().join("short-three-block");
    tool.build(&llvm, &binary, true).unwrap();
    assert!(!Command::new(binary).output().unwrap().status.success());
}
