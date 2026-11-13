//! Stable operation locations for source diagnostics without modifying checked IR.
//!
//! These structural fixtures check every child traversal category and propagation
//! through enclosing operations. They do not execute native code or imply that
//! definition failures have source-operation positions.

use holo::check::check;
use holo::ir::{Access, Compare, Condition, Expr, Op, Program, Record};

/// Return an otherwise valid input with an empty schema and the supplied entry.
fn program(body: Vec<Op>) -> Program {
    Program {
        records: vec![Record {
            name: "Empty".into(),
            fields: vec![],
        }],
        producers: vec![],
        body,
    }
}

/// Return a constant condition so traversal cannot rely on runtime branch choice.
fn condition() -> Condition {
    Condition {
        op: Compare::Eq,
        left: Expr::Const(0),
        right: Expr::Const(0),
    }
}

/// Every successful child consumes preorder positions before the later rejection.
#[test]
fn operation_indices_include_all_structured_bodies_in_order() {
    let input = program(vec![
        Op::Scope(vec![Op::Print(Expr::Const(1))]), // 0, 1
        Op::Region {
            name: "first".into(), // 2
            capacity: Expr::Const(0),
            fail_acquire: false,
            body: vec![Op::Print(Expr::Const(2))],       // 3
            on_failure: vec![Op::Print(Expr::Const(3))], // 4
        },
        Op::Region {
            name: "second".into(), // 5
            capacity: Expr::Const(0),
            fail_acquire: false,
            body: vec![Op::Batch {
                name: "items".into(), // 6
                region: "second".into(),
                record: "Empty".into(),
                body: vec![
                    Op::Repeat {
                        count: Expr::Const(0), // 7
                        index: "i".into(),
                        body: vec![Op::Print(Expr::Const(4))], // 8
                    },
                    Op::ForEach {
                        batch: "items".into(), // 9
                        reference: "item".into(),
                        access: Access::Readonly,
                        body: vec![Op::Print(Expr::Const(5))], // 10
                    },
                    Op::If {
                        condition: condition(),                                      // 11
                        then_body: vec![Op::Print(Expr::Const(6))],                  // 12
                        else_body: vec![Op::Scope(vec![Op::Print(Expr::Const(7))])], // 13, 14
                    },
                    Op::Print(Expr::Local("missing".into())), // 15
                ],
            }],
            on_failure: vec![],
        },
    ]);
    let error = check(&input).unwrap_err();
    assert_eq!(error.operation_index(), Some(15));
    assert!(error.message().contains("missing"));
    assert_eq!(check(&input.clone()).unwrap_err(), error);
}

/// Recursive scope propagation must preserve the failing leaf rather than parent.
#[test]
fn inner_error_index_survives_enclosing_scopes_and_failure_branch() {
    let input = program(vec![Op::Scope(vec![Op::Region {
        name: "owner".into(), // 1, enclosed by 0
        capacity: Expr::Const(0),
        fail_acquire: false,
        body: vec![Op::Print(Expr::Const(0))], // 2
        on_failure: vec![Op::Repeat {
            count: Expr::Const(0), // 3
            index: "index".into(),
            body: vec![Op::If {
                condition: condition(), // 4
                then_body: vec![],
                else_body: vec![Op::Set {
                    name: "missing".into(), // 5
                    value: Expr::Const(0),
                }],
            }],
        }],
    }])]);
    assert_eq!(check(&input).unwrap_err().operation_index(), Some(5));
}

/// A failed enclosing operation is located before any of its children are visited.
#[test]
fn parent_expression_error_has_parent_position() {
    let input = program(vec![
        Op::ExitZero,
        Op::Repeat {
            count: Expr::Local("missing".into()),
            index: "index".into(),
            body: vec![Op::Print(Expr::Local("also_missing".into()))],
        },
    ]);
    assert_eq!(check(&input).unwrap_err().operation_index(), Some(1));
}

/// Invalid definitions are independent of the entry-operation traversal.
#[test]
fn definition_errors_have_no_operation_position() {
    let mut input = program(vec![Op::Print(Expr::Local("missing".into()))]);
    input.records.push(input.records[0].clone());
    assert_eq!(check(&input).unwrap_err().operation_index(), None);
}
