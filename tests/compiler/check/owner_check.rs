//! Checker-only obligations for independent owners and nested marker reset.
//!
//! Existing tests cover one-root alias rejection and reverse native cleanup.
//! This file checks that root identity survives nested owners, cursors, lexical
//! shadowing and reset/reuse: unrelated references must not block a valid reset,
//! while a live reference into the selected root must still block it. Structured
//! IR and actual parsed source enter the same checker; no native performance,
//! runtime allocation or broader sharing rule is inferred from these fixtures.

use holo::check::check;
use holo::frontend::parse_experimental;
use holo::ir::{
    Access, Build, Compare, Condition, Expr, Field, FieldKind, Op, Producer, Program, Record,
};
use holo::ir::{Argument, Parameter};

/// Return a plain one-scalar schema and constructor around caller-owned entry IR.
///
/// `body` is moved into unchecked test input. Host vectors/strings allocate only
/// test/compiler memory; checking must authorize the returned program separately.
fn program(body: Vec<Op>) -> Program {
    Program {
        records: vec![Record {
            name: "Item".into(),
            fields: vec![Field {
                name: "id".into(),
                kind: FieldKind::U64,
            }],
        }],
        producers: vec![Producer {
            name: "item".into(),
            record: "Item".into(),
            error: None,
            params: vec![Parameter::U64; 1],
            body: vec![Build::U64 {
                field: "id".into(),
                value: Expr::Param(0),
            }],
        }],
        body,
    }
}

/// Wrap `body` in one finite backing owner, with an empty acquisition-failure path.
/// The owner exists only in success; this creates IR and performs no acquisition.
fn region(name: &str, body: Vec<Op>) -> Op {
    Op::Region {
        name: name.into(),
        capacity: Expr::Const(128),
        fail_acquire: false,
        body,
        on_failure: vec![],
    }
}

/// Give `name` the tail of `memory`, publish one item, then run caller `body`.
/// The batch's lexical end releases ownership; an explicit reset alone does not.
fn batch(name: &str, memory: &str, body: Vec<Op>) -> Op {
    let mut operations = vec![Op::Append {
        batch: name.into(),
        producer: "item".into(),
        args: vec![Expr::Const(7)]
            .into_iter()
            .map(Argument::Scalar)
            .collect(),
        status: "initial".into(),
    }];
    operations.extend(body);
    Op::Batch {
        name: name.into(),
        region: memory.into(),
        record: "Item".into(),
        body: operations,
    }
}

/// Borrow record zero of `batch` through the requested typed path permission.
/// Its lifetime/root belongs to the existing batch; no ownership is transferred.
fn borrow(name: &str, batch: &str, access: Access) -> Op {
    Op::Borrow {
        name: name.into(),
        batch: batch.into(),
        index: Expr::Const(0),
        access,
    }
}

/// Return a checked scalar read through a live Item reference named `reference`.
fn id(reference: &str) -> Expr {
    Expr::Read {
        reference: reference.into(),
        path: vec!["id".into()],
    }
}

/// Name the batch whose old values become unusable after a permitted reset.
fn reset(batch: &str) -> Op {
    Op::Reset {
        batch: batch.into(),
    }
}

/// Place work inside two independent live backing/batch pairs.
/// `inner` executes while the outer readonly alias remains valid and usable.
fn independent(inner: Vec<Op>) -> Program {
    program(vec![region(
        "outer_memory",
        vec![batch(
            "outer",
            "outer_memory",
            vec![
                borrow("outside", "outer", Access::Readonly),
                region("inner_memory", vec![batch("inner", "inner_memory", inner)]),
                Op::Print(id("outside")),
            ],
        )],
    )])
}

/// Require a semantic rejection and retain the diagnostic's meaningful reason.
/// `input` is borrowed and unchanged; an unexpected acceptance panics the test.
fn rejects(input: &Program, expected: &str) {
    let error = check(input).expect_err("invalid owner operation was accepted");
    assert!(
        error.message().contains(expected),
        "expected {expected:?}, got {error}"
    );
}

/// Outer aliases must not impose a global prohibition on independent inner reset.
#[test]
fn independent_reset_ignores_other_roots_but_preserves_outer_alias() {
    let input = independent(vec![
        Op::Scope(vec![
            borrow("inside", "inner", Access::Readwrite),
            Op::Alias {
                name: "inside_reader".into(),
                source: "inside".into(),
                access: Access::Readonly,
            },
            Op::Print(id("inside_reader")),
        ]),
        reset("inner"),
        Op::Print(id("outside")),
    ]);
    check(&input)
        .expect("closed inner references must permit inner reset beside a live outer alias");
}

/// Reset chooses the referenced root, not whichever independent alias is nearest.
#[test]
fn each_reset_rejects_its_own_live_root_with_both_roots_present() {
    // Selecting either owner is invalid for a different live root-specific alias;
    // checking both prevents a scan that stops after seeing only an unrelated root.
    for selected in ["inner", "outer"] {
        let input = independent(vec![
            borrow("inside", "inner", Access::Readonly),
            reset(selected),
        ]);
        rejects(
            &input,
            &format!("cannot reset batch {selected} while a reference is live"),
        );
    }
}

/// An outer cursor can coexist with independently resetting inner storage.
#[test]
fn outer_cursor_does_not_block_inner_reset_but_still_blocks_outer_reset() {
    // The same cursor/body pair differs only in which backing root is selected.
    // This guards root separation through ForEach's fresh reference scope.
    for (selected, accepted) in [("inner", true), ("outer", false)] {
        let input = program(vec![region(
            "outer_memory",
            vec![batch(
                "outer",
                "outer_memory",
                vec![Op::ForEach {
                    batch: "outer".into(),
                    reference: "cursor".into(),
                    access: Access::Readonly,
                    body: vec![region(
                        "inner_memory",
                        vec![batch(
                            "inner",
                            "inner_memory",
                            vec![reset(selected), Op::Print(id("cursor"))],
                        )],
                    )],
                }],
            )],
        )]);
        // Positive cases must compile too; a conservative rejection of all nested
        // resets would satisfy only the negative half and violate the approved roots.
        if accepted {
            check(&input).expect("outer cursor cannot block an unrelated inner reset");
        } else {
            rejects(&input, "cannot reset batch outer while a reference is live");
        }
    }
}

/// Nested branch/loop traversal cannot lose an older owner's alias hazard.
#[test]
fn inner_control_flow_cannot_hide_the_selected_outer_root() {
    let input = independent(vec![Op::Repeat {
        count: Expr::Const(2),
        index: "iteration".into(),
        body: vec![Op::If {
            condition: Condition {
                op: Compare::Eq,
                left: Expr::Local("iteration".into()),
                right: Expr::Const(1),
            },
            then_body: vec![reset("inner")],
            else_body: vec![reset("outer")],
        }],
    }]);
    rejects(&input, "cannot reset batch outer while a reference is live");
}

/// Shadowed names choose the inner root until lexical exit restores the outer one.
#[test]
fn lexical_shadowing_keeps_outer_references_bound_to_the_original_owner() {
    let mut body = vec![
        borrow("outside", "items", Access::Readonly),
        region(
            "memory",
            vec![batch("items", "memory", vec![reset("items")])],
        ),
        Op::Print(id("outside")),
    ];
    let input = program(vec![region(
        "memory",
        vec![batch("items", "memory", body.clone())],
    )]);
    check(&input).expect("shadowed source names are not identical storage roots");
    // The inner owner has ended, so the same spelling now selects the protected
    // outer owner. Closing the shadow must not accidentally discard its alias.
    body.push(reset("items"));
    let invalid = program(vec![region("memory", vec![batch("items", "memory", body)])]);
    rejects(
        &invalid,
        "cannot reset batch items while a reference is live",
    );
}

/// Reset empties contents while lexical batch ownership still reserves the tail.
#[test]
fn reset_does_not_end_ownership_but_lexical_batch_exit_allows_reuse() {
    let overlapping = program(vec![region(
        "memory",
        vec![batch(
            "first",
            "memory",
            vec![reset("first"), batch("second", "memory", vec![])],
        )],
    )]);
    rejects(&overlapping, "already has a live batch owner");
    let sequential = program(vec![region(
        "memory",
        vec![
            batch("first", "memory", vec![reset("first")]),
            batch("second", "memory", vec![reset("second")]),
        ],
    )]);
    check(&sequential).expect("ending the first batch returns the region tail to its owner");
}

/// Build actual source with two live independent owners and a root-selecting body.
/// Input statements are maintained test text, not runtime data or arbitrary user code.
fn source(inner: &str) -> String {
    format!(
        r#"// Plain data and destination-only construction for checker-root tests.
struct Item {{
    // Published scalar whose alias remains readable across unrelated resets.
    id u64
}}
// Construct one Item in the receiving append slot; no source reference escapes.
func item with noblock arg id u64 as readonly copy returns Item {{
    return Item {{ id: id }}
}}
// Keep an outer alias live while checking operations on an independent owner.
func main returns u64 {{
    let memory = core.region(128) else {{ return 0 }}
    let items = core.batch(memory, Item)
    let first_status = core.append(items, item(7))
    core.assert(first_status == 0)
    let outside = core.borrow_readonly(items, 0)
    {{
        let inner_memory = core.region(128) else {{ return 0 }}
        let inner_items = core.batch(inner_memory, Item)
        let inner_status = core.append(inner_items, item(9))
        core.assert(inner_status == 0)
        {inner}
    }}
    core.assert(outside.id == 7)
    return 0
}}
"#
    )
}

/// The parsed source adapter must preserve root-specific acceptance and diagnostics.
#[test]
fn parsed_source_preserves_independent_owner_reset_obligations() {
    let valid = source("core.reset(inner_items)\ncore.assert(outside.id == 7)");
    parse_experimental(&valid)
        .expect("valid independent owner source must parse")
        .check()
        .expect("live outer alias must permit inner source reset");
    // Both invalid source variants differ in the protected root; exact spans
    // ensure checking rejects the selected reset rather than another statement.
    for (body, reset_text) in [
        (
            "let inside = core.borrow_readonly(inner_items, 0)\ncore.reset(inner_items)",
            "core.reset(inner_items)",
        ),
        ("core.reset(items)", "core.reset(items)"),
    ] {
        let invalid = source(body);
        let parsed =
            parse_experimental(&invalid).expect("invalid reset still has valid source syntax");
        let error = parsed
            .check()
            .expect_err("selected live source root must block reset");
        assert!(
            error.message.contains("reference is live"),
            "{}",
            error.message
        );
        assert_eq!(&invalid[error.span.start..error.span.end], reset_text);
    }
}
