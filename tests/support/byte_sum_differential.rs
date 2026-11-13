//! Differential correctness for ordinary and outlined byte-sum lowering.
//!
//! One checked Holo program constructs packed records with varied byte spans.
//! A host oracle sums each record independently in u128. Both emitted variants
//! use the same native target, driver and layouts. Compiler and test allocations
//! are outside the generated program, and the comparisons have no timing claim.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::parse_experimental;
use holo::layout::LayoutMode;

use super::SUM_CALL;

/// Reproducible SplitMix64 starting state for the complete generated corpus.
const RANDOM_SEED: u64 = 0x5e65_5ec0_4633_80cb;
/// The current positive selector target, also used for static checks elsewhere.
const SIMD_TARGET: &str = "arm64-apple-macosx11.0.0";
/// Same-architecture selector control before restoring the real module header.
const ORDINARY_TARGET: &str = "aarch64-unknown-linux-gnu";
/// Optional fresh destination for retaining exact source, IR and process output.
const ARTIFACTS_ENV: &str = "HOLO_BYTE_SUM_DIFFERENTIAL_ARTIFACTS";

/// Packed source fixture. Each producer initializes scalar values and two inline
/// byte fields. Poisoning through a writable alias happens before either scan.
const SOURCE: &str = r#"
struct Payload { bytes Bytes }
struct Slot {
    prefix u64
    seed u64
    leading Bytes
    payload Payload
}
func fill_payload arg length u64 as readonly copy arg salt u64 as readonly copy returns Payload {
    return Payload {
        bytes: core.bytes(length, core.xor(
            core.add_wrapping(salt, core.mul_wrapping(149, core.index())),
            core.mul_wrapping(core.index(), core.index())
        ))
    }
}
func fill
    arg length u64 as readonly copy
    arg prefix u64 as readonly copy
    arg seed u64 as readonly copy
    arg pad u64 as readonly copy
    arg salt u64 as readonly copy
    returns Slot
{
    return Slot { prefix: prefix, seed: seed, leading: core.bytes(pad, 53), payload: fill_payload(length, salt) }
}
func borrowed arg input Bytes as readonly reference arg seed u64 as readonly copy returns u64 {
    var sum = seed
    // The same checked logical extent supplies the trip count and indexed bytes.
    for j in core.range(core.len(input)) {
        sum = core.add_wrapping(input[j], sum)
    }
    return sum
}
func main returns u64 {
    let owner = core.region(CORPUS_CAPACITY) else { core.assert(false); return 0 }
    let items = core.batch(owner, Slot)
    INSERT_RECORDS
    let first = core.borrow_readonly(items, 0)
    let origin = core.address(first)
    // The selected packed layout controls the real byte stride between records.
    for readwrite writer in items {
        let row = core.alias_readonly(writer)
        let length = core.len(row.payload.bytes)
        let prefix = row.prefix
        core.assert(prefix <= length)
        // Any borrow wider than requested would include nonzero initialized bytes.
        for j in core.range(core.sub_wrapping(length, prefix)) {
            writer.payload.bytes[core.add_wrapping(prefix, j)] = 255
        }
        let record_offset = core.sub_wrapping(core.address(row), origin)
        // The payload's position differs by a schema-constant offset within each layout.
        core.print(core.and(core.add_wrapping(record_offset, core.len(row.leading)), 15))
        var sum = row.seed
        for i in core.range(core.len(row.payload.bytes)) {
            sum = core.add_wrapping(sum, row.payload.bytes[i])
        }
        core.print(sum)
        core.print(borrowed(core.prefix(row.payload.bytes, prefix), row.seed))
    }
    return 0
}
"#;

/// One generated record and its checked borrowed view.
struct Case {
    /// Complete initialized payload width, including poison.
    length: u64,
    /// Prefix width retained in the borrowed descriptor.
    prefix: u64,
    /// Initial wrapping accumulator for both observations.
    seed: u64,
    /// Length of the preceding packed byte field, which shifts the payload.
    pad: u64,
    /// Per-record byte pattern input, separate from the accumulator seed.
    salt: u64,
}

/// Advance the host-only SplitMix64 state and return its next reproducible value.
/// Unsigned wraparound is intentional. This generator does no I/O or allocation.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Generate one record with a valid requested prefix and varied contents, offset
/// and seed. A near-maximum seed makes large nonempty sums cross u64::MAX.
fn case(random: &mut u64, length: u64, prefix: u64) -> Case {
    let value = next(random);
    let seed = if next(random) & 1 == 0 {
        value
    } else {
        u64::MAX - (value & 1023)
    };
    Case {
        length,
        prefix,
        seed,
        pad: next(random) % 33,
        salt: next(random),
    }
}

/// Generate complete/borrowed boundary cases and bounded randomized records.
/// Short and page-scale cases share one source and one native build per layout.
fn cases() -> Vec<Case> {
    let mut random = RANDOM_SEED;
    let mut result = Vec::new();
    // Every short full length has an explicit witness, as do both sides of the
    // larger boundaries where block/tail composition changes.
    for length in (0_u64..=32).chain([
        63, 64, 65, 127, 128, 129, 255, 256, 257, 1023, 1024, 1025, 4095, 4096, 4097, 16383, 16384,
        16385,
    ]) {
        result.push(case(&mut random, length, length.saturating_sub(1)));
    }
    // Give each short borrowed length its own nonempty poisoned suffix.
    for prefix in 0..=32 {
        let length = prefix + 1 + next(&mut random) % 19;
        result.push(case(&mut random, length, prefix));
    }
    // Mix zero, whole, interior and last-byte-excluded prefixes independently
    // of the sampled full width. The cap keeps compile and ASan costs bounded.
    for index in 0..128 {
        let length = next(&mut random) % [33, 258, 4098, 16386][index % 4];
        let prefix = match next(&mut random) % 4 {
            0 => 0,
            1 => length,
            2 => length.saturating_sub(1),
            _ => next(&mut random) % (length + 1),
        };
        result.push(case(&mut random, length, prefix));
    }
    result
}

/// Instantiate source with complete inputs and a finite owner capacity.
/// The generous per-record overhead budget covers the two record headers, two
/// length fields and two u64 fields in the widest layout without using its planner.
fn source(cases: &[Case]) -> String {
    let mut appends = String::new();
    let mut capacity = 0;
    // Each complete record has one admission check before its later observation.
    for (index, case) in cases.iter().enumerate() {
        capacity += 64 + case.length + case.pad;
        writeln!(
            appends,
            "let status{index} = core.append(items, fill({}, {}, {}, {}, {}))\ncore.assert(status{index} == 0)",
            case.length, case.prefix, case.seed, case.pad, case.salt
        )
        .unwrap();
    }
    SOURCE
        .replace("CORPUS_CAPACITY", &capacity.to_string())
        .replace("INSERT_RECORDS", &appends)
}

/// Compute the ordered full and prefix witnesses independently of LLVM and layout.
/// Evaluating the byte pattern in u128 yields the same low eight bits after source
/// arithmetic wraps. The host reduces to u64 once, after summing in u128.
fn expected(case: &Case) -> (u64, u64) {
    let mut prefix = u128::from(case.seed);
    // Only the unpoisoned prefix contributes actual pattern bytes to this sum.
    for i in 0..u128::from(case.prefix) {
        prefix += ((u128::from(case.salt) + 149 * i) ^ (i * i)) & 255;
    }
    let full = prefix + 255 * u128::from(case.length - case.prefix);
    (full as u64, prefix as u64)
}

/// Require both checked reduction call sites and the out-of-line definition.
/// An emitted kernel with no call sites would give a false coverage claim.
fn assert_selected(llvm: &str) {
    assert_eq!(llvm.matches(SUM_CALL).count(), 2);
    assert!(
        llvm.contains(
            "define internal i64 @holo.byte.sum(ptr %data, i64 %length, i64 %seed) noinline {"
        ),
        "expected the production outlined helper"
    );
    assert!(!llvm.contains("@holo.copy.checked("));
}

/// Compile one profile via the normal driver and return successful native stdout.
/// Writes exact IR, commands and output in the caller-owned artifact directory.
fn run(
    tools: &NativeToolchain,
    directory: &Path,
    label: &str,
    llvm: &str,
    profile: &str,
) -> Vec<u8> {
    let binary = directory.join(format!("{label}-{profile}"));
    fs::write(binary.with_extension("ll"), llvm).unwrap();
    // Sanitizer instruments the real generated function bodies.
    let build = if profile == "ASan" {
        tools.build_sanitized(llvm, &binary)
    } else {
        tools.build(llvm, &binary, profile == "O3")
    }
    .unwrap_or_else(|error| panic!("{label}/{profile}: {error}"));
    fs::write(
        binary.with_extension("commands.txt"),
        build.commands.join("\n"),
    )
    .unwrap();
    let output = Command::new(&binary).output().unwrap();
    fs::write(binary.with_extension("stdout"), &output.stdout).unwrap();
    fs::write(binary.with_extension("stderr"), &output.stderr).unwrap();
    fs::write(binary.with_extension("status"), output.status.to_string()).unwrap();
    assert!(output.status.success(), "{label}/{profile}: {output:?}");
    assert!(output.stderr.is_empty(), "{label}/{profile}: {output:?}");
    output.stdout
}

/// Check all outputs and cover every alignment class among spans of at least
/// 16 bytes. The schema offset and start address are constants per execution,
/// so removing them only rotates the 16 observed residue classes.
fn validate(cases: &[Case], stdout: &[u8], label: &str) {
    let values: Vec<u64> = std::str::from_utf8(stdout)
        .unwrap()
        .lines()
        .map(|line| line.parse().unwrap())
        .collect();
    assert_eq!(values.len(), 3 * cases.len(), "{label}");
    let mut full_alignment = 0_u16;
    let mut prefix_alignment = 0_u16;
    // Report an individual case failure before a missing alignment class.
    for (index, (case, witness)) in cases.iter().zip(values.chunks_exact(3)).enumerate() {
        let (full, prefix) = expected(case);
        assert_eq!(&witness[1..], &[full, prefix], "{label}, case {index}");
        assert!(witness[0] < 16, "{label}, case {index}");
        let residue = 1 << witness[0];
        // Count only spans that can actually enter the 16-byte kernel.
        if case.length >= 16 {
            full_alignment |= residue;
        }
        if case.prefix >= 16 {
            prefix_alignment |= residue;
        }
    }
    assert_eq!(full_alignment, u16::MAX, "{label}, full alignment");
    assert_eq!(prefix_alignment, u16::MAX, "{label}, prefix alignment");
}

/// Execute the same checked source using ordinary and selected emission.
/// Other targets check selected LLVM structurally and execute their ordinary
/// native path. Artifact paths are fresh and opt-in for replay.
#[test]
fn randomized_owned_and_borrowed_sums_match_both_lowerings() {
    let cases = cases();
    let source = source(&cases);
    let parsed = parse_experimental(&source).unwrap_or_else(|error| panic!("{error:?}"));
    let checked = parsed.check().unwrap();
    let tools = NativeToolchain::discover().expect("byte-sum differential requires matched LLVM");
    let scratch = TempDir::new().unwrap();
    let preserve = env::var_os(ARTIFACTS_ENV).map(PathBuf::from);
    let directory = preserve.as_deref().unwrap_or_else(|| scratch.path());
    // A persistent capture owns a fresh directory. Routine checks use scratch storage.
    if preserve.is_some() {
        fs::create_dir(directory).expect("byte-sum differential capture directory must be new");
    }
    fs::write(directory.join("program.holo"), &source).unwrap();
    fs::write(directory.join("toolchain.txt"), &tools.identity).unwrap();
    let oracle = cases.iter().fold(String::new(), |mut rows, case| {
        let (full, prefix) = expected(case);
        writeln!(rows, "{full}\t{prefix}").unwrap();
        rows
    });
    fs::write(directory.join("expected.tsv"), oracle).unwrap();
    let native_selection = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    // Native builds use the actual driver target. Other hosts exercise the same
    // selector with a known admitted triple while keeping execution native.
    let selected_target = if native_selection {
        tools.target_triple()
    } else {
        SIMD_TARGET
    };
    // Every layout keeps its own packed size and byte-header rules.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let selected = holo::llvm::emit_with_layout(&checked, selected_target, layout).unwrap();
        assert_selected(&selected);
        let selector = if native_selection {
            ORDINARY_TARGET
        } else {
            tools.target_triple()
        };
        let ordinary = holo::llvm::emit_with_layout(&checked, selector, layout).unwrap();
        // No source-byte copy occurs in this fixture. The target header is the
        // only edited IR line, so the verified same-target run isolates lowering.
        assert!(!ordinary.contains("@holo.byte.sum("));
        assert!(!ordinary.contains("@holo.copy.checked("));
        let from = format!("target triple = \"{selector}\"");
        let to = format!("target triple = \"{}\"", tools.target_triple());
        assert_eq!(ordinary.matches(&from).count(), 1);
        let ordinary = ordinary.replacen(&from, &to, 1);
        // O0 covers native ordinary iteration, O3 covers optimized output, and
        // ASan catches out-of-backing accesses in instrumented generated code.
        for profile in ["O0", "O3", "ASan"] {
            let label = format!("{}-ordinary", layout.name());
            let baseline = run(&tools, directory, &label, &ordinary, profile);
            validate(&cases, &baseline, &format!("{label}/{profile}"));
            // Only execute the selected variant on its admitted native target.
            if native_selection {
                let label = format!("{}-outlined", layout.name());
                let optimized = run(&tools, directory, &label, &selected, profile);
                validate(&cases, &optimized, &format!("{label}/{profile}"));
                assert_eq!(optimized, baseline, "{label}/{profile}");
            }
        }
    }
    println!(
        "byte-sum differential seed {RANDOM_SEED:#018x}, {} records, {} native builds, all 16 residues for owned and borrowed vector-sized spans per build",
        cases.len(),
        if native_selection { 18 } else { 9 }
    );
}
