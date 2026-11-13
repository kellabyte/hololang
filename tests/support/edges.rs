//! Explicit empty-layout and captured-prefix fixtures, independent of seeded recipes.
//!
//! These cases exercise zero capacity, acquisition failure, empty/zero-byte values,
//! same-root cursor rebinding, appends during traversal, and reset/reuse. They are
//! structured-IR correctness inputs, not new source-language syntax or benchmarks.

use holo::ir::{
    Access, Binary, Build, Compare, Condition, Expr, Field, FieldKind, Op, Producer, Program,
    Record,
};

/// Construct explicit empty/zero-capacity and captured-prefix traversal cases.
///
/// This fixture is deliberately structurally different from the seeded nested
/// records, reducing a generator blind spot around empty layouts and reset reuse.
pub fn program() -> Program {
    Program {
        records: vec![
            Record {
                name: "Empty".into(),
                fields: vec![],
            },
            Record {
                name: "ZeroBytes".into(),
                fields: vec![Field {
                    name: "data".into(),
                    kind: FieldKind::Bytes,
                }],
            },
        ],
        producers: vec![
            Producer {
                name: "empty".into(),
                record: "Empty".into(),
                error: None,
                params: vec![],
                body: vec![],
            },
            Producer {
                name: "zero".into(),
                record: "ZeroBytes".into(),
                error: None,
                params: vec![],
                body: vec![Build::Bytes {
                    field: "data".into(),
                    len: Expr::Const(0),
                    value: Expr::Index,
                }],
            },
        ],
        body: vec![
            Op::Region {
                name: "missing".into(),
                capacity: Expr::Const(64),
                fail_acquire: true,
                body: vec![Op::Print(Expr::Const(999))],
                on_failure: vec![Op::Print(Expr::Const(11))],
            },
            Op::Region {
                name: "zero_capacity".into(),
                capacity: Expr::Const(0),
                fail_acquire: false,
                body: vec![Op::Batch {
                    name: "none".into(),
                    region: "zero_capacity".into(),
                    record: "Empty".into(),
                    body: vec![
                        empty_append("none", "no_room"),
                        Op::Print(Expr::Local("no_room".into())),
                        Op::Print(Expr::Used("zero_capacity".into())),
                    ],
                }],
                on_failure: vec![Op::Print(Expr::Const(999))],
            },
            Op::Region {
                name: "arena".into(),
                capacity: Expr::Const(32),
                fail_acquire: false,
                body: vec![
                    Op::Batch {
                        name: "rows".into(),
                        region: "arena".into(),
                        record: "Empty".into(),
                        body: vec![
                            empty_append("rows", "a"),
                            empty_append("rows", "b"),
                            Op::Let {
                                name: "visits".into(),
                                value: Expr::Const(0),
                            },
                            Op::Scope(vec![
                                Op::Borrow {
                                    name: "second".into(),
                                    batch: "rows".into(),
                                    index: Expr::Const(1),
                                    access: Access::Readonly,
                                },
                                Op::ForEach {
                                    batch: "rows".into(),
                                    reference: "cursor".into(),
                                    access: Access::Readonly,
                                    body: vec![
                                        Op::AssignRef {
                                            name: "cursor".into(),
                                            source: "second".into(),
                                        },
                                        Op::Assert(Condition {
                                            op: Compare::Eq,
                                            left: Expr::Address("cursor".into()),
                                            right: Expr::Address("second".into()),
                                        }),
                                        empty_append("rows", "appended"),
                                        Op::Print(Expr::Local("appended".into())),
                                        Op::Set {
                                            name: "visits".into(),
                                            value: Expr::binary(
                                                Binary::AddWrapping,
                                                Expr::Local("visits".into()),
                                                Expr::Const(1),
                                            ),
                                        },
                                    ],
                                },
                            ]),
                            Op::Print(Expr::Local("visits".into())),
                            Op::Print(Expr::BatchLen("rows".into())),
                            Op::Print(Expr::Used("arena".into())),
                            Op::Reset {
                                batch: "rows".into(),
                            },
                            Op::Print(Expr::Used("arena".into())),
                        ],
                    },
                    Op::Batch {
                        name: "zero_payload".into(),
                        region: "arena".into(),
                        record: "ZeroBytes".into(),
                        body: vec![
                            Op::Append {
                                batch: "zero_payload".into(),
                                producer: "zero".into(),
                                args: vec![],
                                status: "ok".into(),
                            },
                            Op::Borrow {
                                name: "value".into(),
                                batch: "zero_payload".into(),
                                index: Expr::Const(0),
                                access: Access::Readonly,
                            },
                            Op::Print(Expr::BytesLen {
                                reference: "value".into(),
                                path: vec!["data".into()],
                            }),
                            Op::Print(Expr::Used("arena".into())),
                        ],
                    },
                ],
                on_failure: vec![Op::Print(Expr::Const(999))],
            },
        ],
    }
}

/// Request a generic empty-record append into a named batch.
fn empty_append(batch: &str, status: &str) -> Op {
    Op::Append {
        batch: batch.into(),
        producer: "empty".into(),
        args: vec![],
        status: status.into(),
    }
}
