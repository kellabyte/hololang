//! Reference-model regressions with independently hand-derived boundary results.
//!
//! These tests validate the oracle's observable expectations before using it for
//! native differential testing. They do not count as compiler conformance or
//! machine-code evidence by themselves.

#[path = "../support/generator.rs"]
mod generator;
#[path = "../support/model/mod.rs"]
mod model;

use holo::ir::{
    Access, Binary, Build, Compare, Condition, ErrorCase, ErrorFamily, Expr, Field, FieldKind,
    Function, FunctionArgument, FunctionParameter, FunctionParameterKind, Op, Producer, Program,
    Record, ResultArm, ResultPattern, ResultValue,
};

/// Nested range descriptors retain their base and extent through model calls.
#[test]
fn ordinary_byte_window_model_keeps_forwarded_offset() {
    let source = r#"
struct Row { bytes Bytes }

func row with noblock returns Row {
    return Row { bytes: core.bytes(5, core.index()) }
}

func first arg bytes Bytes as readonly reference returns u64 {
    return bytes[0]
}

func forward arg bytes Bytes as readonly reference returns u64 {
    return first(core.subrange(bytes, 1, 1))
}

func main returns u64 {
    let region = core.region(128) else { return 0 }
    let rows = core.batch(region, Row)
    let appended = core.append(rows, row())
    core.assert(appended == 0)
    let published = core.borrow_readonly(rows, 0)
    core.print(forward(core.subrange(core.prefix(published.bytes, 4), 1, 3)))
    return 0
}
"#;
    let program = holo::frontend::parse_experimental(source).unwrap();
    assert_eq!(model::run(program.program()).unwrap(), [2]);
    let invalid = source.replace("1, 3", "1, 4");
    let program = holo::frontend::parse_experimental(&invalid).unwrap();
    let error = model::run(program.program()).unwrap_err();
    assert!(error.message.contains("byte argument bounds trap"));
}

/// The model observes the original source bytes even when the ranges overlap.
#[test]
fn ordinary_byte_move_model_keeps_original_overlapping_source() {
    let source = r#"
struct Row { bytes Bytes }

func row with noblock returns Row {
    return Row { bytes: core.bytes(5, core.index()) }
}

func move_range
    arg destination Bytes as readwrite reference
    arg source Bytes as readonly reference
    returns u64
{
    core.move_bytes(destination, source)
    return core.len(source)
}

func main returns u64 {
    let region = core.region(128) else { return 0 }
    let rows = core.batch(region, Row)
    let appended = core.append(rows, row())
    core.assert(appended == 0)
    let published = core.borrow_readwrite(rows, 0)
    core.print(move_range(core.subrange(published.bytes, 1, 3), core.prefix(published.bytes, 3)))
    for index in core.range(5) {
        core.print(published.bytes[index])
    }
    return 0
}
"#;
    let program = holo::frontend::parse_experimental(source).unwrap();
    assert_eq!(model::run(program.program()).unwrap(), [3, 0, 0, 1, 2, 4]);
}

/// Capacity, size overflow, and nested rejection preserve the original record.
///
/// The 52-byte envelope is independently 8 + 8 + (8 + 8 + 8 + 4) + 8 bytes.
/// Its nested ID is 5 + 7 and its final payload byte is 12 + 3.
#[test]
fn independent_boundary_outcomes() {
    let actual = model::run(&generator::boundaries()).unwrap();
    assert_eq!(actual, vec![0, 1, 2, 3, 1, 52, 12, 15]);
}

/// A failed acquisition runs its separate branch without publishing an owner.
#[test]
fn failed_acquisition_has_no_partial_scope() {
    let program = Program {
        body: vec![Op::Region {
            name: "unavailable".into(),
            capacity: Expr::Const(64),
            fail_acquire: true,
            body: vec![Op::Print(Expr::Const(1))],
            on_failure: vec![Op::Print(Expr::Const(2))],
        }],
        ..Program::default()
    };
    assert_eq!(model::run(&program).unwrap(), vec![2]);
}

/// Seeded fixtures replay deterministically while varying meaningful outcomes.
#[test]
fn generated_inputs_replay_and_vary() {
    let mut outputs = Vec::new();
    // Different complete inputs must not collapse into one unexercised path;
    // exact equality is only required when replaying the same generated program.
    for seed in [0, 1, 7, 19, 41, 255, u64::MAX] {
        let program = generator::program(seed);
        let first = model::run(&program).unwrap();
        let replayed = model::run(&program).unwrap();
        assert_eq!(first, replayed, "seed {seed}");
        outputs.push(first);
    }
    assert!(outputs.windows(2).any(|pair| pair[0] != pair[1]));
}

/// A value oracle must refuse to fabricate nondeterministic clock measurements.
#[test]
fn clock_measurements_are_explicitly_outside_the_oracle() {
    let program = Program {
        body: vec![Op::Print(Expr::Nanoseconds)],
        ..Program::default()
    };
    let error = model::run(&program).unwrap_err();
    assert!(error.message.contains("nondeterministic measurement"));
}

/// Optimizer opacity preserves live region contents and the mathematical marker.
#[test]
fn opaque_region_has_no_semantic_storage_effect() {
    let program = Program {
        body: vec![Op::Region {
            name: "backing".into(),
            capacity: Expr::Const(32),
            fail_acquire: false,
            body: vec![
                Op::OpaqueRegion("backing".into()),
                Op::Print(Expr::Used("backing".into())),
            ],
            on_failure: vec![Op::Print(Expr::Const(99))],
        }],
        ..Program::default()
    };
    assert_eq!(model::run(&program).unwrap(), vec![0]);
}

/// The semantic oracle selects by nominal case and sees the whole unsigned success range.
#[test]
fn typed_results_preserve_the_success_value_and_choose_one_error_arm() {
    // Four distinct inputs exercise full-width success, named failure and fallback.
    let mut input = Program {
        body: vec![
            Op::ErrorFamily(ErrorFamily {
                name: "Decode".into(),
                children: vec![],
                cases: vec![
                    ErrorCase {
                        name: "Bad".into(),
                        payload: true,
                    },
                    ErrorCase {
                        name: "Short".into(),
                        payload: false,
                    },
                ],
            }),
            Op::Function(Function {
                returns_bool: false,
                name: "decode".into(),
                params: vec![FunctionParameter {
                    name: "input".into(),
                    kind: FunctionParameterKind::U64,
                }],
                result: Some("Decode".into()),
                body: vec![Op::If {
                    condition: Condition {
                        op: Compare::Lt,
                        left: Expr::Local("input".into()),
                        right: Expr::Const(2),
                    },
                    then_body: vec![Op::ReturnResult(ResultValue::Ok(Expr::binary(
                        Binary::SubWrapping,
                        Expr::Const(0),
                        Expr::Local("input".into()),
                    )))],
                    else_body: vec![Op::If {
                        condition: Condition {
                            op: Compare::Eq,
                            left: Expr::Local("input".into()),
                            right: Expr::Const(2),
                        },
                        then_body: vec![Op::ReturnResult(ResultValue::Error {
                            family: "Decode".into(),
                            case: "Bad".into(),
                            payload: Some(Expr::Const(10)),
                        })],
                        else_body: vec![Op::ReturnResult(ResultValue::Error {
                            family: "Decode".into(),
                            case: "Short".into(),
                            payload: None,
                        })],
                    }],
                }],
            }),
        ],
        ..Program::default()
    };
    // The named arm precedes the fallback and the success value has no sentinel.
    for value in 0..4 {
        input.body.push(Op::Print(Expr::Handle {
            function: "decode".into(),
            args: vec![FunctionArgument::Scalar(Expr::Const(value))],
            arms: vec![
                ResultArm {
                    pattern: ResultPattern::Ok(Some("value".into())),
                    value: Expr::Local("value".into()),
                },
                ResultArm {
                    pattern: ResultPattern::Error {
                        family: "Decode".into(),
                        case: "Bad".into(),
                        binding: Some("required".into()),
                    },
                    value: Expr::Local("required".into()),
                },
                ResultArm {
                    pattern: ResultPattern::Fallback(None),
                    value: Expr::Const(20),
                },
            ],
        }));
    }
    assert_eq!(model::run(&input).unwrap(), [0, u64::MAX, 10, 20]);
}

/// An earlier scalar argument captures its value before a later helper writes.
/// The separately captured state borrow observes the write in the final callee.
#[test]
fn ordinary_call_model_preserves_alias_visible_capture_order() {
    let state = FunctionParameter {
        name: "state".into(),
        kind: FunctionParameterKind::State("Counter".into()),
    };
    // All semantic state reads name the single fixed scalar field.
    let read = || Expr::Read {
        reference: "state".into(),
        path: vec!["value".into()],
    };
    let increment = Op::Function(Function {
        returns_bool: false,
        name: "increment".into(),
        params: vec![state.clone()],
        result: None,
        body: vec![
            Op::Let {
                name: "old".into(),
                value: read(),
            },
            Op::Write {
                reference: "state".into(),
                path: vec!["value".into()],
                value: Expr::binary(
                    Binary::AddWrapping,
                    Expr::Local("old".into()),
                    Expr::Const(1),
                ),
            },
            Op::Return(Expr::Local("old".into())),
        ],
    });
    let sum = Op::Function(Function {
        returns_bool: false,
        name: "sum".into(),
        result: None,
        params: vec![
            FunctionParameter {
                name: "before".into(),
                kind: FunctionParameterKind::U64,
            },
            state,
            FunctionParameter {
                name: "changed".into(),
                kind: FunctionParameterKind::U64,
            },
        ],
        body: vec![Op::Return(Expr::binary(
            Binary::AddWrapping,
            Expr::binary(
                Binary::AddWrapping,
                Expr::Local("before".into()),
                Expr::Local("changed".into()),
            ),
            read(),
        ))],
    });
    let input = Program {
        records: vec![Record {
            name: "Counter".into(),
            fields: vec![Field {
                name: "value".into(),
                kind: FieldKind::U64,
            }],
        }],
        producers: vec![Producer {
            name: "seed".into(),
            record: "Counter".into(),
            error: None,
            params: vec![],
            body: vec![Build::U64 {
                field: "value".into(),
                value: Expr::Const(7),
            }],
        }],
        body: vec![
            increment,
            sum,
            Op::Region {
                name: "region".into(),
                capacity: Expr::Const(64),
                fail_acquire: false,
                on_failure: vec![],
                body: vec![Op::Batch {
                    name: "records".into(),
                    region: "region".into(),
                    record: "Counter".into(),
                    body: vec![
                        Op::Append {
                            batch: "records".into(),
                            producer: "seed".into(),
                            args: vec![],
                            status: "seeded".into(),
                        },
                        Op::Borrow {
                            name: "state".into(),
                            batch: "records".into(),
                            index: Expr::Const(0),
                            access: Access::Readwrite,
                        },
                        Op::Print(Expr::Call {
                            function: "sum".into(),
                            args: vec![
                                FunctionArgument::Scalar(read()),
                                FunctionArgument::State("state".into()),
                                FunctionArgument::Scalar(Expr::Call {
                                    function: "increment".into(),
                                    args: vec![FunctionArgument::State("state".into())],
                                }),
                            ],
                        }),
                        Op::Print(read()),
                    ],
                }],
            },
        ],
    };
    // The two copies are seven; the state read sees the caller's updated eight.
    assert_eq!(model::run(&input).unwrap(), vec![22, 8]);
}

/// Application arguments observe caller values once, then repeated parameters
/// read captured scalars; the closed body cannot inspect the caller's locals.
#[test]
fn scalar_application_captures_values_and_isolates_its_body() {
    let mut program = Program {
        body: vec![
            Op::Let {
                name: "outside".into(),
                value: Expr::Const(7),
            },
            Op::Print(Expr::Apply {
                args: vec![Expr::Local("outside".into())],
                body: Box::new(Expr::binary(
                    Binary::AddWrapping,
                    Expr::Param(0),
                    Expr::Param(0),
                )),
            }),
        ],
        ..Program::default()
    };
    assert_eq!(model::run(&program).unwrap(), vec![14]);
    // A valid lexical local still cannot be observed from a closed application body.
    program.body[1] = Op::Print(Expr::Apply {
        args: vec![Expr::Const(3)],
        body: Box::new(Expr::Local("outside".into())),
    });
    let error = model::run(&program).unwrap_err();
    assert!(
        error
            .message
            .contains("scalar application body observes caller state")
    );
}

/// Unused actuals are evaluated and the first trapping actual wins in written order.
/// These malformed names exercise oracle ordering, not compiler conformance.
#[test]
fn scalar_application_evaluates_unused_arguments_left_to_right() {
    let program = Program {
        body: vec![Op::Print(Expr::Apply {
            args: vec![
                Expr::Local("first_missing".into()),
                Expr::Local("second_missing".into()),
            ],
            body: Box::new(Expr::Const(0)),
        })],
        ..Program::default()
    };
    let error = model::run(&program).unwrap_err();
    assert!(error.message.contains("first_missing"));
}
