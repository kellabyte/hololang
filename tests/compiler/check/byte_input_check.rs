//! Independent malformed-IR checks for typed synchronous readonly byte inputs.
//!
//! These fixtures bypass the source adapter to require the checked core itself
//! to validate every formal/actual kind, live projection, and input observation.
//! Positive same-batch aliases and nested forwarding prevent a blanket rejection
//! from masquerading as safety. Host test values allocate compiler memory only;
//! this file makes no native ABI, payload-copy, or performance claim.

use holo::check::check;
use holo::ir::{
    Access, Argument, Build, BytesSource, Expr, Field, FieldKind, Op, Parameter, Producer, Program,
    Record, Refill,
};

/// Return a complete-field borrow through the existing source reference.
/// This is unchecked test input; the checker must establish the root and path.
fn source() -> Argument {
    Argument::Bytes(BytesSource::Field {
        reference: "source".into(),
        path: vec!["data".into()],
    })
}

/// Return one typed append without evaluating it or acquiring target memory.
/// Caller-owned actuals move into the new operation and each status is distinct.
fn append(batch: &str, producer: &str, args: Vec<Argument>, status: &str) -> Op {
    Op::Append {
        batch: batch.into(),
        producer: producer.into(),
        args,
        status: status.into(),
    }
}

/// Build valid same-batch and nested-forwarding work around one published source.
/// The copier's Bytes formal is index zero; its result owns fresh bytes. Input
/// vectors/strings belong to the host test and create no target runtime objects.
fn fixture() -> Program {
    Program {
        records: vec![
            Record {
                name: "Item".into(),
                fields: vec![
                    Field {
                        name: "id".into(),
                        kind: FieldKind::U64,
                    },
                    Field {
                        name: "data".into(),
                        kind: FieldKind::Bytes,
                    },
                ],
            },
            Record {
                name: "Envelope".into(),
                fields: vec![Field {
                    name: "item".into(),
                    kind: FieldKind::Record("Item".into()),
                }],
            },
        ],
        producers: vec![
            Producer {
                name: "seed".into(),
                record: "Item".into(),
                error: None,
                params: vec![Parameter::U64],
                body: vec![
                    Build::U64 {
                        field: "id".into(),
                        value: Expr::Const(7),
                    },
                    Build::Bytes {
                        field: "data".into(),
                        len: Expr::Param(0),
                        value: Expr::Index,
                    },
                ],
            },
            Producer {
                name: "copy".into(),
                record: "Item".into(),
                error: None,
                params: vec![Parameter::Bytes],
                body: vec![
                    Build::U64 {
                        field: "id".into(),
                        value: Expr::Const(9),
                    },
                    Build::Bytes {
                        field: "data".into(),
                        len: Expr::InputLen(0),
                        value: Expr::InputByte {
                            param: 0,
                            index: Box::new(Expr::Index),
                        },
                    },
                ],
            },
            Producer {
                name: "nested".into(),
                record: "Envelope".into(),
                error: None,
                params: vec![Parameter::Bytes],
                body: vec![Build::Record {
                    field: "item".into(),
                    producer: "copy".into(),
                    args: vec![Argument::Bytes(BytesSource::Param(0))],
                }],
            },
        ],
        body: vec![Op::Region {
            name: "memory".into(),
            capacity: Expr::Const(512),
            fail_acquire: false,
            on_failure: vec![],
            body: vec![Op::Batch {
                name: "items".into(),
                region: "memory".into(),
                record: "Item".into(),
                body: vec![
                    append(
                        "items",
                        "seed",
                        vec![Argument::Scalar(Expr::Const(4))],
                        "seeded",
                    ),
                    Op::Borrow {
                        name: "source".into(),
                        batch: "items".into(),
                        index: Expr::Const(0),
                        access: Access::Readwrite,
                    },
                    append("items", "copy", vec![source()], "copied"),
                    Op::Region {
                        name: "other_memory".into(),
                        capacity: Expr::Const(512),
                        fail_acquire: false,
                        on_failure: vec![],
                        body: vec![Op::Batch {
                            name: "envelopes".into(),
                            region: "other_memory".into(),
                            record: "Envelope".into(),
                            body: vec![append(
                                "envelopes",
                                "nested",
                                vec![source()],
                                "nested_status",
                            )],
                        }],
                    },
                ],
            }],
        }],
    }
}

/// Borrow the source batch's entry operations for focused malformed-input changes.
/// No production API is bypassed: each resulting program must pass check separately.
fn entry(input: &mut Program) -> &mut Vec<Op> {
    let Op::Region { body, .. } = &mut input.body[0] else {
        unreachable!()
    };
    let Op::Batch { body, .. } = &mut body[0] else {
        unreachable!()
    };
    body
}

/// Require rejection with its meaningful diagnostic, rather than accepting a panic.
/// Borrowed input remains unchanged; unexpected success fails this checker test.
fn rejects(input: &Program, message: &str) {
    let error = check(input).expect_err("malformed byte-input IR was accepted");
    assert!(
        error.message().contains(message),
        "expected {message:?}, got {error}"
    );
}

/// A writable source reference can lend readonly bytes during same-root append.
#[test]
fn published_same_batch_and_nested_borrows_are_valid() {
    check(&fixture()).expect("readonly input must permit aliasing and nested forwarding");
}

/// The data arm may lend its fresh descriptor directly to a same-batch append.
/// A forged name in a producer body, sibling arm, or later operation must fail.
#[test]
fn received_prefix_is_a_lexical_producer_actual() {
    let mut input = fixture();
    let copy_received = || {
        append(
            "items",
            "copy",
            vec![Argument::Received("chunk".into())],
            "copied",
        )
    };
    entry(&mut input)[2] = Op::Refill(Refill {
        destination: BytesSource::Field {
            reference: "source".into(),
            path: vec!["data".into()],
        },
        limit: Expr::Const(3),
        data_name: "chunk".into(),
        data_body: vec![copy_received()],
        result_name: "status".into(),
        other_body: vec![],
    });
    check(&input).expect("fresh readonly prefix can be copied into the same batch");

    let mut sibling = input.clone();
    // The nondata continuation sees only the copied native status.
    let Op::Refill(refill) = &mut entry(&mut sibling)[2] else {
        unreachable!("the fixture puts refill in the third entry operation");
    };
    refill.other_body.push(copy_received());
    rejects(&sibling, "chunk");

    let mut expired = input.clone();
    entry(&mut expired).push(copy_received());
    rejects(&expired, "chunk");

    let mut shadowed = input.clone();
    // A lexical shadow does not give a scalar the original view's authority.
    let Op::Refill(refill) = &mut entry(&mut shadowed)[2] else {
        unreachable!("the fixture puts refill in the third entry operation");
    };
    refill.data_body = vec![Op::Scope(vec![
        Op::Let {
            name: "chunk".into(),
            value: Expr::Const(3),
        },
        copy_received(),
    ])];
    rejects(&shadowed, "chunk is not a received view");

    // A nested producer has its own frame and must forward the Bytes formal.
    input.producers[2].body[0] = Build::Record {
        field: "item".into(),
        producer: "copy".into(),
        args: vec![Argument::Received("chunk".into())],
    };
    rejects(&input, "Nested producers must forward a Bytes parameter");
}

/// Formal/actual kinds and full arity are checked independently of source parsing.
#[test]
fn calls_reject_wrong_actual_kinds_and_arity() {
    // Vary only the copied call's actuals; the source root remains valid throughout.
    for args in [
        vec![],
        vec![Argument::Scalar(Expr::Const(4))],
        vec![source(), source()],
    ] {
        let mut input = fixture();
        entry(&mut input)[2] = append("items", "copy", args, "copied");
        assert!(check(&input).is_err(), "incorrect typed call was accepted");
    }
    let mut input = fixture();
    entry(&mut input)[2] = append("items", "seed", vec![source()], "copied");
    rejects(&input, "incompatible type");
}

/// A byte formal can never supply a numeric address or an out-of-range parameter.
#[test]
fn input_observations_require_the_correct_formal_kind() {
    // Each invalid initializer preserves the surrounding complete schema and valid actuals.
    for value in [
        Expr::Param(0),
        Expr::Param(usize::MAX),
        Expr::InputLen(usize::MAX),
    ] {
        let mut input = fixture();
        input.producers[1].body[0] = Build::U64 {
            field: "id".into(),
            value,
        };
        assert!(
            check(&input).is_err(),
            "invalid formal observation was accepted"
        );
    }
    let mut input = fixture();
    input.producers[0].body[0] = Build::U64 {
        field: "id".into(),
        value: Expr::InputLen(0),
    };
    rejects(&input, "incompatible parameter type");
}

/// Byte-read index children obey scalar typing and initializer-index scope.
#[test]
fn input_byte_indexes_are_checked_recursively() {
    // An index formal is Bytes, and core Index is unavailable in a scalar field.
    for index in [Expr::Param(0), Expr::Index] {
        let mut input = fixture();
        input.producers[1].body[0] = Build::U64 {
            field: "id".into(),
            value: Expr::InputByte {
                param: 0,
                index: Box::new(index),
            },
        };
        assert!(
            check(&input).is_err(),
            "invalid input-byte index was accepted"
        );
    }
}

/// A producer can forward existing inputs but cannot name caller-local references.
#[test]
fn descriptor_sources_cannot_cross_execution_and_producer_contexts() {
    let mut input = fixture();
    let Build::Record { args, .. } = &mut input.producers[2].body[0] else {
        unreachable!()
    };
    *args = vec![source()];
    rejects(
        &input,
        "byte argument requires a live published field or forwarded Bytes parameter",
    );
    let mut input = fixture();
    entry(&mut input)[2] = append(
        "items",
        "copy",
        vec![Argument::Bytes(BytesSource::Param(0))],
        "copied",
    );
    rejects(
        &input,
        "byte argument requires a live published field or forwarded Bytes parameter",
    );
    let mut input = fixture();
    let Build::Record { args, .. } = &mut input.producers[2].body[0] else {
        unreachable!()
    };
    *args = vec![Argument::Bytes(BytesSource::Param(usize::MAX))];
    rejects(
        &input,
        "byte argument requires a live published field or forwarded Bytes parameter",
    );
}

/// Entry projections require a live record root and a complete Bytes terminal field.
#[test]
fn input_fields_require_initialized_live_byte_projections() {
    // Missing/empty/nonbyte paths must reject before any LLVM field walk can occur.
    for path in [vec![], vec!["id".into()], vec!["missing".into()]] {
        let mut input = fixture();
        entry(&mut input)[2] = append(
            "items",
            "copy",
            vec![Argument::Bytes(BytesSource::Field {
                reference: "source".into(),
                path,
            })],
            "copied",
        );
        assert!(
            check(&input).is_err(),
            "invalid byte projection was accepted"
        );
    }
    let mut input = fixture();
    let borrowed = entry(&mut input).remove(1);
    entry(&mut input).insert(1, Op::Scope(vec![borrowed]));
    rejects(&input, "out-of-scope binding source");
    let mut input = fixture();
    entry(&mut input)[2] = append(
        "items",
        "copy",
        vec![Argument::Bytes(BytesSource::Field {
            reference: "items".into(),
            path: vec!["data".into()],
        })],
        "copied",
    );
    rejects(&input, "not a reference");
}

/// Input observations cannot materialize an unbound producer formal in entry code.
#[test]
fn entry_scalar_operations_cannot_observe_producer_inputs() {
    // Neither extent nor element access has a producer frame in execution context.
    for value in [
        Expr::InputLen(0),
        Expr::InputByte {
            param: 0,
            index: Box::new(Expr::Const(0)),
        },
    ] {
        let mut input = fixture();
        entry(&mut input).push(Op::Print(value));
        rejects(&input, "readonly byte input is unavailable");
    }
}

/// Helper actuals may read outer bytes, but the scalar body sees only captured values.
#[test]
fn eager_scalar_application_captures_inputs_in_the_enclosing_frame() {
    let mut input = fixture();
    input.producers[1].body[0] = Build::U64 {
        field: "id".into(),
        value: Expr::Apply {
            args: vec![Expr::InputByte {
                param: 0,
                index: Box::new(Expr::Const(0)),
            }],
            body: Box::new(Expr::Apply {
                args: vec![Expr::Param(0)],
                body: Box::new(Expr::Param(0)),
            }),
        },
    };
    check(&input).expect("nested scalar helper frames can capture the enclosing scalar actual");
}

/// Scalar application cannot smuggle outer byte/index authority into its body.
#[test]
fn scalar_application_body_has_a_separate_checked_frame() {
    // Fresh bodies have one scalar formal, no byte formal, no surrounding initializer Index.
    for body in [
        Expr::InputLen(0),
        Expr::InputByte {
            param: 0,
            index: Box::new(Expr::Const(0)),
        },
        Expr::Index,
        Expr::Param(1),
    ] {
        let mut input = fixture();
        input.producers[1].body[1] = Build::Bytes {
            field: "data".into(),
            len: Expr::InputLen(0),
            value: Expr::Apply {
                args: vec![Expr::Index],
                body: Box::new(body),
            },
        };
        assert!(
            check(&input).is_err(),
            "helper body inherited unavailable outer authority"
        );
    }
    let mut input = fixture();
    input.producers[1].body[0] = Build::U64 {
        field: "id".into(),
        value: Expr::Apply {
            args: vec![Expr::Param(0)],
            body: Box::new(Expr::Const(0)),
        },
    };
    rejects(&input, "constructor parameter is unavailable");
}
