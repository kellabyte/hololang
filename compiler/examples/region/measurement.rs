//! Parameterized native lifecycle workload and its independent value oracle.
//!
//! The compiler receives ordinary schemas/producers and structured operations.
//! Timed work constructs inline records, materializes backing once per cycle,
//! scans with a linear cursor, and restores the batch marker. Host expected-value
//! calculation uses ordinary Rust integers rather than checker/backend routines.

use holo::ir::Argument;
use holo::ir::{Access, Binary, Compare, Condition, Expr, Op, Program};

use super::program;

/// Construct a measurement program for `count` records repeated `rounds` times.
///
/// The caller bounds both inputs before calling. The returned unchecked IR owns
/// compiler-side vectors; native backing acquisition/free stay outside the clock
/// window. Failure statuses accumulate rather than adding per-record assertions.
pub fn workload(count: u64, rounds: u64) -> Program {
    let mut input = program::program();
    let id = Expr::Local("record_index".into());
    let length = Expr::binary(Binary::And, id.clone(), Expr::Const(31));
    let scan = Op::ForEach {
        batch: "items".into(),
        reference: "entry".into(),
        access: Access::Readonly,
        body: vec![
            accumulate(
                "checksum",
                Expr::Read {
                    reference: "entry".into(),
                    path: vec!["id".into()],
                },
            ),
            Op::Repeat {
                count: Expr::BytesLen {
                    reference: "entry".into(),
                    path: vec!["payload".into(), "data".into()],
                },
                index: "byte_index".into(),
                body: vec![accumulate(
                    "checksum",
                    Expr::Byte {
                        reference: "entry".into(),
                        path: vec!["payload".into(), "data".into()],
                        index: Box::new(Expr::Local("byte_index".into())),
                    },
                )],
            },
        ],
    };
    input.body = vec![Op::Region {
        name: "memory".into(),
        // Each record occupies at most 32+31 bytes; this explicit bound has
        // non-geometric slack that the report discloses separately from payload.
        capacity: Expr::Const(count * 64),
        fail_acquire: false,
        body: vec![
            Op::Let {
                name: "checksum".into(),
                value: Expr::Const(0),
            },
            Op::Let {
                name: "failure_sum".into(),
                value: Expr::Const(0),
            },
            Op::OpaqueRegion("memory".into()),
            Op::Let {
                name: "started".into(),
                value: Expr::Nanoseconds,
            },
            Op::Repeat {
                count: Expr::Const(rounds),
                index: "round".into(),
                body: vec![Op::Batch {
                    name: "items".into(),
                    region: "memory".into(),
                    record: "Entry".into(),
                    body: vec![
                        Op::Repeat {
                            count: Expr::Const(count),
                            index: "record_index".into(),
                            body: vec![
                                Op::Append {
                                    batch: "items".into(),
                                    producer: "entry".into(),
                                    args: vec![id.clone(), length, id]
                                        .into_iter()
                                        .map(Argument::Scalar)
                                        .collect(),
                                    status: "append_status".into(),
                                },
                                accumulate("failure_sum", Expr::Local("append_status".into())),
                            ],
                        },
                        Op::OpaqueRegion("memory".into()),
                        scan,
                    ],
                }],
            },
            Op::Let {
                name: "elapsed".into(),
                value: Expr::binary(
                    Binary::SubWrapping,
                    Expr::Nanoseconds,
                    Expr::Local("started".into()),
                ),
            },
            // Validation and printing occur after the end timestamp. These
            // assertions remain required checks, independent of report tooling.
            equal(
                Expr::Local("checksum".into()),
                expected_checksum(count, rounds),
            ),
            equal(Expr::Local("failure_sum".into()), 0),
            equal(Expr::Used("memory".into()), 0),
            Op::Print(Expr::Local("elapsed".into())),
            Op::Print(Expr::Local("checksum".into())),
            Op::Print(Expr::Local("failure_sum".into())),
            Op::Print(Expr::Used("memory".into())),
        ],
        on_failure: vec![equal(Expr::Const(0), 1)],
    }];
    input
}

/// Compute the expected aggregate from logical IDs and payload bytes.
///
/// `count` and `rounds` are bounded host integers. Returns modulo-2^64 checksum
/// using no layout sizes, native pointers, or implementation/checker algorithms.
pub fn expected_checksum(count: u64, rounds: u64) -> u64 {
    let mut one_round = 0u64;
    // Each logical record has id=i and bytes (i+j) modulo256 for j<i modulo32.
    for id in 0..count {
        one_round = one_round.wrapping_add(id);
        for byte in 0..(id & 31) {
            one_round = one_round.wrapping_add(id.wrapping_add(byte) & 255);
        }
    }
    one_round.wrapping_mul(rounds)
}

/// Report peak published bytes in the chosen initial record format.
///
/// This includes 32 metadata/scalar bytes per record plus its payload. It is a
/// representation calculation, not a resident/committed memory measurement.
pub fn published_bytes(count: u64) -> u64 {
    (0..count).map(|id| 32 + (id & 31)).sum()
}

/// Add a value to an existing accumulator using explicit wrapping IR arithmetic.
///
/// The operation mutates only the named scalar; it allocates host AST storage.
fn accumulate(name: &str, value: Expr) -> Op {
    Op::Set {
        name: name.into(),
        value: Expr::binary(Binary::AddWrapping, Expr::Local(name.into()), value),
    }
}

/// Require an exact scalar result after the native timing window.
///
/// The returned assertion terminates on mismatch and never converts bad work into
/// a successful timing sample. It introduces no source-language error syntax.
fn equal(value: Expr, expected: u64) -> Op {
    Op::Assert(Condition {
        op: Compare::Eq,
        left: value,
        right: Expr::Const(expected),
    })
}
