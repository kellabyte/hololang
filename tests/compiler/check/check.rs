//! Positive and negative obligations for the fresh structured-IR checker.
//!
//! These tests exercise definition, initialization, lexical lifetime, and access
//! rules before LLVM lowering. The placement cases also exercise elaboration of
//! one private scoped destination into that checked IR. They do not stand in for
//! native address/failure evidence. Fixtures use ordinary schemas and destination
//! producers. All vectors and strings allocated here belong to the host test
//! process.

use holo::check::check;
use holo::ir::{
    Access, AppendArm, AppendPattern, Build, Compare, Condition, ConstructArm, ErrorCase,
    ErrorChild, ErrorFamily, Expr, Field, FieldKind, Function, FunctionArgument, FunctionParameter,
    FunctionParameterKind, Op, Producer, Program, Record, ResultArm, ResultPattern, ResultValue,
};
use holo::ir::{Argument, Parameter};
use holo::placement::{PlacementError, ScopedPlacement, place_scoped_aggregate};

/// Construct a two-field record fixture around `body` in one region/batch scope.
///
/// The returned unchecked IR owns its compiler/test data and has no native effect.
fn program(body: Vec<Op>) -> Program {
    region_program(vec![Op::Batch {
        name: "items".into(),
        region: "memory".into(),
        record: "Item".into(),
        body,
    }])
}

/// Wrap operations in the same fixture region without assigning its tail to a batch.
/// The unchecked IR owns the schema, producer and lexical operations.
fn region_program(body: Vec<Op>) -> Program {
    Program {
        records: vec![Record {
            name: "Item".into(),
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
        }],
        producers: vec![Producer {
            name: "item".into(),
            record: "Item".into(),
            error: None,
            params: vec![Parameter::U64; 2],
            body: vec![
                Build::U64 {
                    field: "id".into(),
                    value: Expr::Param(0),
                },
                Build::Bytes {
                    field: "payload".into(),
                    len: Expr::Param(1),
                    value: Expr::Index,
                },
            ],
        }],
        body: vec![Op::Region {
            name: "memory".into(),
            capacity: Expr::Const(4096),
            fail_acquire: false,
            body,
            on_failure: vec![],
        }],
    }
}

/// Build one append with a distinct status name and initialized scalar arguments.
fn append(status: &str) -> Op {
    Op::Append {
        batch: "items".into(),
        producer: "item".into(),
        args: vec![Expr::Const(1), Expr::Const(3)]
            .into_iter()
            .map(Argument::Scalar)
            .collect(),
        status: status.into(),
    }
}

/// Borrow the first fixture element with the requested path permission.
fn borrow(name: &str, access: Access) -> Op {
    Op::Borrow {
        name: name.into(),
        batch: "items".into(),
        index: Expr::Const(0),
        access,
    }
}

/// Produce a scalar observation through a named reference's `id` field.
fn id(reference: &str) -> Expr {
    Expr::Read {
        reference: reference.into(),
        path: vec!["id".into()],
    }
}

/// Construct a pure equality condition for branch/assertion fixtures.
fn equal(left: Expr, right: Expr) -> Condition {
    Condition {
        op: Compare::Eq,
        left,
        right,
    }
}

/// Require rejection with a meaningful diagnostic substring, not merely a panic.
fn rejects(program: &Program, expected: &str) {
    let error = check(program).expect_err("invalid structured IR was accepted");
    assert!(
        error.message().contains(expected),
        "expected {expected:?}, got {error}"
    );
}

/// Static text accepts complete UTF-8 bytes only at the process entry boundary.
#[test]
fn text_output_rejects_interior_nul_and_helper_effects() {
    let valid = program(vec![Op::PrintText("Hello, 世界! 100%".into())]);
    check(&valid).expect("entry text output");

    // The C terminator must never truncate a checked literal.
    let truncated = program(vec![Op::PrintText("before\0after".into())]);
    rejects(&truncated, "cannot contain a NUL byte");

    // Even an unused helper cannot bypass the entry-only output contract.
    let mut helper = valid;
    helper.body.insert(
        0,
        Op::Function(Function {
            name: "write_text".into(),
            params: vec![],
            result: None,
            body: vec![
                Op::PrintText("helper output".into()),
                Op::Return(Expr::Const(0)),
            ],
        }),
    );
    rejects(
        &helper,
        "cannot acquire or invalidate storage owners or perform external effects",
    );
}

/// A whole result must come from one acyclic, exactly typed destination call.
/// Check the public IR API independently of any source lowering restriction.
#[test]
fn whole_result_forward_requires_one_exact_acyclic_call() {
    let mut valid = program(Vec::new());
    valid.producers.push(Producer {
        name: "wrapper".into(),
        record: "Item".into(),
        error: None,
        params: vec![Parameter::U64; 2],
        body: vec![Build::Forward {
            producer: "item".into(),
            args: vec![
                Argument::Scalar(Expr::Param(0)),
                Argument::Scalar(Expr::Param(1)),
            ],
        }],
    });
    check(&valid).expect("matching destination constructor");

    let mut guarded = valid.clone();
    guarded.producers[1]
        .body
        .insert(0, Build::FailIf(equal(Expr::Const(0), Expr::Const(1))));
    rejects(&guarded, "only producer operation");

    // Equal field layouts do not make different record declarations interchangeable.
    let mut other_record = valid.clone();
    let mut schema = other_record.records[0].clone();
    schema.name = "Other".into();
    other_record.records.push(schema);
    other_record.producers[1].record = "Other".into();
    rejects(&other_record, "exact record and error family");

    let mut other_family = valid.clone();
    other_family.body.insert(
        0,
        Op::ErrorFamily(ErrorFamily {
            name: "Failure".into(),
            cases: vec![ErrorCase {
                name: "Failed".into(),
                payload: false,
            }],
            children: Vec::new(),
        }),
    );
    other_family.producers[1].error = Some("Failure".into());
    rejects(&other_family, "exact record and error family");

    let mut cycle = valid;
    let Build::Forward { producer, .. } = &mut cycle.producers[1].body[0] else {
        unreachable!("fixture has one forwarding step");
    };
    *producer = "wrapper".into();
    rejects(&cycle, "producer call");
}

/// Retained writable/readonly aliases remain valid across suffix append.
#[test]
fn retained_aliases_allow_append_and_mutation() {
    let input = program(vec![
        append("first_status"),
        borrow("first", Access::Readwrite),
        Op::Alias {
            name: "reader".into(),
            source: "first".into(),
            access: Access::Readonly,
        },
        append("second_status"),
        Op::Write {
            reference: "first".into(),
            path: vec!["id".into()],
            value: Expr::Const(9),
        },
        Op::Assert(equal(id("reader"), Expr::Const(9))),
    ]);
    assert!(check(&input).is_ok());
}

/// A readonly capability cannot write scalar fields or payload bytes.
#[test]
fn readonly_writes_are_rejected() {
    let writes = [
        Op::Write {
            reference: "reader".into(),
            path: vec!["id".into()],
            value: Expr::Const(2),
        },
        Op::WriteByte {
            reference: "reader".into(),
            path: vec!["payload".into()],
            index: Expr::Const(0),
            value: Expr::Const(2),
        },
    ];
    // Each write form must independently respect the same access boundary.
    for write in writes {
        rejects(
            &program(vec![
                append("status"),
                borrow("reader", Access::Readonly),
                write,
            ]),
            "readonly",
        );
    }
}

/// Creating another name cannot upgrade a readonly reference's authority.
#[test]
fn readonly_alias_upgrade_is_rejected() {
    rejects(
        &program(vec![
            borrow("reader", Access::Readonly),
            Op::Alias {
                name: "writer".into(),
                source: "reader".into(),
                access: Access::Readwrite,
            },
        ]),
        "upgrade a readonly",
    );
}

/// Rebinding aliases of one root preserves the initial checker's sound subset.
#[test]
fn same_root_reference_assignment_is_allowed() {
    let input = program(vec![
        borrow("first", Access::Readwrite),
        borrow("second", Access::Readwrite),
        Op::AssignRef {
            name: "first".into(),
            source: "second".into(),
        },
    ]);
    assert!(check(&input).is_ok());
}

/// Reset cannot discard contents while any named alias remains usable.
#[test]
fn reset_rejects_a_live_reference() {
    rejects(
        &program(vec![
            borrow("first", Access::Readwrite),
            Op::Reset {
                batch: "items".into(),
            },
        ]),
        "reference is live",
    );
}

/// Ending the reference scope allows marker reuse through the surviving owner.
#[test]
fn reset_after_reference_scope_is_allowed() {
    let input = program(vec![
        append("status"),
        Op::Scope(vec![borrow("first", Access::Readonly)]),
        Op::Reset {
            batch: "items".into(),
        },
        append("again"),
    ]);
    assert!(check(&input).is_ok());
}

/// Branch and loop scopes must retain visibility of outer-root reference hazards.
#[test]
fn branches_and_loops_cannot_hide_outer_references_from_reset() {
    let nested = [
        Op::If {
            condition: equal(Expr::Const(0), Expr::Const(1)),
            then_body: vec![],
            else_body: vec![Op::Reset {
                batch: "items".into(),
            }],
        },
        Op::Repeat {
            count: Expr::Const(2),
            index: "iteration".into(),
            body: vec![Op::Reset {
                batch: "items".into(),
            }],
        },
    ];
    // Both control-flow forms must inspect lexical aliases outside their body.
    for operation in nested {
        rejects(
            &program(vec![borrow("outer", Access::Readonly), operation]),
            "reference is live",
        );
    }
}

/// A sequential cursor borrows the existing prefix without preventing safe append.
#[test]
fn sequential_scan_allows_suffix_append() {
    let input = program(vec![
        append("initial"),
        Op::ForEach {
            batch: "items".into(),
            reference: "cursor".into(),
            access: Access::Readonly,
            body: vec![Op::Append {
                batch: "items".into(),
                producer: "item".into(),
                args: vec![
                    id("cursor"),
                    Expr::BytesLen {
                        reference: "cursor".into(),
                        path: vec!["payload".into()],
                    },
                ]
                .into_iter()
                .map(Argument::Scalar)
                .collect(),
                status: "added".into(),
            }],
        },
    ]);
    assert!(check(&input).is_ok());
}

/// A live sequential cursor blocks reset and enforces its requested access mode.
#[test]
fn sequential_scan_rejects_reset_and_readonly_write() {
    let invalid = [
        Op::Reset {
            batch: "items".into(),
        },
        Op::Write {
            reference: "cursor".into(),
            path: vec!["id".into()],
            value: Expr::Const(9),
        },
    ];
    // Each body separately checks a different cursor invariant before native lowering.
    for (operation, expected) in invalid.into_iter().zip(["reference is live", "readonly"]) {
        rejects(
            &program(vec![Op::ForEach {
                batch: "items".into(),
                reference: "cursor".into(),
                access: Access::Readonly,
                body: vec![operation],
            }]),
            expected,
        );
    }
}

/// An inner batch's reference cannot be stored in an older surviving binding.
#[test]
fn reference_assignment_cannot_escape_an_inner_root() {
    let input = program(vec![
        borrow("outer", Access::Readwrite),
        Op::Region {
            name: "inner_memory".into(),
            capacity: Expr::Const(128),
            fail_acquire: false,
            body: vec![Op::Batch {
                name: "inner".into(),
                region: "inner_memory".into(),
                record: "Item".into(),
                body: vec![
                    Op::Borrow {
                        name: "short".into(),
                        batch: "inner".into(),
                        index: Expr::Const(0),
                        access: Access::Readwrite,
                    },
                    Op::AssignRef {
                        name: "outer".into(),
                        source: "short".into(),
                    },
                ],
            }],
            on_failure: vec![],
        },
    ]);
    rejects(&input, "outlive");
}

/// Cross-root rebindings are an explicit first-checker limit, not lost origins.
#[test]
fn cross_root_assignment_is_explicitly_unsupported() {
    let input = program(vec![Op::Region {
        name: "other_memory".into(),
        capacity: Expr::Const(128),
        fail_acquire: false,
        body: vec![Op::Batch {
            name: "others".into(),
            region: "other_memory".into(),
            record: "Item".into(),
            body: vec![Op::Scope(vec![
                borrow("left", Access::Readwrite),
                Op::Borrow {
                    name: "right".into(),
                    batch: "others".into(),
                    index: Expr::Const(0),
                    access: Access::Readwrite,
                },
                Op::AssignRef {
                    name: "left".into(),
                    source: "right".into(),
                },
            ])],
        }],
        on_failure: vec![],
    }]);
    rejects(&input, "cross-root reference rebinding is unsupported");
}

/// A second live batch cannot acquire the first batch's construction tail.
#[test]
fn one_region_cannot_have_two_live_batches() {
    rejects(
        &program(vec![Op::Batch {
            name: "competing".into(),
            region: "memory".into(),
            record: "Item".into(),
            body: vec![],
        }]),
        "live batch owner",
    );
}

/// A reference local becomes inaccessible when its declaration scope ends.
#[test]
fn reference_use_after_scope_is_rejected() {
    rejects(
        &program(vec![
            Op::Scope(vec![borrow("short", Access::Readonly)]),
            Op::Print(id("short")),
        ]),
        "out-of-scope",
    );
}

/// Acquisition failure must not expose an uninitialized region owner.
#[test]
fn acquisition_failure_has_no_region_binding() {
    let mut input = program(vec![]);
    // Only the region operation's failure branch changes; ordinary success stays valid.
    if let Op::Region { on_failure, .. } = &mut input.body[0] {
        on_failure.push(Op::Print(Expr::Used("memory".into())));
    }
    rejects(&input, "out-of-scope");
}

/// The implicit counted-loop index is initialized and readonly in its body.
#[test]
fn loop_index_cannot_be_reassigned() {
    rejects(
        &program(vec![Op::Repeat {
            count: Expr::Const(2),
            index: "i".into(),
            body: vec![Op::Set {
                name: "i".into(),
                value: Expr::Const(99),
            }],
        }]),
        "writable scalar",
    );
}

/// Payloads and nested headers cannot be reinterpreted as scalar writable fields.
#[test]
fn shape_and_field_kinds_are_protected() {
    rejects(
        &program(vec![
            borrow("writer", Access::Readwrite),
            Op::Write {
                reference: "writer".into(),
                path: vec!["payload".into()],
                value: Expr::Const(9),
            },
        ]),
        "incompatible type",
    );
    rejects(
        &program(vec![
            borrow("writer", Access::Readwrite),
            Op::Write {
                reference: "writer".into(),
                path: vec!["payload".into(), "length".into()],
                value: Expr::Const(9),
            },
        ]),
        "not a nested record",
    );
}

/// Constructors cannot observe entry locals/published data through scalar expressions.
#[test]
fn constructor_memory_observations_are_rejected() {
    let mut input = program(vec![]);
    input.producers[0].body[0] = Build::U64 {
        field: "id".into(),
        value: id("published"),
    };
    rejects(&input, "constructor expressions cannot observe");
}

/// A possible successful result requires every field initialized exactly in order.
#[test]
fn incomplete_and_out_of_order_construction_is_rejected() {
    let mut input = program(vec![]);
    input.producers[0].body.pop();
    rejects(&input, "uninitialized");
    let mut input = program(vec![]);
    input.producers[0].body.swap(0, 1);
    rejects(&input, "in order");
}

/// The byte index is scoped to byte values, not payload lengths or scalar fields.
#[test]
fn constructor_inputs_have_precise_scope() {
    let mut input = program(vec![]);
    input.producers[0].body[0] = Build::U64 {
        field: "id".into(),
        value: Expr::Index,
    };
    rejects(&input, "index is unavailable");
    let mut input = program(vec![]);
    input.producers[0].body[0] = Build::U64 {
        field: "id".into(),
        value: Expr::Param(2),
    };
    rejects(&input, "parameter is unavailable");
}

/// Recursive inline data has no finite construction layout in this subset.
#[test]
fn recursive_schemas_are_rejected() {
    let input = Program {
        records: vec![Record {
            name: "Cycle".into(),
            fields: vec![Field {
                name: "next".into(),
                kind: FieldKind::Record("Cycle".into()),
            }],
        }],
        ..Program::default()
    };
    rejects(&input, "recursive inline record");
}

/// Append needs a known constructor with matching schema and scalar arity.
#[test]
fn unknown_or_wrong_arity_producers_are_rejected() {
    rejects(
        &program(vec![Op::Append {
            batch: "items".into(),
            producer: "missing".into(),
            args: vec![],
            status: "result".into(),
        }]),
        "unknown producer",
    );
    rejects(
        &program(vec![Op::Append {
            batch: "items".into(),
            producer: "item".into(),
            args: vec![],
            status: "result".into(),
        }]),
        "incompatible result or arguments",
    );
}

/// Nested producer calls initialize a schema field directly with checked arguments.
#[test]
fn nested_destination_production_is_allowed() {
    let mut input = program(vec![]);
    input.records.push(Record {
        name: "Envelope".into(),
        fields: vec![Field {
            name: "entry".into(),
            kind: FieldKind::Record("Item".into()),
        }],
    });
    input.producers.push(Producer {
        name: "envelope".into(),
        record: "Envelope".into(),
        error: None,
        params: vec![Parameter::U64; 2],
        body: vec![Build::Record {
            field: "entry".into(),
            producer: "item".into(),
            args: vec![Expr::Param(0), Expr::Param(1)]
                .into_iter()
                .map(Argument::Scalar)
                .collect(),
        }],
    });
    assert!(check(&input).is_ok());
}

/// Native timing and opaque observations are allowed only at checked harness boundaries.
#[test]
fn measurement_operations_require_live_execution_storage() {
    assert!(
        check(&program(vec![
            Op::Print(Expr::Nanoseconds),
            Op::OpaqueRegion("memory".into())
        ]))
        .is_ok()
    );
    rejects(
        &program(vec![Op::OpaqueRegion("items".into())]),
        "not a region",
    );
    let mut input = program(vec![]);
    input.producers[0].body[0] = Build::U64 {
        field: "id".into(),
        value: Expr::Nanoseconds,
    };
    rejects(&input, "constructor expressions cannot observe");
}

/// Declare an ordinary helper with explicit parameter bindings and structured body.
fn ordinary(name: &str, params: Vec<FunctionParameter>, body: Vec<Op>) -> Op {
    Op::Function(Function {
        name: name.into(),
        params,
        result: None,
        body,
    })
}

/// Construct a direct ordinary call expression with explicit captured actuals.
fn call(name: &str, args: Vec<FunctionArgument>) -> Expr {
    Expr::Call {
        function: name.into(),
        args,
    }
}

/// Forward ordinary calls can resolve statically, while cycles have no checked frame.
#[test]
fn ordinary_helpers_require_an_acyclic_closed_graph() {
    let mut input = Program {
        body: vec![
            ordinary("first", vec![], vec![Op::Return(call("second", vec![]))]),
            ordinary("second", vec![], vec![Op::Return(Expr::Const(1))]),
            Op::Print(call("first", vec![])),
        ],
        ..Program::default()
    };
    assert!(check(&input).is_ok());
    input.body[1] = ordinary("second", vec![], vec![Op::Return(call("first", vec![]))]);
    rejects(&input, "recursive ordinary call graph");
    input.body[1] = ordinary("second", vec![], vec![Op::Return(call("unknown", vec![]))]);
    rejects(&input, "unknown ordinary function unknown");
}

/// A raw IR result has the same nominal and exhaustive gate as parsed source.
#[test]
fn typed_result_ir_rejects_forged_cases_and_payloads() {
    let mut input = Program {
        body: vec![
            Op::ErrorFamily(ErrorFamily {
                name: "First".into(),
                children: vec![],
                cases: vec![
                    ErrorCase {
                        name: "Bad".into(),
                        payload: false,
                    },
                    ErrorCase {
                        name: "Full".into(),
                        payload: true,
                    },
                ],
            }),
            Op::ErrorFamily(ErrorFamily {
                name: "Other".into(),
                children: vec![],
                cases: vec![ErrorCase {
                    name: "Bad".into(),
                    payload: false,
                }],
            }),
            Op::Function(Function {
                name: "make".into(),
                params: vec![],
                result: Some("First".into()),
                body: vec![Op::ReturnResult(ResultValue::Ok(Expr::Const(u64::MAX)))],
            }),
            Op::Print(Expr::Handle {
                function: "make".into(),
                args: vec![],
                arms: vec![
                    ResultArm {
                        pattern: ResultPattern::Ok(Some("value".into())),
                        value: Expr::Local("value".into()),
                    },
                    ResultArm {
                        pattern: ResultPattern::Error {
                            family: "First".into(),
                            case: "Bad".into(),
                            binding: None,
                        },
                        value: Expr::Const(10),
                    },
                    ResultArm {
                        pattern: ResultPattern::Fallback(Some("reason".into())),
                        value: Expr::Const(20),
                    },
                ],
            }),
        ],
        ..Program::default()
    };
    assert!(check(&input).is_ok());
    // The fallback must never let its nominal binding appear as a scalar tag.
    let mut rejected = input.clone();
    let Op::Print(Expr::Handle { arms, .. }) = &mut rejected.body[3] else {
        unreachable!()
    };
    arms[2].value = Expr::Local("reason".into());
    rejects(&rejected, "not a scalar local");
    // A raw handler cannot read a tag-only case's inactive lane.
    let mut rejected = input.clone();
    let Op::Print(Expr::Handle { arms, .. }) = &mut rejected.body[3] else {
        unreachable!("test input has a result handler");
    };
    let ResultPattern::Error { binding, .. } = &mut arms[1].pattern else {
        unreachable!("test has a named error arm");
    };
    *binding = Some("forged".into());
    rejects(&rejected, "has no payload to bind");
    // A same-spelling case from another family cannot supply a native tag.
    let Op::Function(function) = &mut input.body[2] else {
        unreachable!()
    };
    function.body[0] = Op::ReturnResult(ResultValue::Error {
        family: "First".into(),
        case: "Full".into(),
        payload: None,
    });
    rejects(&input, "wrong payload shape");
    let Op::Function(function) = &mut input.body[2] else {
        unreachable!("test result producer is an ordinary function");
    };
    function.body[0] = Op::ReturnResult(ResultValue::Error {
        family: "Other".into(),
        case: "Bad".into(),
        payload: None,
    });
    rejects(&input, "expected error family First, found Other");
}

/// Build unchecked fixed-record append IR with one payload case and one tag-only
/// case. Two nominal families share the same spelling to exercise identity.
fn typed_append(arms: Vec<AppendArm>) -> Program {
    typed_fixed_fixture(program(vec![Op::AppendHandled {
        batch: "items".into(),
        producer: "item".into(),
        args: vec![
            Argument::Scalar(Expr::Const(1)),
            Argument::Scalar(Expr::Const(3)),
        ],
        result: "outcome".into(),
        arms,
    }]))
}

/// Use one typed fixed-record producer and two nominally distinct error families.
/// Change only the fixture declarations, leaving all caller operations intact.
fn typed_fixed_fixture(mut input: Program) -> Program {
    input.records[0].fields.pop();
    let producer = &mut input.producers[0];
    producer.error = Some("BuildError".into());
    producer.body.pop();
    producer.body.insert(
        0,
        Build::Reject {
            condition: equal(Expr::Param(0), Expr::Const(1)),
            case: "Bad".into(),
            payload: Some(Expr::Param(1)),
        },
    );
    // Both definitions follow their use to exercise the complete family table.
    for family in ["BuildError", "OtherError"] {
        input.body.push(Op::ErrorFamily(ErrorFamily {
            name: family.into(),
            children: vec![],
            cases: vec![
                ErrorCase {
                    name: "Bad".into(),
                    payload: true,
                },
                ErrorCase {
                    name: "Empty".into(),
                    payload: false,
                },
            ],
        }));
    }
    input
}

/// Make a payload-free arm or fallback for the fixed-record test fixture.
fn constant_arm(pattern: AppendPattern) -> AppendArm {
    AppendArm {
        pattern,
        value: Expr::Const(1),
    }
}

/// Supply the fixed-record fixture's checked actual shape and exhaustive arms.
/// The first arm may lend a readonly local. The fallback owns no reference.
fn scoped_local(region: &str, reference: Option<&str>, success: Vec<Op>, failure: Vec<Op>) -> Op {
    Op::ConstructHandled {
        region: region.into(),
        producer: "item".into(),
        args: vec![
            Argument::Scalar(Expr::Const(2)),
            Argument::Scalar(Expr::Const(3)),
        ],
        arms: vec![
            ConstructArm {
                pattern: AppendPattern::Ok,
                reference: reference.map(String::from),
                body: success,
            },
            ConstructArm {
                pattern: AppendPattern::Fallback(None),
                reference: None,
                body: failure,
            },
        ],
    }
}

/// Build the typed placement fixture with a prefix scalar and explicit region.
/// Move the supplied operations into the entry suffix and return its start.
/// The independent region ends before acquisition of the private backing.
fn placement_fixture(mut suffix: Vec<Op>) -> (Program, usize) {
    let mut input =
        typed_fixed_fixture(region_program(vec![Op::Print(Expr::Used("memory".into()))]));
    input.body.push(Op::Let {
        name: "placement_capacity".into(),
        value: Expr::Const(4096),
    });
    let acquire_at = input.body.len();
    input.body.append(&mut suffix);
    (input, acquire_at)
}

/// Own a request targeting the supplied lexical path and success operations.
/// Reuse the typed fixture's argument shape and exhaustive handler partition.
fn placement_request(acquire_at: usize, site: Vec<usize>, success: Vec<Op>) -> ScopedPlacement {
    let Op::ConstructHandled {
        producer,
        args,
        arms,
        ..
    } = scoped_local("memory", Some("local"), success, vec![])
    else {
        unreachable!("the fixed fixture always constructs a record");
    };
    ScopedPlacement {
        acquire_at,
        capacity: Expr::Local("placement_capacity".into()),
        fail_acquire: false,
        on_failure: vec![Op::Print(Expr::Local("placement_capacity".into()))],
        site,
        producer,
        args,
        arms,
    }
}

/// Make an explicitly rooted older record and a nested repeat scaffold.
/// The returned indices descend through the older region and batch, a scope,
/// and the repeat before inserting after its first operation. An optional
/// fallback name exercises expression-local binding during the initial append.
fn placement_with_live_root(fallback_binding: Option<&str>) -> (Vec<Op>, Vec<usize>) {
    let input = program(vec![
        Op::AppendHandled {
            batch: "items".into(),
            producer: "item".into(),
            args: vec![
                Argument::Scalar(Expr::Const(2)),
                Argument::Scalar(Expr::Const(3)),
            ],
            result: "status".into(),
            arms: vec![
                constant_arm(AppendPattern::Ok),
                constant_arm(AppendPattern::Fallback(fallback_binding.map(String::from))),
            ],
        },
        borrow("older", Access::Readonly),
        Op::Scope(vec![Op::Repeat {
            count: Expr::Const(2),
            index: "iteration".into(),
            body: vec![Op::Let {
                name: "actual".into(),
                value: Expr::Local("iteration".into()),
            }],
        }]),
        Op::Print(id("older")),
    ]);
    (input.body, vec![0, 0, 2, 0, 1])
}

/// An admitted nested site sees its lexical actuals and keeps the older root.
/// Independently verify that the elaboration produces ordinary checked IR.
#[test]
fn scoped_placement_preserves_prefix_and_nested_bindings() {
    let (suffix, site) = placement_with_live_root(None);
    let (input, acquire_at) = placement_fixture(suffix);
    let mut request = placement_request(
        acquire_at,
        site,
        vec![Op::Print(id("local")), Op::Print(id("older"))],
    );
    request.args[0] = Argument::Scalar(Expr::Local("actual".into()));
    let candidate = place_scoped_aggregate(input, request).expect("nested placement");
    // The setup and declarations stay outside the generated acquisition scope.
    assert_eq!(candidate.body.len(), acquire_at + 1);
    assert!(matches!(
        &candidate.body[0],
        Op::Region { name, .. } if name == "memory"
    ));
    assert!(matches!(&candidate.body[acquire_at], Op::Region { .. }));
    check(&candidate).expect("ordinary checker accepts the selected scope");
}

/// Check that placement rejects the requested condition through a diagnostic.
/// The helper consumes the owned request and fixture without native execution.
fn rejects_placement(input: Program, request: ScopedPlacement, expected: &str) {
    let error = place_scoped_aggregate(input, request).expect_err("invalid placement was accepted");
    // These tests reach checking rather than failing to select a site.
    let PlacementError::Check(error) = error else {
        panic!("expected checker rejection, got {error}");
    };
    assert!(
        error.message().contains(expected),
        "expected {expected:?}, got {error}"
    );
}

/// Invalid acquisition and insertion locations must fail before verification.
#[test]
fn scoped_placement_rejects_invalid_paths() {
    let (input, acquire_at) = placement_fixture(vec![Op::If {
        condition: equal(Expr::Const(1), Expr::Const(1)),
        then_body: vec![],
        else_body: vec![],
    }]);
    let invalid_owner = placement_request(input.body.len() + 1, vec![0], vec![]);
    assert!(matches!(
        place_scoped_aggregate(input.clone(), invalid_owner),
        Err(PlacementError::AcquisitionOutOfBounds)
    ));
    assert!(matches!(
        place_scoped_aggregate(input.clone(), placement_request(acquire_at, vec![], vec![]),),
        Err(PlacementError::EmptySite)
    ));
    assert!(matches!(
        place_scoped_aggregate(
            input.clone(),
            placement_request(acquire_at, vec![2], vec![]),
        ),
        Err(PlacementError::SiteOutOfBounds)
    ));
    // An If has no uniquely selected body in the current site path API.
    assert!(matches!(
        place_scoped_aggregate(input, placement_request(acquire_at, vec![0, 0], vec![])),
        Err(PlacementError::UnsupportedSiteScope)
    ));
}

/// Selecting backing must preserve the ordinary scope escape rule.
#[test]
fn scoped_placement_rejects_escaping_success_alias() {
    let (suffix, site) = placement_with_live_root(None);
    let (input, acquire_at) = placement_fixture(suffix);
    let request = placement_request(
        acquire_at,
        site,
        vec![
            Op::Alias {
                name: "alias".into(),
                source: "local".into(),
                access: Access::Readonly,
            },
            Op::AssignRef {
                name: "older".into(),
                source: "alias".into(),
            },
        ],
    );
    rejects_placement(input, request, "outlive");
}

/// User-owned operations cannot bind or observe the compiler's private name.
#[test]
fn scoped_placement_rejects_private_region_access() {
    let private = "$holo.scoped";
    let (input, acquire_at) = placement_fixture(vec![Op::Scope(vec![Op::Let {
        name: private.into(),
        value: Expr::Const(0),
    }])]);
    // The unrelated inner binding would be legal without namespace isolation.
    rejects_placement(
        input,
        placement_request(acquire_at, vec![1], vec![]),
        "private scoped placement region",
    );

    let (input, acquire_at) = placement_fixture(vec![Op::Region {
        name: "other".into(),
        capacity: Expr::Const(128),
        fail_acquire: false,
        body: vec![],
        on_failure: vec![Op::Print(Expr::Used(private.into()))],
    }]);
    // A nested failure branch otherwise has lexical access to the outer owner.
    rejects_placement(
        input,
        placement_request(acquire_at, vec![1], vec![]),
        "private scoped placement region",
    );

    let (input, acquire_at) = placement_fixture(vec![]);
    let mut request = placement_request(acquire_at, vec![0], vec![]);
    // Only the selected destination operand may resolve the private owner.
    request.args[0] = Argument::Scalar(Expr::Used(private.into()));
    rejects_placement(input, request, "private scoped placement region");

    let (input, acquire_at) = placement_fixture(vec![]);
    rejects_placement(
        input,
        placement_request(
            acquire_at,
            vec![0],
            vec![Op::Print(Expr::Used(private.into()))],
        ),
        "private scoped placement region",
    );

    let (suffix, site) = placement_with_live_root(Some(private));
    let (input, acquire_at) = placement_fixture(suffix);
    // Expression-local bindings also reserve the private name, even when the
    // selected arm never reads its fallback value.
    rejects_placement(
        input,
        placement_request(acquire_at, site, vec![]),
        "private scoped placement region",
    );
}

/// A free tail lends a readonly local and permits reuse after either arm.
#[test]
fn scoped_construct_ir_accepts_local_reads_and_region_reuse() {
    let mut attempt = scoped_local(
        "memory",
        Some("local"),
        vec![
            Op::Alias {
                name: "reader".into(),
                source: "local".into(),
                access: Access::Readonly,
            },
            Op::Print(id("reader")),
        ],
        // The failure arm starts with its attempted tail already released.
        vec![scoped_local("memory", None, vec![], vec![])],
    );
    let Op::ConstructHandled { arms, .. } = &mut attempt else {
        unreachable!("fixture provides a scoped constructor");
    };
    arms.insert(
        1,
        ConstructArm {
            pattern: AppendPattern::Rejected {
                family: "BuildError".into(),
                case: "Bad".into(),
                binding: Some("code".into()),
            },
            reference: None,
            body: vec![Op::Print(Expr::Local("code".into()))],
        },
    );
    let input = typed_fixed_fixture(region_program(vec![
        attempt,
        scoped_local("memory", None, vec![], vec![]),
        Op::Batch {
            name: "items".into(),
            region: "memory".into(),
            record: "Item".into(),
            body: vec![],
        },
    ]));
    check(&input).expect("each complete arm releases its own scoped tail");
}

/// A record can be read only in the successful lexical scope.
#[test]
fn scoped_construct_ir_rejects_inactive_or_escaped_reference() {
    let mut forged = scoped_local("memory", Some("local"), vec![], vec![]);
    let Op::ConstructHandled { arms, .. } = &mut forged else {
        unreachable!("fixture provides a scoped constructor");
    };
    arms[1].reference = Some("forged".into());
    rejects(
        &typed_fixed_fixture(region_program(vec![forged])),
        "only a construct ok arm may bind the record",
    );
    // The success name is absent both from the failure arm and after the handler.
    let read = Op::Print(id("local"));
    for body in [
        vec![scoped_local(
            "memory",
            Some("local"),
            vec![],
            vec![read.clone()],
        )],
        vec![scoped_local("memory", Some("local"), vec![], vec![]), read],
    ] {
        rejects(
            &typed_fixed_fixture(region_program(body)),
            "out-of-scope binding local",
        );
    }
    let escaped = typed_fixed_fixture(program(vec![
        borrow("outer", Access::Readonly),
        Op::Region {
            name: "scratch".into(),
            capacity: Expr::Const(4096),
            fail_acquire: false,
            body: vec![scoped_local(
                "scratch",
                Some("local"),
                vec![
                    Op::Alias {
                        name: "alias".into(),
                        source: "local".into(),
                        access: Access::Readonly,
                    },
                    Op::AssignRef {
                        name: "outer".into(),
                        source: "alias".into(),
                    },
                ],
                vec![],
            )],
            on_failure: vec![],
        },
    ]));
    rejects(
        &escaped,
        "reference stored in outer would outlive its owner",
    );
    let writable = scoped_local(
        "memory",
        Some("local"),
        vec![Op::Write {
            reference: "local".into(),
            path: vec!["id".into()],
            value: Expr::Const(5),
        }],
        vec![],
    );
    rejects(
        &typed_fixed_fixture(region_program(vec![writable])),
        "reference local is readonly",
    );
    // Even a same-root readonly alias cannot replace the direct reference.
    let rebound = scoped_local(
        "memory",
        Some("local"),
        vec![
            Op::Alias {
                name: "alias".into(),
                source: "local".into(),
                access: Access::Readonly,
            },
            Op::AssignRef {
                name: "local".into(),
                source: "alias".into(),
            },
        ],
        vec![],
    );
    rejects(
        &typed_fixed_fixture(region_program(vec![rebound])),
        "cannot rebind a scoped record's success reference",
    );
}

/// The region identity stays occupied even when success discards its reference.
#[test]
fn scoped_construct_ir_rejects_competing_tail_owners() {
    let claimants = [
        scoped_local("memory", None, vec![], vec![]),
        Op::Batch {
            name: "items".into(),
            region: "memory".into(),
            record: "Item".into(),
            body: vec![],
        },
    ];
    // A success scope prevents both kinds of owner from reusing its tail.
    for claimant in claimants {
        let local = scoped_local("memory", None, vec![claimant], vec![]);
        rejects(
            &typed_fixed_fixture(region_program(vec![local])),
            "already has a live scoped record owner",
        );
    }
    // The same region cannot be borrowed from a batch owner by shadowing its name.
    let local = scoped_local("memory", None, vec![], vec![]);
    let body = Op::Scope(vec![
        Op::Let {
            name: "items".into(),
            value: Expr::Const(0),
        },
        local,
    ]);
    rejects(
        &typed_fixed_fixture(program(vec![body])),
        "already has a live batch owner",
    );
}

/// Only typed producers and complete block handlers reach scoped lowering.
#[test]
fn scoped_construct_ir_rejects_invalid_convention_and_fallback() {
    let mut local = scoped_local("memory", None, vec![], vec![]);
    rejects(
        &region_program(vec![local.clone()]),
        "construct outcome handler requires a typed producer",
    );
    let Op::ConstructHandled { arms, .. } = &mut local else {
        unreachable!("fixture provides a scoped constructor");
    };
    arms[1].pattern = AppendPattern::Fallback(Some("cause".into()));
    rejects(
        &typed_fixed_fixture(region_program(vec![local])),
        "construct error fallback cannot bind a value",
    );
    let mut local = scoped_local("memory", None, vec![], vec![]);
    let Op::ConstructHandled { arms, .. } = &mut local else {
        unreachable!("fixture provides a scoped constructor");
    };
    arms.remove(0);
    rejects(
        &typed_fixed_fixture(region_program(vec![local])),
        "missing construct ok arm",
    );
}

/// A typed append partitions success, storage failures and nominal rejections.
/// Malformed raw IR cannot reach lowering even when a branch is unreachable.
#[test]
fn typed_append_ir_checks_exhaustiveness_and_payload_scope() {
    let success = constant_arm(AppendPattern::Ok);
    let full = constant_arm(AppendPattern::Full);
    let overflow = constant_arm(AppendPattern::SizeOverflow);
    let fallback = constant_arm(AppendPattern::Fallback(Some("cause".into())));
    let rejection = AppendArm {
        pattern: AppendPattern::Rejected {
            family: "BuildError".into(),
            case: "Bad".into(),
            binding: Some("value".into()),
        },
        value: Expr::Local("value".into()),
    };
    let empty = constant_arm(AppendPattern::Rejected {
        family: "BuildError".into(),
        case: "Empty".into(),
        binding: None,
    });
    assert!(
        check(&typed_append(vec![
            success.clone(),
            rejection.clone(),
            fallback.clone()
        ]))
        .is_ok()
    );
    assert!(
        check(&typed_append(vec![
            success.clone(),
            full.clone(),
            overflow.clone(),
            rejection.clone(),
            empty
        ]))
        .is_ok()
    );
    rejects(
        &typed_append(vec![fallback.clone()]),
        "missing append ok arm",
    );
    rejects(
        &typed_append(vec![success.clone(), overflow.clone()]),
        "missing append Full arm",
    );
    rejects(
        &typed_append(vec![success.clone(), full.clone()]),
        "missing append SizeOverflow arm",
    );
    rejects(
        &typed_append(vec![
            success.clone(),
            full.clone(),
            overflow,
            rejection.clone(),
        ]),
        "missing append rejection arm BuildError.Empty",
    );
    rejects(
        &typed_append(vec![success.clone(), full.clone(), full, fallback.clone()]),
        "duplicate append Full arm",
    );
    rejects(
        &typed_append(vec![success.clone(), fallback.clone(), rejection.clone()]),
        "fallback must be the last arm",
    );
    let wrong_family = constant_arm(AppendPattern::Rejected {
        family: "OtherError".into(),
        case: "Bad".into(),
        binding: None,
    });
    rejects(
        &typed_append(vec![success.clone(), wrong_family, fallback.clone()]),
        "expected error family BuildError, found OtherError",
    );
    let inactive_payload = constant_arm(AppendPattern::Rejected {
        family: "BuildError".into(),
        case: "Empty".into(),
        binding: Some("value".into()),
    });
    rejects(
        &typed_append(vec![success.clone(), inactive_payload, fallback.clone()]),
        "has no payload to bind",
    );
    let mut leaked = fallback.clone();
    leaked.value = Expr::Local("value".into());
    rejects(
        &typed_append(vec![success.clone(), rejection, leaked]),
        "out-of-scope binding value",
    );
    let mut opaque = fallback;
    opaque.value = Expr::Local("cause".into());
    rejects(&typed_append(vec![success, opaque]), "not a scalar local");
}

/// The typed calling convention requires named prefixes and complete fields.
#[test]
fn typed_append_ir_checks_producer_conventions_and_shape() {
    let valid = typed_append(vec![
        constant_arm(AppendPattern::Ok),
        constant_arm(AppendPattern::Fallback(None)),
    ]);
    let mut wrong_payload = valid.clone();
    let Build::Reject { payload, .. } = &mut wrong_payload.producers[0].body[0] else {
        unreachable!("fixture starts with a named rejection");
    };
    *payload = None;
    rejects(&wrong_payload, "has incompatible payload");

    let mut late = valid.clone();
    late.producers[0].body.swap(0, 1);
    rejects(&late, "named rejection must precede producer fields");

    let mut numeric = valid.clone();
    let Op::Region { body, .. } = &mut numeric.body[0] else {
        unreachable!("fixture provides a region");
    };
    let Op::Batch { body, .. } = &mut body[0] else {
        unreachable!("fixture provides a batch");
    };
    body[0] = append("numeric");
    rejects(&numeric, "typed append requires an outcome handler");

    let mut dynamic = valid.clone();
    dynamic.records[0].fields.push(Field {
        name: "dynamic".into(),
        kind: FieldKind::Bytes,
    });
    rejects(&dynamic, "leaves fields uninitialized");
    // An ordered dynamic byte initializer uses the same destination checker.
    dynamic.producers[0].body.push(Build::Bytes {
        field: "dynamic".into(),
        len: Expr::Param(1),
        value: Expr::Index,
    });
    assert!(check(&dynamic).is_ok());

    let mut mixed = valid;
    mixed.records.push(Record {
        name: "Outer".into(),
        fields: vec![Field {
            name: "inner".into(),
            kind: FieldKind::Record("Item".into()),
        }],
    });
    mixed.producers.push(Producer {
        name: "outer".into(),
        record: "Outer".into(),
        error: Some("OtherError".into()),
        params: vec![Parameter::U64; 2],
        body: vec![Build::Record {
            field: "inner".into(),
            producer: "item".into(),
            args: vec![
                Argument::Scalar(Expr::Param(0)),
                Argument::Scalar(Expr::Param(1)),
            ],
        }],
    });
    rejects(&mixed, "must have the same error family");
    // An explicit child union permits the same nested destination call.
    let Op::ErrorFamily(outer_error) = &mut mixed.body[2] else {
        unreachable!("fixture ends with the outer error family");
    };
    outer_error.children.push(ErrorChild {
        alias: "Inner".into(),
        family: "BuildError".into(),
    });
    outer_error.cases.extend([
        ErrorCase {
            name: "Inner.Bad".into(),
            payload: true,
        },
        ErrorCase {
            name: "Inner.Empty".into(),
            payload: false,
        },
    ]);
    assert_eq!(outer_error.try_error_offset("BuildError"), Some(2));
    assert!(check(&mixed).is_ok());
    mixed.producers[1].error = Some("BuildError".into());
    assert!(check(&mixed).is_ok());
}

/// A checked union encodes exactly the declared child tags and payload lane.
#[test]
fn error_union_ir_requires_exact_acyclic_child_blocks() {
    let base = ErrorFamily {
        name: "Base".into(),
        children: vec![],
        cases: vec![
            ErrorCase {
                name: "Bad".into(),
                payload: true,
            },
            ErrorCase {
                name: "Missing".into(),
                payload: false,
            },
        ],
    };
    let parent = ErrorFamily {
        name: "Parent".into(),
        children: vec![ErrorChild {
            alias: "Read".into(),
            family: "Base".into(),
        }],
        cases: vec![
            ErrorCase {
                name: "Local".into(),
                payload: false,
            },
            ErrorCase {
                name: "Read.Bad".into(),
                payload: true,
            },
            ErrorCase {
                name: "Read.Missing".into(),
                payload: false,
            },
        ],
    };
    // Keep the parent first so this test also verifies name registration
    // before checking child edges. Every fixture lives only in the host test.
    let program = |parent: ErrorFamily, base: ErrorFamily| Program {
        body: vec![Op::ErrorFamily(parent), Op::ErrorFamily(base)],
        ..Program::default()
    };
    assert!(check(&program(parent.clone(), base.clone())).is_ok());

    let mut wrong_payload = parent.clone();
    wrong_payload.cases[1].payload = false;
    rejects(
        &program(wrong_payload, base.clone()),
        "contiguous block of exact leaves",
    );
    let mut wrong_order = parent.clone();
    wrong_order.cases.swap(0, 1);
    rejects(
        &program(wrong_order, base.clone()),
        "contiguous block of exact leaves",
    );
    let mut missing = parent.clone();
    missing.cases.truncate(1);
    rejects(
        &program(missing, base.clone()),
        "missing leaves for child Read",
    );
    let mut forged = parent.clone();
    forged.children.clear();
    rejects(
        &program(forged, base.clone()),
        "unexpected child leaf Read.Bad",
    );
    let mut collision = parent.clone();
    collision.cases[0].name = "Read".into();
    rejects(
        &program(collision, base.clone()),
        "direct case Read conflicts with a child alias",
    );
    let mut ambiguous = parent.clone();
    ambiguous.children.push(ErrorChild {
        alias: "Other".into(),
        family: "Base".into(),
    });
    rejects(
        &program(ambiguous, base.clone()),
        "duplicate child family Base",
    );
    let mut unknown = parent.clone();
    unknown.children[0].family = "Missing".into();
    rejects(
        &program(unknown, base.clone()),
        "unknown error family Missing",
    );

    // A cycle is invalid even before its fabricated child leaves are examined.
    let mut cycle = base;
    cycle.children.push(ErrorChild {
        alias: "Parent".into(),
        family: "Parent".into(),
    });
    rejects(&program(parent, cycle), "recursive error family graph");
}

/// The highest child tag fits in one byte and the next leaf is rejected.
#[test]
fn error_union_ir_keeps_the_full_tag_range() {
    let leaf = ErrorCase {
        name: "Bad".into(),
        payload: false,
    };
    let mut outer = ErrorFamily {
        name: "Outer".into(),
        children: vec![ErrorChild {
            alias: "Inner".into(),
            family: "InnerError".into(),
        }],
        cases: (1..u8::MAX)
            .map(|ordinal| ErrorCase {
                name: format!("Leaf{ordinal}"),
                payload: false,
            })
            .collect(),
    };
    // A child's first tag is one, so a block after 254 leaves reaches 255.
    outer.cases.push(ErrorCase {
        name: "Inner.Bad".into(),
        ..leaf.clone()
    });
    assert_eq!(outer.try_error_offset("InnerError"), Some(254));
    let mut program = Program {
        body: vec![
            Op::ErrorFamily(outer.clone()),
            Op::ErrorFamily(ErrorFamily {
                name: "InnerError".into(),
                children: vec![],
                cases: vec![leaf],
            }),
        ],
        ..Program::default()
    };
    assert!(check(&program).is_ok());
    outer.cases.push(ErrorCase {
        name: "TooMany".into(),
        payload: false,
    });
    program.body[0] = Op::ErrorFamily(outer);
    rejects(&program, "must declare between 1 and 255 cases");
}

/// A propagation call needs a typed frame with an immutable success binding.
/// Even a checked propagation does not terminate success.
#[test]
fn try_ir_requires_its_ordinary_frame_and_a_success_return() {
    let declaration = Op::TryLet {
        name: "value".into(),
        function: "decode".into(),
        args: vec![],
    };
    let program = Program {
        body: vec![
            Op::ErrorFamily(ErrorFamily {
                name: "First".into(),
                children: vec![],
                cases: vec![ErrorCase {
                    name: "Bad".into(),
                    payload: false,
                }],
            }),
            Op::ErrorFamily(ErrorFamily {
                name: "Other".into(),
                children: vec![],
                cases: vec![ErrorCase {
                    name: "Bad".into(),
                    payload: false,
                }],
            }),
            Op::Function(Function {
                name: "decode".into(),
                params: vec![],
                result: Some("First".into()),
                body: vec![Op::ReturnResult(ResultValue::Ok(Expr::Const(u64::MAX)))],
            }),
            Op::Function(Function {
                name: "relay".into(),
                params: vec![],
                result: Some("First".into()),
                body: vec![
                    declaration.clone(),
                    Op::ReturnResult(ResultValue::Ok(Expr::Local("value".into()))),
                ],
            }),
        ],
        ..Program::default()
    };
    assert!(check(&program).is_ok());
    // A raw entry operation must not forge a return from a nonexistent helper frame.
    let mut entry = program.clone();
    entry.body.push(declaration);
    rejects(&entry, "try requires a scalar typed-result function");
    // Same-spelling cases do not authorize forwarding another family's ordinal.
    let mut wrong_family = program.clone();
    let Op::Function(relay) = &mut wrong_family.body[3] else {
        unreachable!("test relay is a function");
    };
    relay.result = Some("Other".into());
    rejects(&wrong_family, "must have error family Other");
    // Explicit propagation can lift a declared child without permitting a
    // bare return to change the nominal type of its result.
    let Op::ErrorFamily(outer) = &mut wrong_family.body[1] else {
        unreachable!("test defines the outer family");
    };
    outer.children.push(ErrorChild {
        alias: "Child".into(),
        family: "First".into(),
    });
    outer.cases.push(ErrorCase {
        name: "Child.Bad".into(),
        payload: false,
    });
    assert!(check(&wrong_family).is_ok());
    let Op::Function(relay) = &mut wrong_family.body[3] else {
        unreachable!("test relay is a function");
    };
    relay.body = vec![Op::ReturnResult(ResultValue::Call {
        function: "decode".into(),
        args: vec![],
    })];
    rejects(
        &wrong_family,
        "forwarded result from decode must have error family Other",
    );
    // Neither mutation nor missing success completion is valid after the try.
    let mut incomplete = program.clone();
    let Op::Function(relay) = &mut incomplete.body[3] else {
        unreachable!("test relay is a function");
    };
    relay.body.pop();
    rejects(&incomplete, "return a typed result on every path");
    let mut mutable = program;
    let Op::Function(relay) = &mut mutable.body[3] else {
        unreachable!("test relay is a function");
    };
    relay.body.insert(
        1,
        Op::Set {
            name: "value".into(),
            value: Expr::Const(0),
        },
    );
    rejects(&mutable, "not a writable scalar local");
}

/// Ordinary helper lookup cannot introduce mutable call effects into pure frames.
#[test]
fn ordinary_calls_are_unavailable_in_constructors_and_apply_bodies() {
    let mut input = program(vec![]);
    input.body.insert(
        0,
        ordinary("value", vec![], vec![Op::Return(Expr::Const(1))]),
    );
    input.producers[0].body[0] = Build::U64 {
        field: "id".into(),
        value: call("value", vec![]),
    };
    rejects(&input, "ordinary calls require an execution context");

    input.producers[0].body[0] = Build::U64 {
        field: "id".into(),
        value: Expr::Param(0),
    };
    input.body.push(Op::Print(Expr::Apply {
        args: vec![],
        body: Box::new(call("value", vec![])),
    }));
    rejects(&input, "ordinary calls require an execution context");
}

/// A mutable state actual needs the right fixed schema and a writable live path.
#[test]
fn ordinary_state_rejects_dynamic_schema_and_readonly_authority() {
    let mut input = program(vec![
        append("status"),
        borrow("state", Access::Readonly),
        Op::Print(call(
            "read_state",
            vec![FunctionArgument::State("state".into())],
        )),
    ]);
    input.body.insert(
        0,
        ordinary(
            "read_state",
            vec![FunctionParameter {
                name: "state".into(),
                kind: FunctionParameterKind::State("Item".into()),
            }],
            vec![Op::Return(id("state"))],
        ),
    );
    rejects(&input, "must have only fixed scalar fields");

    // Removing the payload makes the schema fixed, without upgrading the actual.
    input.records[0].fields.pop();
    input.producers[0].body.pop();
    rejects(&input, "reference state is readonly");
}

/// A helper cannot use a borrowed descriptor as a scalar return or let a branch fall through.
#[test]
fn ordinary_returns_preserve_call_lifetimes_and_control_flow() {
    let mut input = Program {
        body: vec![ordinary(
            "leak",
            vec![FunctionParameter {
                name: "bytes".into(),
                kind: FunctionParameterKind::Bytes,
            }],
            vec![Op::Return(Expr::Local("bytes".into()))],
        )],
        ..Program::default()
    };
    rejects(&input, "bytes is not a scalar local");

    input.body[0] = ordinary(
        "conditional",
        vec![],
        vec![Op::If {
            condition: equal(Expr::Const(1), Expr::Const(1)),
            then_body: vec![Op::Return(Expr::Const(1))],
            else_body: vec![],
        }],
    );
    rejects(&input, "must return a scalar on every path");
}
