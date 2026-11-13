//! Generic IR workload for nested construction, retained references, and reuse.
//!
//! The workload is shared by the example and native tests, not by the independent
//! semantic oracle. Every operation exercises the real checker and LLVM backend.
//! One backing allocation serves all records; failures discard unpublished bytes.

use holo::ir::{
    Access, Binary, Build, Compare, Condition, Expr, Field, FieldKind, Op, Producer, Program,
    Record,
};
use holo::ir::{Argument, Parameter};

/// Construct the acceptance workload; host vectors allocate compiler input only.
/// Returns unchecked IR so callers must use the real checker before lowering.
pub fn program() -> Program {
    let records = vec![
        Record {
            name: "Payload".into(),
            fields: vec![Field {
                name: "data".into(),
                kind: FieldKind::Bytes,
            }],
        },
        Record {
            name: "Entry".into(),
            fields: vec![
                Field {
                    name: "id".into(),
                    kind: FieldKind::U64,
                },
                Field {
                    name: "payload".into(),
                    kind: FieldKind::Record("Payload".into()),
                },
            ],
        },
    ];
    let payload = Producer {
        name: "payload".into(),
        record: "Payload".into(),
        error: None,
        params: vec![Parameter::U64; 2],
        body: vec![Build::Bytes {
            field: "data".into(),
            len: Expr::Param(0),
            value: Expr::binary(Binary::AddWrapping, Expr::Param(1), Expr::Index),
        }],
    };
    let mut rejected_payload = payload.clone();
    rejected_payload.name = "rejected_payload".into();
    rejected_payload
        .body
        .push(Build::FailIf(eq(Expr::Const(1), Expr::Const(1))));
    let producers = vec![
        payload,
        rejected_payload,
        entry("entry", "payload"),
        entry("rejected_entry", "rejected_payload"),
    ];
    let mut references = vec![
        Op::Borrow {
            name: "first".into(),
            batch: "items".into(),
            index: Expr::Const(0),
            access: Access::Readwrite,
        },
        Op::Alias {
            name: "reader".into(),
            source: "first".into(),
            access: Access::Readonly,
        },
        Op::Let {
            name: "original_address".into(),
            value: Expr::Address("first".into()),
        },
        Op::Repeat {
            count: Expr::Const(8),
            index: "i".into(),
            body: vec![
                append(
                    "entry",
                    "grow_status",
                    vec![
                        Expr::binary(Binary::AddWrapping, Expr::Local("i".into()), Expr::Const(2)),
                        Expr::binary(Binary::AddWrapping, Expr::Local("i".into()), Expr::Const(1)),
                        Expr::Const(20),
                    ],
                ),
                assert_value(Expr::Local("grow_status".into()), 0),
            ],
        },
        Op::Write {
            reference: "first".into(),
            path: vec!["id".into()],
            value: Expr::Const(9),
        },
        Op::WriteByte {
            reference: "first".into(),
            path: vec!["payload".into(), "data".into()],
            index: Expr::Const(1),
            value: Expr::Const(77),
        },
        Op::Assert(eq(
            Expr::Address("first".into()),
            Expr::Local("original_address".into()),
        )),
        Op::Let {
            name: "published_bytes".into(),
            value: Expr::Used("memory".into()),
        },
        Op::Let {
            name: "published_count".into(),
            value: Expr::BatchLen("items".into()),
        },
    ];
    // Each failure occurs after construction has started and must preserve all
    // previously published data; distinct cases test capacity, rejection, overflow.
    for (callee, name, len, expected) in [
        ("entry", "full", 8192, 1),
        ("rejected_entry", "rejected", 7, 3),
        ("entry", "overflow", u64::MAX, 2),
    ] {
        references.extend([
            append(
                callee,
                name,
                vec![Expr::Const(99), Expr::Const(len), Expr::Const(0)],
            ),
            assert_value(Expr::Local(name.into()), expected),
            Op::Assert(eq(
                Expr::Used("memory".into()),
                Expr::Local("published_bytes".into()),
            )),
            Op::Assert(eq(
                Expr::BatchLen("items".into()),
                Expr::Local("published_count".into()),
            )),
            Op::Assert(eq(
                Expr::Address("first".into()),
                Expr::Local("original_address".into()),
            )),
        ]);
    }
    references.extend([
        Op::Let {
            name: "checksum".into(),
            value: Expr::Const(0),
        },
        Op::ForEach {
            batch: "items".into(),
            reference: "entry".into(),
            access: Access::Readonly,
            body: vec![
                Op::Set {
                    name: "checksum".into(),
                    value: Expr::binary(
                        Binary::AddWrapping,
                        Expr::Local("checksum".into()),
                        Expr::Read {
                            reference: "entry".into(),
                            path: vec!["id".into()],
                        },
                    ),
                },
                Op::Repeat {
                    count: Expr::BytesLen {
                        reference: "entry".into(),
                        path: vec!["payload".into(), "data".into()],
                    },
                    index: "byte".into(),
                    body: vec![Op::Set {
                        name: "checksum".into(),
                        value: Expr::binary(
                            Binary::AddWrapping,
                            Expr::Local("checksum".into()),
                            Expr::Byte {
                                reference: "entry".into(),
                                path: vec!["payload".into(), "data".into()],
                                index: Box::new(Expr::Local("byte".into())),
                            },
                        ),
                    }],
                },
            ],
        },
        Op::Print(Expr::Local("checksum".into())),
        Op::Print(Expr::Read {
            reference: "reader".into(),
            path: vec!["id".into()],
        }),
        Op::Print(Expr::Byte {
            reference: "reader".into(),
            path: vec!["payload".into(), "data".into()],
            index: Box::new(Expr::Const(1)),
        }),
        Op::Print(Expr::BatchLen("items".into())),
        Op::Print(Expr::Used("memory".into())),
    ]);
    let batch = Op::Batch {
        name: "items".into(),
        region: "memory".into(),
        record: "Entry".into(),
        body: vec![
            append(
                "entry",
                "first_status",
                vec![Expr::Const(1), Expr::Const(2), Expr::Const(10)],
            ),
            assert_value(Expr::Local("first_status".into()), 0),
            Op::Scope(references),
            Op::Reset {
                batch: "items".into(),
            },
            assert_value(Expr::Used("memory".into()), 0),
            Op::Print(Expr::Used("memory".into())),
            append(
                "entry",
                "reused",
                vec![Expr::Const(42), Expr::Const(0), Expr::Const(0)],
            ),
            assert_value(Expr::Local("reused".into()), 0),
            Op::Borrow {
                name: "again".into(),
                batch: "items".into(),
                index: Expr::Const(0),
                access: Access::Readonly,
            },
            Op::Print(Expr::Read {
                reference: "again".into(),
                path: vec!["id".into()],
            }),
        ],
    };
    Program {
        records,
        producers,
        body: vec![
            Op::Region {
                name: "memory".into(),
                capacity: Expr::Const(8192),
                fail_acquire: false,
                body: vec![
                    batch,
                    assert_value(Expr::Used("memory".into()), 0),
                    Op::Print(Expr::Used("memory".into())),
                ],
                on_failure: vec![assert_value(Expr::Const(0), 1)],
            },
            Op::Region {
                name: "unavailable".into(),
                capacity: Expr::Const(1),
                fail_acquire: true,
                body: vec![assert_value(Expr::Const(0), 1)],
                on_failure: vec![Op::Print(Expr::Const(1))],
            },
            Op::Region {
                name: "empty".into(),
                capacity: Expr::Const(0),
                fail_acquire: false,
                body: vec![Op::Batch {
                    name: "items".into(),
                    region: "empty".into(),
                    record: "Entry".into(),
                    body: vec![
                        append(
                            "entry",
                            "empty_status",
                            vec![Expr::Const(0), Expr::Const(0), Expr::Const(0)],
                        ),
                        Op::Print(Expr::Local("empty_status".into())),
                    ],
                }],
                on_failure: vec![assert_value(Expr::Const(0), 1)],
            },
        ],
    }
}

/// Define an outer producer delegating its nested payload to `callee`.
/// Parameters are id/length/seed; output is built entirely in the caller's slot.
fn entry(name: &str, callee: &str) -> Producer {
    Producer {
        name: name.into(),
        record: "Entry".into(),
        error: None,
        params: vec![Parameter::U64; 3],
        body: vec![
            Build::U64 {
                field: "id".into(),
                value: Expr::Param(0),
            },
            Build::Record {
                field: "payload".into(),
                producer: callee.into(),
                args: vec![Expr::Param(1), Expr::Param(2)]
                    .into_iter()
                    .map(Argument::Scalar)
                    .collect(),
            },
        ],
    }
}

/// Describe one append into the example batch; checking resolves schema/arity.
fn append(producer: &str, status: &str, args: Vec<Expr>) -> Op {
    Op::Append {
        batch: "items".into(),
        producer: producer.into(),
        args: args.into_iter().map(Argument::Scalar).collect(),
        status: status.into(),
    }
}

/// Construct an equality condition without a new language-level boolean syntax.
fn eq(left: Expr, right: Expr) -> Condition {
    Condition {
        op: Compare::Eq,
        left,
        right,
    }
}

/// Require one exact observed scalar; failure remains fatal in native release builds.
fn assert_value(value: Expr, expected: u64) -> Op {
    Op::Assert(eq(value, Expr::Const(expected)))
}
