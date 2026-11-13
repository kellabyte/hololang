//! Behavioral regressions for ordered finite-region reservation lowering.
//!
//! Real checked programs exercise exact fits, shortage, failure precedence, and
//! stable published prefixes at O0/O3/ASan under every explicit representation.
//! Hand-derived outputs are independently checked against the owned-value model.
//! The same observable semantics apply to the preserved production lowering;
//! rejected optimization-specific arithmetic exercises remain in its evidence archive.

#[path = "../../support/model/mod.rs"]
mod model;

use holo::driver::{NativeToolchain, TempDir};
use holo::ir::{
    Access, Build, Compare, Condition, Expr, Field, FieldKind, Op, Producer, Program, Record,
};
use holo::ir::{Argument, Parameter};
use holo::layout::LayoutMode;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Build the same small logical schema/producer set for every reservation fixture.
/// Only source-level plain data and closed pure calls appear; no native layout
/// helper supplies expected capacities, offsets, or status classification.
fn program(body: Vec<Op>) -> Program {
    Program {
        records: vec![
            Record {
                name: "Empty".into(),
                fields: vec![],
            },
            Record {
                name: "Scalar".into(),
                fields: vec![field("value", FieldKind::U64)],
            },
            Record {
                name: "Pair".into(),
                fields: vec![field("a", FieldKind::U64), field("b", FieldKind::U64)],
            },
            Record {
                name: "Tail".into(),
                fields: vec![
                    field("first", FieldKind::U64),
                    field("data", FieldKind::Bytes),
                ],
            },
            Record {
                name: "Outer".into(),
                fields: vec![
                    field("tag", FieldKind::U64),
                    field("child", FieldKind::Record("Pair".into())),
                    field("tail", FieldKind::Bytes),
                ],
            },
        ],
        producers: vec![
            producer("empty", "Empty", 0, vec![]),
            producer(
                "empty_reject",
                "Empty",
                0,
                vec![Build::FailIf(equal(c(1), c(1)))],
            ),
            producer("scalar", "Scalar", 1, vec![scalar("value", Expr::Param(0))]),
            producer(
                "pair",
                "Pair",
                0,
                vec![scalar("a", c(2)), scalar("b", c(3))],
            ),
            producer(
                "pair_reject",
                "Pair",
                0,
                vec![
                    scalar("a", c(7)),
                    Build::FailIf(equal(c(1), c(1))),
                    scalar("b", c(8)),
                ],
            ),
            producer(
                "tail_reject",
                "Tail",
                0,
                vec![
                    scalar("first", c(11)),
                    Build::FailIf(equal(c(1), c(1))),
                    bytes("data", c(0)),
                ],
            ),
            producer(
                "tail_badlen",
                "Tail",
                0,
                vec![scalar("first", c(11)), bytes("data", c(u64::MAX))],
            ),
            producer(
                "outer",
                "Outer",
                0,
                vec![
                    scalar("tag", c(1)),
                    nested("child", "pair"),
                    bytes("tail", c(0)),
                ],
            ),
            producer(
                "outer_nested_reject",
                "Outer",
                0,
                vec![
                    scalar("tag", c(44)),
                    nested("child", "pair_reject"),
                    bytes("tail", c(u64::MAX)),
                ],
            ),
            producer(
                "outer_badlen",
                "Outer",
                0,
                vec![
                    scalar("tag", c(44)),
                    nested("child", "pair"),
                    bytes("tail", c(u64::MAX)),
                ],
            ),
        ],
        body,
    }
}

/// Own one logical field declaration; no representation bytes are computed here.
fn field(name: &str, kind: FieldKind) -> Field {
    Field {
        name: name.into(),
        kind,
    }
}

/// Own one pure destination constructor with explicitly ordered steps.
fn producer(name: &str, record: &str, params: usize, body: Vec<Build>) -> Producer {
    Producer {
        name: name.into(),
        record: record.into(),
        error: None,
        params: vec![Parameter::U64; params],
        body,
    }
}

/// Initialize one copied unsigned field in its receiving slot.
fn scalar(name: &str, value: Expr) -> Build {
    Build::U64 {
        field: name.into(),
        value,
    }
}

/// Initialize bytes with an explicit low-eight-bit index value, including zero length.
fn bytes(name: &str, len: Expr) -> Build {
    Build::Bytes {
        field: name.into(),
        len,
        value: Expr::Index,
    }
}

/// Invoke a closed zero-argument nested producer at the current field destination.
fn nested(field: &str, producer: &str) -> Build {
    Build::Record {
        field: field.into(),
        producer: producer.into(),
        args: vec![],
    }
}

/// Construct an exact unsigned literal without host wrapping conversions.
fn c(value: u64) -> Expr {
    Expr::Const(value)
}

/// Refer to one already initialized scalar fixture binding.
fn local(name: &str) -> Expr {
    Expr::Local(name.into())
}

/// Compare exact unsigned values; assertions execute only after ordered work.
fn equal(left: Expr, right: Expr) -> Condition {
    Condition {
        op: Compare::Eq,
        left,
        right,
    }
}

/// Read a typed initialized scalar through the current live reference.
fn read(reference: &str, path: &[&str]) -> Expr {
    Expr::Read {
        reference: reference.into(),
        path: path.iter().map(|part| (*part).into()).collect(),
    }
}

/// Observe a live backing marker; it cannot be used to modify allocator metadata.
fn used(region: &str) -> Expr {
    Expr::Used(region.into())
}

/// Observe the number of complete published records in one live batch.
fn count(batch: &str) -> Expr {
    Expr::BatchLen(batch.into())
}

/// Attempt one append in a fresh status scope and print its actual failure/success code.
fn attempt(batch: &str, producer: &str, args: Vec<Expr>) -> Op {
    Op::Scope(vec![
        Op::Append {
            batch: batch.into(),
            producer: producer.into(),
            args: args.into_iter().map(Argument::Scalar).collect(),
            status: "status".into(),
        },
        Op::Print(local("status")),
    ])
}

/// Own one finite region, aborting unexpected acquisition failure rather than skipping checks.
fn region(name: &str, capacity: u64, body: Vec<Op>) -> Op {
    Op::Region {
        name: name.into(),
        capacity: c(capacity),
        fail_acquire: false,
        body,
        on_failure: vec![Op::Assert(equal(c(0), c(1)))],
    }
}

/// Introduce a batch whose scope restores the original marker after its supplied body.
fn batch(name: &str, region: &str, record: &str, body: Vec<Op>) -> Op {
    Op::Batch {
        name: name.into(),
        region: region.into(),
        record: record.into(),
        body,
    }
}

/// Hand-calculate the fixed Scalar/Empty/Outer sizes for the explicitly chosen mode.
/// These constants follow the public fixture rules rather than reading LayoutPlan.
fn fixture_sizes(layout: LayoutMode) -> (u64, u64, u64, u64) {
    // Only Wide adds a record-size header; derived empty values retain a positive unit.
    match layout {
        LayoutMode::Wide => (8, 16, 8, 48),
        LayoutMode::Derived64 => (0, 8, 1, 32),
        LayoutMode::Compact32 => (0, 8, 1, 28),
    }
}

/// Construct exact-fit, zero-capacity, and one-byte-short cases plus marker reuse.
fn exact_fixture(layout: LayoutMode) -> (Program, Vec<u64>) {
    let (_, scalar_bytes, empty_bytes, _) = fixture_sizes(layout);
    let input = program(vec![
        region(
            "zero",
            0,
            vec![batch(
                "none",
                "zero",
                "Empty",
                vec![
                    attempt("none", "empty", vec![]),
                    Op::Print(used("zero")),
                    Op::Print(count("none")),
                ],
            )],
        ),
        region(
            "exact",
            scalar_bytes,
            vec![
                batch(
                    "rows",
                    "exact",
                    "Scalar",
                    vec![
                        attempt("rows", "scalar", vec![c(7)]),
                        Op::Print(used("exact")),
                        Op::Scope(vec![
                            Op::Borrow {
                                name: "first".into(),
                                batch: "rows".into(),
                                index: c(0),
                                access: Access::Readonly,
                            },
                            Op::Let {
                                name: "address".into(),
                                value: Expr::Address("first".into()),
                            },
                            attempt("rows", "scalar", vec![c(9)]),
                            Op::Print(count("rows")),
                            Op::Print(used("exact")),
                            Op::Print(read("first", &["value"])),
                            Op::Assert(equal(Expr::Address("first".into()), local("address"))),
                        ]),
                        Op::Reset {
                            batch: "rows".into(),
                        },
                        Op::Print(used("exact")),
                        attempt("rows", "scalar", vec![c(11)]),
                        Op::Scope(vec![
                            Op::Borrow {
                                name: "current".into(),
                                batch: "rows".into(),
                                index: c(0),
                                access: Access::Readonly,
                            },
                            Op::Print(read("current", &["value"])),
                        ]),
                        Op::Print(used("exact")),
                    ],
                ),
                Op::Print(used("exact")),
            ],
        ),
        region(
            "short",
            scalar_bytes - 1,
            vec![
                batch(
                    "failed",
                    "short",
                    "Scalar",
                    vec![
                        attempt("failed", "scalar", vec![c(7)]),
                        Op::Print(count("failed")),
                        Op::Print(used("short")),
                    ],
                ),
                Op::Print(used("short")),
                batch(
                    "reused",
                    "short",
                    "Empty",
                    vec![attempt("reused", "empty", vec![]), Op::Print(used("short"))],
                ),
                Op::Print(used("short")),
            ],
        ),
    ]);
    let mut expected = vec![1, 0, 0];
    expected.extend([
        0,
        scalar_bytes,
        1,
        1,
        scalar_bytes,
        7,
        0,
        0,
        11,
        scalar_bytes,
        0,
    ]);
    expected.extend([1, 0, 0, 0, 0, empty_bytes, 0]);
    (input, expected)
}

/// Construct failures whose first observable status must remain in field/call order.
fn ordered_fixture(layout: LayoutMode) -> (Program, Vec<u64>) {
    let (prefix, _, _, outer_bytes) = fixture_sizes(layout);
    let small = |name: &str, capacity, producer: &str| {
        region(
            name,
            capacity,
            vec![batch(
                "rows",
                name,
                if producer == "empty_reject" {
                    "Empty"
                } else {
                    "Tail"
                },
                vec![
                    attempt("rows", producer, vec![]),
                    Op::Print(used(name)),
                    Op::Print(count("rows")),
                ],
            )],
        )
    };
    let input = program(vec![
        // Even explicit rejection follows the record header/empty-unit reservation.
        small("prefix_first", 0, "empty_reject"),
        // A first scalar capacity failure precedes a later rejection or length overflow.
        small("capacity_before_reject", prefix + 7, "tail_reject"),
        small("capacity_before_overflow", prefix + 7, "tail_badlen"),
        // Once that scalar fits, rejection or field-size overflow wins before later capacity.
        small("reject_after_first", prefix + 8, "tail_reject"),
        small("overflow_after_first", prefix + 8, "tail_badlen"),
        region(
            "stable",
            256,
            vec![
                batch(
                    "outer_rows",
                    "stable",
                    "Outer",
                    vec![
                        attempt("outer_rows", "outer", vec![]),
                        Op::Print(used("stable")),
                        Op::Scope(vec![
                            Op::Borrow {
                                name: "first".into(),
                                batch: "outer_rows".into(),
                                index: c(0),
                                access: Access::Readonly,
                            },
                            Op::Let {
                                name: "address".into(),
                                value: Expr::Address("first".into()),
                            },
                            Op::Let {
                                name: "before".into(),
                                value: used("stable"),
                            },
                            attempt("outer_rows", "outer_nested_reject", vec![]),
                            Op::Assert(equal(used("stable"), local("before"))),
                            attempt("outer_rows", "outer_badlen", vec![]),
                            Op::Assert(equal(used("stable"), local("before"))),
                            Op::Assert(equal(Expr::Address("first".into()), local("address"))),
                            Op::Print(count("outer_rows")),
                            Op::Print(used("stable")),
                            Op::Print(read("first", &["tag"])),
                            Op::Print(read("first", &["child", "a"])),
                            Op::Print(read("first", &["child", "b"])),
                            Op::Print(Expr::BytesLen {
                                reference: "first".into(),
                                path: vec!["tail".into()],
                            }),
                        ]),
                        Op::Reset {
                            batch: "outer_rows".into(),
                        },
                        Op::Print(used("stable")),
                        attempt("outer_rows", "outer", vec![]),
                        Op::Print(used("stable")),
                    ],
                ),
                Op::Print(used("stable")),
            ],
        ),
    ]);
    let mut expected = vec![1, 0, 0, 1, 0, 0, 1, 0, 0, 3, 0, 0, 2, 0, 0];
    expected.extend([
        0,
        outer_bytes,
        3,
        2,
        1,
        outer_bytes,
        1,
        2,
        3,
        0,
        0,
        0,
        outer_bytes,
        0,
    ]);
    (input, expected)
}

#[test]
/// Boundary capacities retain a usable prefix and permit reset/backing reuse in every layout.
fn exact_fit_shortage_and_empty_capacity_preserve_publication() {
    compare("exact-fit", exact_fixture);
}

#[test]
/// Capacity, explicit rejection, and width/size overflow preserve their original precedence.
fn ordered_failures_preserve_status_and_nested_rollback() {
    compare("ordered-failures", ordered_fixture);
}

/// Compare a hand-derived fixture with independent semantics and nine native builds.
/// No native timing result is interpreted and every layout uses the same checker.
fn compare(label: &str, fixture: fn(LayoutMode) -> (Program, Vec<u64>)) {
    let tools = NativeToolchain::discover().expect("reservation tests require matched LLVM");
    let scratch = TempDir::new().unwrap();
    // Distinct physical formats must preserve each fixture's independently specified outcomes.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let (input, expected) = fixture(layout);
        let modeled = if layout == LayoutMode::Wide {
            model::run(&input)
        } else {
            model::run_with_layout(&input, layout)
        }
        .unwrap();
        assert_eq!(modeled, expected, "independent model {label}/{layout:?}");
        let checked = holo::check::check(&input).unwrap();
        let llvm = holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        let text = expected
            .iter()
            .map(|value| format!("{value}\n"))
            .collect::<String>();
        // O0, O3 and ASan exercise actual emitted guards without matching instruction strings.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch.path().join(format!("{layout:?}-{mode}"));
            let result = match mode {
                "asan" => tools.build_sanitized(&llvm, &binary),
                "o3" => tools.build(&llvm, &binary, true),
                _ => tools.build(&llvm, &binary, false),
            };
            let build = result.unwrap_or_else(|error| {
                fail(label, &input, &llvm, &tools, layout, mode, &error, &binary)
            });
            let output = Command::new(&binary)
                .env("ASAN_OPTIONS", "detect_leaks=0")
                .output()
                .unwrap();
            // A trap or incorrect state is a failed semantic check, never a discarded timing sample.
            if !output.status.success() || output.stdout != text.as_bytes() {
                fail(
                    label,
                    &input,
                    &llvm,
                    &tools,
                    layout,
                    mode,
                    &format!(
                        "commands: {:#?}\nexpected: {text:?}\nstatus: {}\nstdout: {}\nstderr: {}",
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

/// Preserve IR, LLVM, actual tools, and available binary before rejecting a discrepancy.
/// Evidence-I/O failure retains the original failure text in the resulting panic.
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
            .join(format!(
                "reservation-{label}-{}-{stamp}",
                std::process::id()
            ));
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
        // Failed compilation can leave no binary; preserve one only when it exists.
        if binary.is_file() {
            fs::copy(binary, directory.join("native"))?;
        }
        Ok(directory)
    };
    match save() {
        Ok(path) => panic!(
            "{label}/{layout:?}/{mode}: {detail}\nevidence: {}",
            path.display()
        ),
        Err(error) => panic!("{label}/{layout:?}/{mode}: {detail}\nevidence write failed: {error}"),
    }
}
