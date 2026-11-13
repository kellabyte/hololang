//! Native regressions for width-aware byte-initializer expression lowering.
//!
//! Hand-reduced byte formulas and the independent full-u64 value model validate
//! every supported wrapping/bitwise operation across real IR and parsed source.
//! Lengths, indices, scalar fields, producer arguments, publication and rollback
//! remain full-width semantics. O0/O3/ASan run actual checked native programs;
//! no timing result or symbolic expression mirror counts as conformance evidence.
//! Exact eight-byte prefix/tail boundaries and mixed helper applications protect
//! later vector-prefix experiments without assuming their code-generation shape.

#[path = "../../support/model/mod.rs"]
mod model;

use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::parse_experimental;
use holo::ir::{
    Access, Binary, Build, Compare, Condition, Expr, Field, FieldKind, Op, Producer, Program,
    Record,
};
use holo::ir::{Argument, Parameter};
use holo::layout::LayoutMode;
use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Vector-boundary and byte-counter-boundary lengths, including an empty payload.
const LENGTHS: [u64; 20] = [
    0, 1, 7, 8, 9, 15, 16, 17, 23, 24, 31, 32, 33, 63, 64, 65, 255, 256, 257, 511,
];
/// Ordered byte fields, each with one distinct operation or a mixed nested tree.
const BYTE_FIELDS: [&str; 6] = ["add", "sub", "mul", "xor", "and", "nested"];
/// High application bits must survive outside the byte-initializer context.
const TAG: u64 = 0xfedc_ba98_7654_3210;
/// This input has both bit63 and low bits set, unlike a small scalar substitute.
const OTHER: u64 = 0x8000_0000_0000_0082;

/// Own an exact unsigned fixture constant without changing scalar width.
fn c(value: u64) -> Expr {
    Expr::Const(value)
}

/// Build one explicit wrapping/bitwise u64 source expression for the checked IR.
fn binary(op: Binary, left: Expr, right: Expr) -> Expr {
    Expr::binary(op, left, right)
}

/// Read a named initialized scalar; no target memory or parameter is copied here.
fn local(name: &str) -> Expr {
    Expr::Local(name.into())
}

/// Compare two complete unsigned values for an actual native assertion.
fn equal(left: Expr, right: Expr) -> Condition {
    Condition {
        op: Compare::Eq,
        left,
        right,
    }
}

/// Observe an initialized byte through the source's retained record reference.
fn byte(reference: &str, field: &str, index: Expr) -> Expr {
    Expr::Byte {
        reference: reference.into(),
        path: vec![field.into()],
        index: Box::new(index),
    }
}

/// Observe frozen byte length, which must stay u64 even when its values use i8.
fn length(reference: &str, field: &str) -> Expr {
    Expr::BytesLen {
        reference: reference.into(),
        path: vec![field.into()],
    }
}

/// Read one full-width application field rather than a byte representation.
fn scalar(reference: &str, field: &str) -> Expr {
    Expr::Read {
        reference: reference.into(),
        path: vec![field.into()],
    }
}

/// Derive one byte by a hand-reduced closed formula for fixed seed low bytes 255/130.
/// This is fixture arithmetic, not a traversal of compiler expressions or layout plans.
fn expected_byte(field: &str, index: u64) -> u64 {
    let j = index % 256;
    // Each formula follows modular arithmetic independently of the emitted expression tree.
    match field {
        "add" => (255 + j) % 256,
        "sub" => (386 - j) % 256,
        "mul" => (256 - j) % 256,
        "xor" => 130 ^ j,
        "and" => 130 & (255 ^ j),
        "nested" => ((129 * ((j + 1) % 256)) % 256) ^ (((130 + j) % 256) & 243),
        _ => panic!("unknown fixture byte field {field}"),
    }
}

/// Construct all five operations plus a nested tree from u64 parameters and index.
/// Parameter 0 is length, 1 is u64::MAX, 2 is OTHER, 3 is the high application tag.
fn initializer(field: &str) -> Expr {
    let a = Expr::Param(1);
    let b = Expr::Param(2);
    // The mixed tree includes overflow, subtraction underflow, high mask bits,
    // and index wrapping while retaining the original full-u64 semantic input.
    match field {
        "add" => binary(Binary::AddWrapping, a, Expr::Index),
        "sub" => binary(Binary::SubWrapping, b, Expr::Index),
        "mul" => binary(Binary::MulWrapping, a, Expr::Index),
        "xor" => binary(Binary::Xor, b, Expr::Index),
        "and" => binary(Binary::And, b, binary(Binary::Xor, a, Expr::Index)),
        "nested" => binary(
            Binary::Xor,
            binary(
                Binary::MulWrapping,
                binary(Binary::AddWrapping, a.clone(), b.clone()),
                binary(Binary::SubWrapping, Expr::Index, a),
            ),
            binary(
                Binary::And,
                binary(Binary::AddWrapping, b, Expr::Index),
                c(0xf0f0_0000_0000_00f3),
            ),
        ),
        _ => panic!("unknown fixture initializer {field}"),
    }
}

/// Append one fully parameterized operation record; success is required or printed explicitly.
/// The helper scopes its status name so repeated attempts cannot shadow publication facts.
fn append(len: u64, expected_success: bool) -> Op {
    Op::Scope(vec![
        Op::Append {
            batch: "rows".into(),
            producer: "operations".into(),
            args: vec![c(len), c(u64::MAX), c(OTHER), c(TAG)]
                .into_iter()
                .map(Argument::Scalar)
                .collect(),
            status: "status".into(),
        },
        // Ordinary successful construction and deliberately failed reservation are different witnesses.
        if expected_success {
            Op::Assert(equal(local("status"), c(0)))
        } else {
            Op::Print(local("status"))
        },
    ])
}

/// Build an IR fixture that prints every byte and verifies retained aliases/reset reuse.
/// Expected sizes follow this record's retained header and selected length width,
/// independently of compiler planner helpers. Failed large appends use tiny backing.
fn ir_fixture(layout: LayoutMode) -> (Program, Vec<u64>) {
    // Multiple variable fields retain the outer header; only byte-length width differs.
    let base = if layout == LayoutMode::Compact32 {
        48
    } else {
        72
    };
    let total = base * LENGTHS.len() as u64 + 6 * LENGTHS.iter().sum::<u64>();
    let mut fields = vec![Field {
        name: "tag".into(),
        kind: FieldKind::U64,
    }];
    let mut body = vec![Build::U64 {
        field: "tag".into(),
        value: Expr::Param(3),
    }];
    // Every operation receives its own readable byte field, preserving field initialization order.
    for field in BYTE_FIELDS {
        fields.push(Field {
            name: field.into(),
            kind: FieldKind::Bytes,
        });
        body.push(Build::Bytes {
            field: field.into(),
            len: Expr::Param(0),
            value: initializer(field),
        });
    }
    fields.push(Field {
        name: "tail".into(),
        kind: FieldKind::U64,
    });
    body.push(Build::U64 {
        field: "tail".into(),
        value: Expr::Param(2),
    });
    let mut scan = vec![
        Op::Print(scalar("entry", "tag")),
        Op::Print(scalar("entry", "tail")),
    ];
    // Print every initialized byte rather than relying solely on a collision-prone sum.
    for field in BYTE_FIELDS {
        scan.push(Op::Print(length("entry", field)));
        scan.push(Op::Repeat {
            count: length("entry", field),
            index: "i".into(),
            body: vec![Op::Print(byte("entry", field, local("i")))],
        });
    }
    let mut retained = vec![
        Op::Borrow {
            name: "first".into(),
            batch: "rows".into(),
            index: c(1),
            access: Access::Readwrite,
        },
        Op::Alias {
            name: "reader".into(),
            source: "first".into(),
            access: Access::Readonly,
        },
        Op::Let {
            name: "address".into(),
            value: Expr::Address("first".into()),
        },
    ];
    // The one-byte record's alias remains live while larger values extend the same batch.
    for len in LENGTHS.into_iter().skip(2) {
        retained.push(append(len, true));
    }
    retained.extend([
        Op::ForEach {
            batch: "rows".into(),
            reference: "entry".into(),
            access: Access::Readonly,
            body: scan,
        },
        Op::Print(Expr::Used("memory".into())),
        append(100000, false),
        Op::Assert(equal(Expr::Used("memory".into()), c(total))),
        Op::Assert(equal(
            Expr::BatchLen("rows".into()),
            c(LENGTHS.len() as u64),
        )),
        Op::Assert(equal(Expr::Address("reader".into()), local("address"))),
        Op::Print(scalar("reader", "tag")),
        Op::Print(scalar("reader", "tail")),
    ]);
    // Hand witnesses protect each operator independently of the model's full-u64 evaluator.
    for field in BYTE_FIELDS {
        retained.push(Op::Print(byte("reader", field, c(0))));
        retained.push(Op::Assert(equal(
            byte("reader", field, c(0)),
            c(expected_byte(field, 0)),
        )));
    }
    retained.extend([
        Op::WriteByte {
            reference: "first".into(),
            path: vec!["add".into()],
            index: c(0),
            value: c(77),
        },
        Op::Print(byte("reader", "add", c(0))),
    ]);
    let mut reuse = vec![Op::Borrow {
        name: "rebuilt".into(),
        batch: "rows".into(),
        index: c(0),
        access: Access::Readonly,
    }];
    // Reset must remove old initialized values before a fresh two-byte payload becomes visible.
    for field in BYTE_FIELDS {
        reuse.push(Op::Print(byte("rebuilt", field, c(1))));
    }
    let program = Program {
        records: vec![Record {
            name: "Operations".into(),
            fields,
        }],
        producers: vec![Producer {
            name: "operations".into(),
            record: "Operations".into(),
            error: None,
            params: vec![Parameter::U64; 4],
            body,
        }],
        body: vec![Op::Region {
            name: "memory".into(),
            capacity: c(20000),
            fail_acquire: false,
            body: vec![
                Op::Batch {
                    name: "rows".into(),
                    region: "memory".into(),
                    record: "Operations".into(),
                    body: vec![
                        append(0, true),
                        append(1, true),
                        Op::Scope(retained),
                        Op::Reset {
                            batch: "rows".into(),
                        },
                        Op::Print(Expr::Used("memory".into())),
                        append(2, true),
                        Op::Scope(reuse),
                        Op::Print(Expr::Used("memory".into())),
                    ],
                },
                Op::Print(Expr::Used("memory".into())),
            ],
            on_failure: vec![Op::Assert(equal(c(0), c(1)))],
        }],
    };
    let mut expected = Vec::new();
    // These closed formulas independently enumerate the exact native print stream.
    for len in LENGTHS {
        expected.extend([TAG, OTHER]);
        // Each field has the same extent but its own independent byte formula.
        for field in BYTE_FIELDS {
            expected.push(len);
            // Enumerate the whole payload so errors cannot hide behind checksum collisions.
            for index in 0..len {
                expected.push(expected_byte(field, index));
            }
        }
    }
    expected.extend([total, 1, TAG, OTHER]);
    expected.extend(BYTE_FIELDS.map(|field| expected_byte(field, 0)));
    expected.extend([77, 0]);
    expected.extend(BYTE_FIELDS.map(|field| expected_byte(field, 1)));
    expected.extend([base + 12, 0]);
    (program, expected)
}

/// Real source exercises pure helper expansion while scalar calls retain high bits.
/// Index 256 must execute and remain distinct from the end of a 257-byte payload.
const SOURCE: &str = r#"
// Scalar data surrounds the frozen byte field, requiring full-width projections.
struct Packet { id u64, data Bytes, tail u64 }
// The same full-u64 helper is used inside and outside a byte initializer.
func mix
    arg a u64 as readonly copy
    arg b u64 as readonly copy
    arg index u64 as readonly copy
    returns u64
{
    return core.xor(
        core.mul_wrapping(core.add_wrapping(a, b), core.sub_wrapping(index, a)),
        core.and(core.add_wrapping(b, index), 0xf0f00000000000f3)
    )
}
// Producer arguments and scalar fields remain u64, including the length257.
func packet
    arg id u64 as readonly copy
    arg length u64 as readonly copy
    arg a u64 as readonly copy
    arg b u64 as readonly copy
    returns Packet
{
    return Packet { id: id, data: core.bytes(length, mix(a, b, core.index())), tail: b }
}
// Keep existing byte aliases live through growth/failure, then reset and reconstruct.
func main returns u64 {
    let memory = core.region(8192) else { core.assert(false); return 0 }
    let rows = core.batch(memory, Packet)
    let first_status = core.append(rows, packet(0xfedcba9876543210, 257, core.sub_wrapping(0, 1), core.add_wrapping(0x8000000000000000, 130)))
    core.assert(first_status == 0)
    {
        let first = core.borrow_readwrite(rows, 0)
        let reader = core.alias_readonly(first)
        let original = core.address(first)
        core.print(reader.id)
        core.print(reader.tail)
        core.print(core.len(reader.data))
        core.print(reader.data[0])
        core.print(reader.data[1])
        core.print(reader.data[255])
        core.print(reader.data[256])
        // Outside byte construction the helper must still return its complete u64 result.
        core.print(mix(18446744073709551615, 0x8000000000000082, 1))
        first.data[256] = 201
        let grown = core.append(rows, packet(0xfedcba9876543210, 0, 18446744073709551615, 0x8000000000000082))
        core.assert(grown == 0)
        let before = core.used(memory)
        let full = core.append(rows, packet(99, 100000, 18446744073709551615, 0x8000000000000082))
        core.print(full)
        let length_limit = core.append(rows, packet(99, 4294967296, 18446744073709551615, 0x8000000000000082))
        core.print(length_limit)
        core.assert(core.used(memory) == before)
        core.assert(core.address(reader) == original)
        core.print(core.len(rows))
        core.print(core.used(memory))
        core.print(reader.data[256])
    }
    core.reset(rows)
    core.print(core.used(memory))
    let reused = core.append(rows, packet(0xfedcba9876543210, 1, 18446744073709551615, 0x8000000000000082))
    core.assert(reused == 0)
    {
        let fresh = core.borrow_readonly(rows, 0)
        core.print(core.len(fresh.data))
        core.print(fresh.data[0])
        core.print(fresh.id)
    }
    core.print(core.used(memory))
    return 0
}
"#;

/// Return parsed ordinary Holo source and hand-calculated expected output for its mode.
fn source_fixture(layout: LayoutMode) -> (Program, Vec<u64>) {
    let input = parse_experimental(SOURCE)
        .unwrap_or_else(|error| panic!("{}", error.render("byte-source.holo", SOURCE)));
    input.check().unwrap();
    // The non-tail byte field retains a header; Compact32 alone rejects 2^32 at width check.
    let (used, reused, length_limit) = if layout == LayoutMode::Compact32 {
        (313, 29, 2)
    } else {
        (321, 33, 1)
    };
    (
        input.program().clone(),
        vec![
            TAG,
            OTHER,
            257,
            3,
            129,
            129,
            3,
            0x8000_0000_0000_0181,
            1,
            length_limit,
            2,
            used,
            201,
            0,
            1,
            3,
            TAG,
            reused,
        ],
    )
}

/// Two byte fields share full-width scalar arguments but have distinct expression forms.
/// The first is a closed arithmetic initializer; the second contains nested eager
/// helper applications and must retain correct behavior through scalar fallback.
const MIXED_SOURCE: &str = r#"
// Multiple variable fields retain a size header in every explicit representation.
struct Mixed { tag u64, generated Bytes, applied Bytes, tail u64 }
// Preserve the complete copied value outside byte stores, including high bits.
func step
    with noalloc, noblock
    arg value u64 as readonly copy
    arg index u64 as readonly copy
    returns u64
{
    return core.xor(core.add_wrapping(value, index), 42)
}
// Both fields observe the same immutable scalar actual and physical byte index.
func mixed
    with noblock
    arg length u64 as readonly copy
    arg seed u64 as readonly copy
    returns Mixed
{
    return Mixed {
        tag: 0xfedcba9876543210
        generated: core.bytes(length, core.add_wrapping(seed, core.index()))
        applied: core.bytes(length, step(step(seed, core.index()), core.index()))
        tail: 0x8000000000000082
    }
}
// A complete vector prefix, one-byte tail and index256 all remain observable.
func main returns u64 {
    let memory = core.region(1024) else { core.assert(false); return 0 }
    {
        let rows = core.batch(memory, Mixed)
        let first = core.append(rows, mixed(9, 18446744073709551615))
        let second = core.append(rows, mixed(257, 18446744073709551615))
        core.assert(first == 0)
        core.assert(second == 0)
        // Failed field reservation must not publish a partial third record.
        let failed = core.append(rows, mixed(4096, 18446744073709551615))
        core.print(failed)
        core.print(core.len(rows))
        // Scan every published byte so a correct checksum cannot hide lane errors.
        for readonly item in rows {
            core.print(item.tag)
            core.print(item.tail)
            core.print(core.len(item.generated))
            // The generated field covers full u64 indices, narrowed only at stores.
            for index in core.range(core.len(item.generated)) {
                core.print(item.generated[index])
            }
            core.print(core.len(item.applied))
            // Nested helper parameter frames must not inherit or lose outer indices.
            for index in core.range(core.len(item.applied)) {
                core.print(item.applied[index])
            }
        }
        core.print(core.used(memory))
        // This ordinary helper call returns its full u64 result rather than a byte.
        core.print(step(18446744073709551615, 2))
    }
    core.print(core.used(memory))
    return 0
}
"#;

/// Hand-enumerate mixed direct/helper field values independently of compiler trees.
/// `layout` changes only two length headers; mathematical byte formulas use low
/// seed255 while the printed scalar tag/tail and ordinary helper remain u64.
fn mixed_source_fixture(layout: LayoutMode) -> (Program, Vec<u64>) {
    let source = parse_experimental(MIXED_SOURCE)
        .unwrap_or_else(|error| panic!("{}", error.render("mixed-byte.holo", MIXED_SOURCE)));
    source.check().unwrap();
    let mut expected = vec![1, 2];
    // Enumerate both complete physical-index ranges, including257's index256.
    for len in [9, 257] {
        expected.extend([TAG, OTHER, len]);
        for index in 0..len {
            expected.push((255 + index) % 256);
        }
        expected.push(len);
        for index in 0..len {
            let inner = ((255 + index) % 256) ^ 42;
            expected.push(((inner + index) % 256) ^ 42);
        }
    }
    // Each record has an8-byte size header, two8-byte scalars and two lengths.
    // Compact alone uses4-byte lengths, saving8 bytes per published record.
    let prefix = if layout == LayoutMode::Compact32 {
        32
    } else {
        40
    };
    expected.extend([2 * prefix + 2 * (9 + 257), 43, 0]);
    (source.program().clone(), expected)
}

#[test]
/// All supported operators match full-u64 semantics across zero/vector/index-width boundaries.
fn all_byte_operations_and_index_boundaries_preserve_values() {
    compare("byte-ir", ir_fixture);
}

#[test]
/// Parsed helpers retain scalar/argument/length widths while byte payloads narrow at the store.
fn source_helpers_keep_full_width_semantics_outside_byte_values() {
    compare("byte-source", source_fixture);
}

#[test]
/// Direct and nested-helper initializers preserve values, scalar ABI and failed publication.
fn mixed_direct_and_helper_initializers_preserve_full_width_arguments() {
    compare("mixed-byte-source", mixed_source_fixture);
}

/// Build/check one explicit mode at a time and compare full outputs against two independent oracles.
/// The bounded native runner prevents a mistakenly narrowed loop counter from hanging the suite.
fn compare(label: &str, fixture: fn(LayoutMode) -> (Program, Vec<u64>)) {
    let tools =
        NativeToolchain::discover().expect("byte initialization tests require matched LLVM");
    let scratch = TempDir::new().unwrap();
    // Mode-specific widths are checked independently; no compiler layout plan feeds the oracle.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let (input, expected) = fixture(layout);
        // Preserve the model's normal Wide API while selecting explicit alternate representations.
        let modeled = if layout == LayoutMode::Wide {
            model::run(&input)
        } else {
            model::run_with_layout(&input, layout)
        }
        .unwrap();
        assert_eq!(
            modeled, expected,
            "independent full-u64 model {label}/{layout:?}"
        );
        let checked = holo::check::check(&input).unwrap();
        let llvm = holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        let expected_text = expected
            .iter()
            .map(|value| format!("{value}\n"))
            .collect::<String>();
        // O0 observes direct lowering, O3 permits optimization, and ASan checks memory accesses.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch.path().join(format!("{layout:?}-{mode}"));
            // Each actual driver mode is selected explicitly, without changing the program.
            let result = match mode {
                "asan" => tools.build_sanitized(&llvm, &binary),
                "o3" => tools.build(&llvm, &binary, true),
                _ => tools.build(&llvm, &binary, false),
            };
            let build = result.unwrap_or_else(|error| {
                fail(label, &input, &llvm, &tools, layout, mode, &error, &binary)
            });
            let output = run_native(&binary).unwrap_or_else(|error| {
                fail(label, &input, &llvm, &tools, layout, mode, &error, &binary)
            });
            // Any abnormal completion or value discrepancy preserves the complete reproducer.
            if !output.status.success() || output.stdout != expected_text.as_bytes() {
                fail(
                    label,
                    &input,
                    &llvm,
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

/// Run one self-owned native child with file streams and a ten-second deadline.
/// Files avoid pipe backpressure from the complete byte-by-byte output; only this
/// test's child can be killed, and timeout is a failure with partial output retained.
fn run_native(binary: &Path) -> Result<Output, String> {
    let stdout_path = binary.with_extension("stdout");
    let stderr_path = binary.with_extension("stderr");
    let stdout = File::create_new(&stdout_path).map_err(|error| error.to_string())?;
    let stderr = File::create_new(&stderr_path).map_err(|error| error.to_string())?;
    let mut child = Command::new(binary)
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(10);
    // Poll without holding a pipe reader; each iteration either observes completion or advances time.
    let status = loop {
        // A completed child supplies its actual exit status without killing another process.
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        // A broken narrowed loop counter must fail promptly instead of hanging the test runner.
        if Instant::now() >= deadline {
            child.kill().map_err(|error| error.to_string())?;
            child.wait().map_err(|error| error.to_string())?;
            return Err("native byte initializer test exceeded 10 seconds".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(Output {
        status,
        stdout: fs::read(stdout_path).map_err(|error| error.to_string())?,
        stderr: fs::read(stderr_path).map_err(|error| error.to_string())?,
    })
}

/// Preserve complete fixture/tool/output identities before rejecting a native mismatch.
/// Evidence-I/O failure reports its cause alongside the original discrepancy.
#[allow(clippy::too_many_arguments)]
fn fail(
    label: &str,
    input: &Program,
    llvm: &str,
    tools: &NativeToolchain,
    layout: LayoutMode,
    mode: &str,
    detail: &str,
    binary: &Path,
) -> ! {
    let save = || -> Result<std::path::PathBuf, std::io::Error> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CLI package must live directly under the workspace root")
            .join("build/failures")
            .join(format!("{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&directory)?;
        fs::write(directory.join("input.txt"), format!("{input:#?}\n"))?;
        fs::write(directory.join("program.ll"), llvm)?;
        fs::write(
            directory.join("observed.txt"),
            format!("layout: {layout:?}\nmode: {mode}\n{detail}\n"),
        )?;
        fs::write(
            directory.join("tools.txt"),
            format!(
                "{}\nopt: {}\nclang: {}\n",
                tools.identity,
                tools.opt.display(),
                tools.clang.display()
            ),
        )?;
        // Native files may be missing after compiler failure; preserve every available stream/artifact.
        for (source, name) in [
            (binary.to_path_buf(), "native"),
            (binary.with_extension("stdout"), "native.stdout"),
            (binary.with_extension("stderr"), "native.stderr"),
        ] {
            if source.is_file() {
                fs::copy(source, directory.join(name))?;
            }
        }
        Ok(directory)
    };
    // Both archive outcomes retain the original discrepancy rather than disguising it as success.
    match save() {
        Ok(path) => panic!(
            "{label}/{layout:?}/{mode}: {detail}\nevidence: {}",
            path.display()
        ),
        Err(error) => panic!("{label}/{layout:?}/{mode}: {detail}\nevidence write failed: {error}"),
    }
}
