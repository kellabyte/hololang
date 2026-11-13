//! Source and native witnesses for compile-time tiny wrapping reductions.
//!
//! Literal prefix calls share a callee with the unspecialized dynamic call.
//! Backing is reused through a writable alias and an independent host oracle
//! expects only the selected prefix. No foreign payload routine is involved.

use std::process::Command;

use holo::{
    driver::{NativeToolchain, TempDir},
    frontend::parse_experimental,
    layout::LayoutMode,
};

/// Archived checked fixture with both statically selected and dynamic calls.
const SOURCE: &str = r#"
struct Item {
    pad Bytes
    data Bytes
}
func make returns Item {
    return Item { pad: core.bytes(1, 1), data: core.bytes(4, 213) }
}
func sum arg input Bytes as readonly reference arg seed u64 as readonly copy returns u64 {
    var total = seed
    // Wrap once per initialized byte in the exact logical prefix.
    for i in core.range(core.len(input)) {
        total = core.add_wrapping(total, input[i])
    }
    return total
}
func literal_sum arg input Bytes as readonly reference returns u64 {
    var total = 18446744073709551598
    // Reversed operands have the same wrapping result.
    for i in core.range(core.len(input)) {
        total = core.add_wrapping(input[i], total)
    }
    return total
}
func main returns u64 {
    let region = core.region(128) else { return 0 }
    let batch = core.batch(region, Item)
    let status = core.append(batch, make())
    core.assert(status == 0)
    for readwrite writer in batch {
        let row = core.alias_readonly(writer)
        writer.data[0] = 227
        writer.data[1] = 193
        core.opaque(region)
        let seed = 18446744073709551598
        core.print(sum(core.prefix(row.data, 0), seed))
        core.print(sum(core.prefix(row.data, 1), seed))
        core.print(sum(core.prefix(row.data, 2), seed))
        core.print(sum(core.prefix(row.data, 3), seed))
        // Metadata stays a dynamic extent at this call site.
        let dynamic = core.len(row.data)
        core.print(sum(core.prefix(row.data, dynamic), seed))
        core.print(literal_sum(core.prefix(row.data, 2)))
    }
    return 0
}
"#;

/// Parse/check the source and return its emitted layout-specific LLVM.
fn emit(source: &str, target: &str, layout: LayoutMode) -> String {
    let parsed = parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("static-sum.holo", source)));
    holo::llvm::emit_with_layout(&parsed.check().unwrap(), target, layout).unwrap()
}

/// Confirm width and purity constrain selection, independent of native LLVM passes.
#[test]
fn literal_prefix_only_replaces_calls_to_a_pure_matching_callee() {
    let target = "arm64-apple-macosx11.0.0";
    let count = |llvm: &str| llvm.matches("call i64 @holo.function.").count();
    assert_eq!(count(&emit(SOURCE, target, LayoutMode::Compact32)), 2);
    assert_eq!(
        count(&emit(
            SOURCE,
            "aarch64-unknown-linux-gnu",
            LayoutMode::Compact32
        )),
        6
    );
    // Computed seed initialization or a changed result leaves all calls to
    // that helper intact. The separate literal helper stays eligible.
    for (old, replacement) in [
        ("var total = seed", "var total = core.add_wrapping(seed, 1)"),
        (
            "return total\n}\nfunc literal_sum",
            "return core.add_wrapping(total, 1)\n}\nfunc literal_sum",
        ),
    ] {
        let altered = SOURCE.replace(old, replacement);
        assert_ne!(SOURCE, altered);
        assert_eq!(count(&emit(&altered, target, LayoutMode::Compact32)), 5);
    }
}

/// Execute the static and dynamic paths under each physical layout and compiler mode.
#[test]
fn literal_prefix_outputs_match_wrapping_oracle_and_ordinary_lowering() {
    let tool = NativeToolchain::discover().expect("static byte-sum witness requires LLVM");
    let temp = TempDir::new().unwrap();
    let expected = [0, 227, 420, 633, 846, 420]
        .map(|sum| (u64::MAX - 17).wrapping_add(sum).to_string())
        .join("\n")
        + "\n";
    // Re-target the ordinary emitted IR so both paths use identical native code
    // generation settings. The fixture has no copy-selection opportunities.
    let ordinary_target = "aarch64-unknown-linux-gnu";
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        for (variant, llvm) in [
            (
                "ordinary",
                emit(SOURCE, ordinary_target, layout).replacen(
                    ordinary_target,
                    tool.target_triple(),
                    1,
                ),
            ),
            ("candidate", emit(SOURCE, tool.target_triple(), layout)),
        ] {
            // Debug, optimized, and sanitized executions must all read exact widths.
            for profile in ["O0", "O3", "ASan"] {
                let output = temp.path().join(format!("{layout:?}-{variant}-{profile}"));
                if profile == "ASan" {
                    tool.build_sanitized(&llvm, &output).unwrap();
                } else {
                    tool.build(&llvm, &output, profile == "O3").unwrap();
                }
                let run = Command::new(output).output().unwrap();
                assert!(
                    run.status.success(),
                    "{layout:?}/{variant}/{profile}: {run:?}"
                );
                assert_eq!(
                    run.stdout,
                    expected.as_bytes(),
                    "{layout:?}/{variant}/{profile}"
                );
                assert!(run.stderr.is_empty(), "{layout:?}/{variant}/{profile}");
            }
        }
    }
}
