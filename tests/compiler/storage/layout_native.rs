//! Source/native conformance for explicit schema-aware compact layout experiments.
//!
//! Hand-derived expected values are checked against an independent owned-value
//! model before actual O0/O3/ASan binaries run. No compiler layout size or offset
//! helper supplies expected results. Fixed/variable nested projections, unaligned
//! bytes, retained aliases, private rollback, and cleanup remain ordinary source.

#[path = "../../support/model/mod.rs"]
mod model;

use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::parse_experimental;
use holo::layout::LayoutMode;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The benchmark-shaped unary variable tail with growth, mutation, failure and reuse.
/// Physical addresses are compared within one execution, never printed as oracle integers.
const TAIL: &str = r#"
// Inline frozen bytes and a copied application identity.
struct Payload { data Bytes }
struct Entry { id u64, payload Payload }
// Nested field construction keeps its final destination through ordinary calls.
func payload arg length u64 as readonly copy arg seed u64 as readonly copy returns Payload {
    return Payload { data: core.bytes(length, core.add_wrapping(seed, core.index())) }
}
// The outer id starts initialization before its nested producer returns.
func entry arg id u64 as readonly copy arg length u64 as readonly copy returns Entry {
    return Entry { id: id, payload: payload(length, id) }
}
// This nested failure must discard the outer id without publishing a partial record.
func rejected_payload arg length u64 as readonly copy returns Payload {
    core.reject_if(true)
    return Payload { data: core.bytes(length, 99) }
}
func rejected_entry returns Entry {
    return Entry { id: 99, payload: rejected_payload(7) }
}
// Observe every successful and rejected state without embedding physical-layout assumptions.
func main returns u64 {
    let memory = core.region(128) else { core.assert(false); return 0 }
    {
        let items = core.batch(memory, Entry)
        let initial = core.append(items, entry(7, 3))
        core.assert(initial == 0)
        {
            let first = core.borrow_readwrite(items, 0)
            let reader = core.alias_readonly(first)
            let original = core.address(first)
            let second_status = core.append(items, entry(2, 1))
            let third_status = core.append(items, entry(3, 0))
            core.assert(second_status == 0)
            core.assert(third_status == 0)
            first.id = 9
            first.payload.data[1] = 77
            core.assert(core.address(reader) == original)
            let second = core.borrow_readonly(items, 1)
            core.print(core.sub_wrapping(core.address(second), original))
            core.print(second.id)
            let before = core.used(memory)
            let full = core.append(items, entry(99, 1000))
            core.print(full)
            let rejected = core.append(items, rejected_entry())
            core.print(rejected)
            core.assert(core.used(memory) == before)
            core.assert(core.len(items) == 3)
            core.assert(core.address(first) == original)
            var sum = 0
            // A physical cursor must follow the selected record shape without copying data.
            for readonly item in items {
                sum = core.add_wrapping(sum, item.id)
                for byte in core.range(core.len(item.payload.data)) {
                    sum = core.add_wrapping(sum, item.payload.data[byte])
                }
            }
            core.print(sum)
            core.print(core.len(items))
            core.print(core.used(memory))
            core.print(reader.id)
            core.print(reader.payload.data[1])
        }
        core.reset(items)
        core.print(core.used(memory))
        let reused = core.append(items, entry(42, 0))
        core.assert(reused == 0)
        core.print(core.used(memory))
    }
    core.print(core.used(memory))
    return 0
}
"#;

/// Mixed layouts force retained headers, derived parents, fixed-unit skipping and unaligned u64.
/// The second tail batch also appends during a captured-prefix scan.
const COMPLEX: &str = r#"
// Empty values retain positive stride; fixed nested fields can follow that byte.
struct Empty {}
struct Fixed { zero Empty, marker u64 }
// Multiple variable fields require a retained total-size header in this experiment.
struct Pair { first Bytes, middle u64, second Bytes }
// A variable nested field before a scalar also retains its outer size header.
struct Wrapper { tag u64, pair Pair, suffix u64 }
// This parent may derive size from its last child's retained header.
struct TailWrap { head Fixed, pair Pair }
func pair arg left u64 as readonly copy arg right u64 as readonly copy arg seed u64 as readonly copy returns Pair {
    return Pair {
        first: core.bytes(left, core.add_wrapping(seed, core.index())),
        middle: 11,
        second: core.bytes(right, core.add_wrapping(core.add_wrapping(seed, 10), core.index()))
    }
}
func wrapped arg left u64 as readonly copy arg right u64 as readonly copy arg seed u64 as readonly copy returns Wrapper {
    return Wrapper { tag: 1, pair: pair(left, right, seed), suffix: 3 }
}
func tail arg seed u64 as readonly copy returns TailWrap {
    return TailWrap { head: Fixed { zero: Empty {}, marker: 100 }, pair: pair(0, 1, seed) }
}
func main returns u64 {
    let memory = core.region(512) else { core.assert(false); return 0 }
    {
        let rows = core.batch(memory, Wrapper)
        let first_status = core.append(rows, wrapped(1, 2, 5))
        core.assert(first_status == 0)
        let first = core.borrow_readwrite(rows, 0)
        let address = core.address(first)
        let second_status = core.append(rows, wrapped(0, 3, 6))
        core.assert(second_status == 0)
        first.suffix = 17
        first.pair.second[1] = 77
        core.assert(core.address(first) == address)
        let second = core.borrow_readonly(rows, 1)
        core.print(core.sub_wrapping(core.address(second), address))
        core.print(core.used(memory))
        var sum = 0
        // Scalar fields after variable components must remain accessible through the schema.
        for readonly row in rows {
            sum = core.add_wrapping(sum, row.tag)
            sum = core.add_wrapping(sum, row.pair.middle)
            sum = core.add_wrapping(sum, row.suffix)
            for byte in core.range(core.len(row.pair.first)) {
                sum = core.add_wrapping(sum, row.pair.first[byte])
            }
            for byte in core.range(core.len(row.pair.second)) {
                sum = core.add_wrapping(sum, row.pair.second[byte])
            }
        }
        core.print(sum)
        core.print(first.suffix)
        core.print(core.len(first.pair.first))
        core.print(core.len(first.pair.second))
        core.print(second.pair.second[2])
    }
    {
        let tails = core.batch(memory, TailWrap)
        let initial = core.append(tails, tail(1))
        core.assert(initial == 0)
        {
            let first = core.borrow_readwrite(tails, 0)
            first.head.marker = 101
            first.pair.middle = 12
            core.print(core.used(memory))
            core.print(first.head.marker)
            core.print(first.pair.middle)
            core.print(first.pair.second[0])
        }
        var visits = 0
        // Newly appended records do not extend the prefix captured by this scan.
        for readonly item in tails {
            let grown = core.append(tails, tail(2))
            core.assert(grown == 0)
            visits = core.add_wrapping(visits, 1)
            core.print(item.head.marker)
        }
        core.print(visits)
        core.print(core.len(tails))
        core.print(core.used(memory))
    }
    core.print(core.used(memory))
    return 0
}
"#;

/// Huge logical lengths hit width/capacity checks in tiny backing, without huge allocation.
/// The maximal u32 value fits its encoding and must report capacity, while its
/// successor reports width overflow only under the explicit compact selection.
const BOUNDARIES: &str = r#"
struct Blob { data Bytes }
func blob arg length u64 as readonly copy returns Blob {
    return Blob { data: core.bytes(length, core.index()) }
}
func rejected arg length u64 as readonly copy returns Blob {
    core.reject_if(true)
    return Blob { data: core.bytes(length, core.index()) }
}
func main returns u64 {
    let memory = core.region(64) else { core.assert(false); return 0 }
    let rows = core.batch(memory, Blob)
    let initial = core.append(rows, blob(0))
    core.assert(initial == 0)
    {
        let first = core.borrow_readonly(rows, 0)
        let original = core.address(first)
        let largest_narrow = core.append(rows, blob(4294967295))
        core.print(largest_narrow)
        let beyond_narrow = core.append(rows, blob(4294967296))
        core.print(beyond_narrow)
        let overflow = core.append(rows, blob(18446744073709551615))
        core.print(overflow)
        let early_rejection = core.append(rows, rejected(18446744073709551615))
        core.print(early_rejection)
        core.print(core.len(rows))
        core.print(core.used(memory))
        core.print(core.len(first.data))
        core.assert(core.address(first) == original)
    }
    core.reset(rows)
    core.print(core.used(memory))
    let reused = core.append(rows, blob(2))
    core.assert(reused == 0)
    core.print(core.used(memory))
    return 0
}
"#;

/// A field extent that fits u64 still overflows when added to an existing top.
/// Tiny backing must report arithmetic overflow before capacity exhaustion, and
/// failed unpublished construction must preserve the earlier empty record.
const TOP_OVERFLOW: &str = r#"
// This record contains one inline frozen byte payload.
struct Blob { data Bytes }
// Build directly into the receiving slot; every byte would use its scoped index.
func blob arg length u64 as readonly copy returns Blob {
    return Blob { data: core.bytes(length, core.index()) }
}
// Nonzero published top distinguishes addition overflow from field-size overflow.
func main returns u64 {
    let memory = core.region(128) else { core.assert(false); return 0 }
    let rows = core.batch(memory, Blob)
    let initial = core.append(rows, blob(0))
    core.assert(initial == 0)
    {
        let first = core.borrow_readonly(rows, 0)
        let original = core.address(first)
        let before = core.used(memory)
        // length+8 fits u64, but adding that extent to an existing nonzero top does not.
        let overflow = core.append(rows, blob(18446744073709551606))
        core.print(overflow)
        core.print(core.len(rows))
        core.print(core.used(memory))
        core.print(core.len(first.data))
        core.assert(core.used(memory) == before)
        core.assert(core.address(first) == original)
    }
    // The failed suffix leaves no publication and reset permits ordinary reuse.
    core.reset(rows)
    core.print(core.used(memory))
    let reused = core.append(rows, blob(3))
    core.assert(reused == 0)
    let current = core.borrow_readonly(rows, 0)
    core.print(core.used(memory))
    core.print(core.len(current.data))
    return 0
}
"#;

#[test]
/// Exact 32/16/12-byte fixed parts preserve values, aliases, rollback and reset reuse.
fn unary_tail_layouts_preserve_growth_and_publication() {
    // Three records have payload lengths3,1,0. Their values sum109 after mutation.
    compare(
        "unary-tail",
        TAIL,
        [
            &[35, 2, 1, 3, 109, 3, 100, 9, 77, 0, 32, 0],
            &[19, 2, 1, 3, 109, 3, 52, 9, 77, 0, 16, 0],
            &[15, 2, 1, 3, 109, 3, 40, 9, 77, 0, 12, 0],
        ],
    );
}

#[test]
/// Mixed retained/derived schemas preserve every nested projection and cursor boundary.
fn complex_shapes_and_empty_units_remain_correct() {
    // Wrapper images are59/59/51 bytes; fixed+retained-tail images are65/42/34.
    compare(
        "complex-shapes",
        COMPLEX,
        [
            &[
                59, 118, 192, 17, 1, 2, 18, 65, 101, 12, 11, 101, 1, 2, 130, 0,
            ],
            &[
                59, 118, 192, 17, 1, 2, 18, 42, 101, 12, 11, 101, 1, 2, 84, 0,
            ],
            &[
                51, 102, 192, 17, 1, 2, 18, 34, 101, 12, 11, 101, 1, 2, 68, 0,
            ],
        ],
    );
}

#[test]
/// Narrow overflow precedes truncation and leaves existing published storage unchanged.
fn compact_length_boundaries_reject_without_large_allocations() {
    compare(
        "length-boundaries",
        BOUNDARIES,
        [
            &[1, 1, 2, 3, 1, 16, 0, 0, 18],
            &[1, 1, 2, 3, 1, 8, 0, 0, 10],
            &[1, 2, 2, 3, 1, 4, 0, 0, 6],
        ],
    );
}

#[test]
/// Region-top addition overflow precedes finite capacity and preserves published data.
fn top_plus_payload_overflow_precedes_capacity() {
    compare(
        "top-addition-overflow",
        TOP_OVERFLOW,
        [
            &[2, 1, 16, 0, 0, 19, 3],
            &[2, 1, 8, 0, 0, 11, 3],
            &[2, 1, 4, 0, 0, 7, 3],
        ],
    );
}

/// Parse/check real source once and independently check every explicit representation.
/// Expected arrays are manually calculated; the model must agree before native work.
fn compare(label: &str, source: &str, expected: [&[u64]; 3]) {
    let input = parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render(label, source)));
    let checked = input
        .check()
        .unwrap_or_else(|error| panic!("{}", error.render(label, source)));
    let tools = NativeToolchain::discover().expect("layout native tests require matched LLVM");
    let scratch = TempDir::new().unwrap();
    // Each mode is an explicit fixture, never a silent fallback selected from benchmark names.
    for (position, layout) in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ]
    .into_iter()
    .enumerate()
    {
        let modeled = if layout == LayoutMode::Wide {
            model::run(input.program())
        } else {
            model::run_with_layout(input.program(), layout)
        }
        .unwrap();
        assert_eq!(
            modeled, expected[position],
            "independent {label}/{layout:?}"
        );
        let emitted =
            holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        assert!(!emitted.contains("noalias"));
        assert!(!emitted.contains("@realloc"));
        let expected_text = expected[position]
            .iter()
            .map(|value| format!("{value}\n"))
            .collect::<String>();
        // Optimization and sanitizer modes execute identical source and layout semantics.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch.path().join(format!("{layout:?}-{mode}"));
            let build = match mode {
                "asan" => tools.build_sanitized(&emitted, &binary),
                "o3" => tools.build(&emitted, &binary, true),
                _ => tools.build(&emitted, &binary, false),
            }
            .unwrap_or_else(|error| {
                failure(
                    label, source, &emitted, &tools, layout, mode, &error, &binary,
                )
            });
            let output = Command::new(&binary)
                .env("ASAN_OPTIONS", "detect_leaks=0")
                .output()
                .unwrap();
            // A trap, extra output or wrong byte count rejects the complete native observation.
            if !output.status.success() || output.stdout != expected_text.as_bytes() {
                failure(
                    label,
                    source,
                    &emitted,
                    &tools,
                    layout,
                    mode,
                    &format!(
                        "commands: {:#?}\nstatus: {}\nexpected: {expected_text:?}\nstdout: {}\nstderr: {}",
                        build.commands,
                        output.status,
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    ),
                    &binary,
                );
            }
        }
    }
}

/// Preserve complete source/LLVM/tool/native context before rejecting a discrepancy.
/// Existing evidence is never overwritten and temporary binaries are copied before cleanup.
#[allow(clippy::too_many_arguments)]
fn failure(
    label: &str,
    source: &str,
    llvm: &str,
    tools: &NativeToolchain,
    layout: LayoutMode,
    mode: &str,
    detail: &str,
    binary: &Path,
) -> ! {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CLI package must live directly under the workspace root")
        .join("build/failures")
        .join(format!("layout-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("input.holo"), source).unwrap();
    fs::write(directory.join("program.ll"), llvm).unwrap();
    fs::write(
        directory.join("observed.txt"),
        format!("layout: {layout:?}\nmode: {mode}\n{detail}\n"),
    )
    .unwrap();
    fs::write(
        directory.join("toolchain.txt"),
        format!(
            "{}\ntarget: {}\nopt: {}\nclang: {}\n",
            tools.identity,
            tools.target_triple(),
            tools.opt.display(),
            tools.clang.display()
        ),
    )
    .unwrap();
    // Failed compilation may produce no binary; completed native code remains replayable.
    if binary.is_file() {
        fs::copy(binary, directory.join("native")).unwrap();
    }
    panic!(
        "{label}/{layout:?}/{mode}: {detail}\nevidence: {}",
        directory.display()
    );
}
