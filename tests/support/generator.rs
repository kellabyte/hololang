//! Deterministic, bounded programs for independent model/native comparisons.
//!
//! Each seed varies capacities, payloads, field values, and attempted appends while
//! keeping native work small. These inputs exercise aliasing, nested destination
//! construction, partial failure, scanning, reset, and backing reuse. They are
//! correctness workloads, not performance measurements or exhaustive proofs.

use holo::ir::{
    Access, Binary, Build, Compare, Condition, Expr, Field, FieldKind, Op, Producer, Program,
    Record,
};
use holo::ir::{Argument, Parameter};

/// Build one small complete region program from a reproducible unsigned seed.
///
/// The returned input contains no host-dependent printed addresses. Native Assert
/// operations check same-run address identity before/after growth and reuse.
pub fn program(seed: u64) -> Program {
    let mut random = seed;
    let capacity = 128 + next(&mut random) % 5 * 64;
    let first_len = 3 + next(&mut random) % 13;
    let attempts = 4 + next(&mut random) % 9;
    let payload_salt = next(&mut random);
    let value_salt = next(&mut random);
    let (records, producers) = definitions();
    let mut scan = Vec::new();
    // Accumulate all named scalar fields and every byte, so a wrong nested field
    // offset or incomplete payload affects independent expected output.
    for path in [vec!["tag"], vec!["nested", "id"], vec!["tail"]] {
        scan.push(Op::Set {
            name: "sum".into(),
            value: add(local("sum"), read("item", &path)),
        });
    }
    scan.push(Op::Repeat {
        count: Expr::BytesLen {
            reference: "item".into(),
            path: path(&["nested", "payload"]),
        },
        index: "byte".into(),
        body: vec![Op::Set {
            name: "sum".into(),
            value: add(
                local("sum"),
                Expr::Byte {
                    reference: "item".into(),
                    path: path(&["nested", "payload"]),
                    index: Box::new(local("byte")),
                },
            ),
        }],
    });
    let retained_scope = vec![
        Op::Borrow {
            name: "first".into(),
            batch: "rows".into(),
            index: c(0),
            access: Access::Readwrite,
        },
        Op::Alias {
            name: "observer".into(),
            source: "first".into(),
            access: Access::Readonly,
        },
        Op::If {
            condition: eq(local("round"), c(0)),
            then_body: vec![Op::Set {
                name: "saved_address".into(),
                value: Expr::Address("first".into()),
            }],
            else_body: vec![Op::Assert(eq(
                local("saved_address"),
                Expr::Address("first".into()),
            ))],
        },
        Op::Let {
            name: "stable_address".into(),
            value: Expr::Address("first".into()),
        },
        Op::Repeat {
            count: c(attempts),
            index: "attempt".into(),
            body: vec![
                append(
                    "trial",
                    add(c(value_salt), local("attempt")),
                    Expr::binary(
                        Binary::And,
                        Expr::binary(Binary::Xor, local("attempt"), c(payload_salt)),
                        c(31),
                    ),
                    Expr::binary(Binary::And, local("attempt"), c(1)),
                ),
                Op::Print(local("trial")),
                Op::Assert(eq(local("stable_address"), Expr::Address("first".into()))),
            ],
        },
        Op::Write {
            reference: "first".into(),
            path: path(&["nested", "id"]),
            value: add(c(seed), local("round")),
        },
        Op::WriteByte {
            reference: "first".into(),
            path: path(&["nested", "payload"]),
            index: c(0),
            value: add(add(c(seed), local("round")), c(1)),
        },
        Op::Print(read("observer", &["nested", "id"])),
        Op::Print(Expr::Byte {
            reference: "observer".into(),
            path: path(&["nested", "payload"]),
            index: Box::new(c(0)),
        }),
        Op::Print(Expr::BytesLen {
            reference: "observer".into(),
            path: path(&["nested", "payload"]),
        }),
        Op::Print(Expr::BatchLen("rows".into())),
        Op::Print(Expr::Used("arena".into())),
        Op::Let {
            name: "sum".into(),
            value: c(0),
        },
        Op::ForEach {
            batch: "rows".into(),
            reference: "item".into(),
            access: Access::Readonly,
            body: scan,
        },
        Op::Print(local("sum")),
    ];
    Program {
        records,
        producers,
        body: vec![Op::Region {
            name: "arena".into(),
            capacity: c(capacity),
            fail_acquire: false,
            body: vec![
                Op::Let {
                    name: "saved_address".into(),
                    value: c(0),
                },
                Op::Repeat {
                    count: c(3),
                    index: "round".into(),
                    body: vec![
                        Op::Batch {
                            name: "rows".into(),
                            region: "arena".into(),
                            record: "Envelope".into(),
                            body: vec![
                                append("first_status", c(seed), c(first_len), c(0)),
                                Op::Assert(eq(local("first_status"), c(0))),
                                Op::Scope(retained_scope),
                                Op::Reset {
                                    batch: "rows".into(),
                                },
                                Op::Assert(eq(Expr::BatchLen("rows".into()), c(0))),
                                Op::Assert(eq(Expr::Used("arena".into()), c(0))),
                                append("reused_status", c(77), c(1), c(0)),
                                Op::Print(local("reused_status")),
                                Op::Print(Expr::Used("arena".into())),
                            ],
                        },
                        Op::Print(Expr::Used("arena".into())),
                    ],
                },
            ],
            on_failure: vec![Op::Print(c(999))],
        }],
    }
}

/// Build explicit capacity/overflow/rejection cases with exact small outcomes.
///
/// The program keeps one original record and verifies that each failed append
/// restores its marker/count and preserves an existing reference and payload.
pub fn boundaries() -> Program {
    let (records, producers) = definitions();
    Program {
        records,
        producers,
        body: vec![Op::Region {
            name: "arena".into(),
            capacity: c(128),
            fail_acquire: false,
            body: vec![Op::Batch {
                name: "rows".into(),
                region: "arena".into(),
                record: "Envelope".into(),
                body: vec![
                    append("initial", c(5), c(4), c(0)),
                    Op::Print(local("initial")),
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
                        append("capacity", c(6), c(200), c(0)),
                        Op::Print(local("capacity")),
                        append("overflow", c(6), c(u64::MAX), c(0)),
                        Op::Print(local("overflow")),
                        append("rejected", c(6), c(4), c(1)),
                        Op::Print(local("rejected")),
                        Op::Assert(eq(local("address"), Expr::Address("first".into()))),
                        Op::Print(Expr::BatchLen("rows".into())),
                        Op::Print(Expr::Used("arena".into())),
                        Op::Print(read("first", &["nested", "id"])),
                        Op::Print(Expr::Byte {
                            reference: "first".into(),
                            path: path(&["nested", "payload"]),
                            index: Box::new(c(3)),
                        }),
                    ]),
                ],
            }],
            on_failure: vec![Op::Print(c(999))],
        }],
    }
}

/// Supply two generic inline schemas and statically resolved nested producers.
///
/// Byte values exercise low-bit truncation and explicit wrapping arithmetic.
/// Rejection follows payload construction, so nested partial failure is exercised.
fn definitions() -> (Vec<Record>, Vec<Producer>) {
    let records = vec![
        Record {
            name: "Payload".into(),
            fields: vec![
                Field {
                    name: "id".into(),
                    kind: FieldKind::U64,
                },
                Field {
                    name: "payload".into(),
                    kind: FieldKind::Bytes,
                },
            ],
        },
        Record {
            name: "Envelope".into(),
            fields: vec![
                Field {
                    name: "tag".into(),
                    kind: FieldKind::U64,
                },
                Field {
                    name: "nested".into(),
                    kind: FieldKind::Record("Payload".into()),
                },
                Field {
                    name: "tail".into(),
                    kind: FieldKind::U64,
                },
            ],
        },
    ];
    let producers = vec![
        Producer {
            name: "payload".into(),
            record: "Payload".into(),
            error: None,
            params: vec![Parameter::U64; 3],
            body: vec![
                Build::U64 {
                    field: "id".into(),
                    value: Expr::Param(0),
                },
                Build::Bytes {
                    field: "payload".into(),
                    len: Expr::Param(1),
                    value: add(Expr::Param(0), Expr::Index),
                },
                Build::FailIf(eq(Expr::Param(2), c(1))),
            ],
        },
        Producer {
            name: "envelope".into(),
            record: "Envelope".into(),
            error: None,
            params: vec![Parameter::U64; 3],
            body: vec![
                Build::U64 {
                    field: "tag".into(),
                    value: Expr::binary(Binary::Xor, Expr::Param(0), c(0x1234)),
                },
                Build::Record {
                    field: "nested".into(),
                    producer: "payload".into(),
                    args: vec![add(Expr::Param(0), c(7)), Expr::Param(1), Expr::Param(2)]
                        .into_iter()
                        .map(Argument::Scalar)
                        .collect(),
                },
                Build::U64 {
                    field: "tail".into(),
                    value: Expr::binary(Binary::MulWrapping, Expr::Param(0), c(31)),
                },
            ],
        },
    ];
    (records, producers)
}

/// Describe one append attempt without deciding its expected failure outcome.
fn append(status: &str, id: Expr, len: Expr, rejected: Expr) -> Op {
    Op::Append {
        batch: "rows".into(),
        producer: "envelope".into(),
        args: vec![id, len, rejected]
            .into_iter()
            .map(Argument::Scalar)
            .collect(),
        status: status.into(),
    }
}

/// Construct a nested field path from small source-like names.
fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

/// Construct a scalar projection without asking compiler layout code for offsets.
fn read(reference: &str, parts: &[&str]) -> Expr {
    Expr::Read {
        reference: reference.into(),
        path: path(parts),
    }
}

/// Name an unsigned lexical input in the generated operation tree.
fn local(name: &str) -> Expr {
    Expr::Local(name.into())
}

/// Preserve an exact unsigned constant, including boundary values near u64::MAX.
fn c(value: u64) -> Expr {
    Expr::Const(value)
}

/// Build explicit wrapping addition, avoiding source overflow-default assumptions.
fn add(left: Expr, right: Expr) -> Expr {
    Expr::binary(Binary::AddWrapping, left, right)
}

/// Construct an equality assertion/branch condition without predicting its result.
fn eq(left: Expr, right: Expr) -> Condition {
    Condition {
        op: Compare::Eq,
        left,
        right,
    }
}

/// Advance a fixed LCG solely to vary bounded test inputs reproducibly.
///
/// The generator is not cryptographic. Complete generated IR is preserved on
/// failure because seeds alone do not identify future generator revisions.
fn next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}
