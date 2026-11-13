//! Syntax-only acceptance/rejection tests for the experimental source frontend.
//!
//! These tests inspect spanned ASTs, never substitute parsed syntax for checked
//! execution evidence. They cover preferred headers, lexical boundaries, record
//! and control-flow brace ambiguity, and bounded rejection of hostile nesting.

use holo::frontend::ast::{
    BinaryOp, Expr, ExprKind, Function, ResultPattern, Statement, StatementKind,
};
use holo::frontend::parse::parse;

/// Parse one complete function, returning its owned syntax for focused assertions.
/// Test sources are trusted fixture strings; an unexpected parse failure fails the test.
fn function(source: &str) -> Function {
    parse(source).expect("valid syntax").functions.remove(0)
}

/// Wrap statements in the explicit entry header and return its parsed body.
/// Allocation belongs to the test harness; semantic checking is outside this helper.
fn body(source: &str) -> Vec<Statement> {
    function(&format!("func main\n    returns u64\n{{\n{source}\n}}")).body
}

/// Parse one rejected source and require an informative, in-range diagnostic.
/// The returned message permits callers to verify the relevant grammar boundary.
fn rejection(source: &str) -> String {
    let diagnostic = parse(source).expect_err("malformed source must fail");
    assert!(diagnostic.span.start <= diagnostic.span.end);
    assert!(diagnostic.span.end <= source.len());
    assert!(!diagnostic.message.is_empty());
    diagnostic.message
}

#[test]
/// Preferred headers retain explicit access/passing modes and exact contract spans.
fn preferred_header_and_record_declarations_preserve_written_information() {
    let source = "struct Sample {\n    value u64\n    payload Bytes\n}\n\nfunc inspect\n    with noalloc, noblock\n    arg samples Samples as readwrite reference\n    arg ceiling u64 as readonly copy\n    returns u64\n{\n    return ceiling\n}";
    let module = parse(source).expect("preferred declarations");
    assert_eq!(module.records[0].name, "Sample");
    assert_eq!(module.records[0].fields[1].ty, "Bytes");
    let function = &module.functions[0];
    assert_eq!(function.name, "inspect");
    assert_eq!(function.args[0].access, "readwrite");
    assert_eq!(function.args[0].passing, "reference");
    assert_eq!(function.args[1].access, "readonly");
    assert_eq!(function.args[1].passing, "copy");
    assert_eq!(function.result, "u64");
    assert_eq!(function.guarantees.len(), 2);
    let (contract, span) = &function.guarantees[1];
    assert_eq!(contract, "noblock");
    assert_eq!(&source[span.start..span.end], "noblock");
    assert_eq!(
        &source[function.span.start..function.span.end],
        &source[source.find("func").unwrap()..]
    );
}

/// Union members keep their written child source separate from ordinary payloads.
#[test]
fn child_error_declarations_preserve_source_identity_and_spans() {
    let source = "error DashboardError { from session.SessionError as Session, from }\nerror Other { from(code u64) }";
    let module = parse(source).expect("valid union syntax");
    let members = &module.errors[0].cases;
    assert_eq!(members[0].name, "Session");
    assert!(members[0].payload.is_none());
    let child = members[0].source.as_ref().expect("written child source");
    assert_eq!(
        &source[child.span.start..child.span.end],
        "session.SessionError"
    );
    let ExprKind::Member { base, field } = &child.kind else {
        panic!("qualified source path");
    };
    assert!(matches!(&base.kind, ExprKind::Name(name) if name == "session"));
    assert_eq!(field, "SessionError");
    assert_eq!(
        &source[members[0].span.start..members[0].span.end],
        "from session.SessionError as Session"
    );
    assert_eq!(members[1].name, "from");
    assert!(members[1].source.is_none());
    let ordinary = &module.errors[1].cases[0];
    assert_eq!(
        ordinary.payload.as_ref().expect("ordinary payload").name,
        "code"
    );
    assert!(ordinary.source.is_none());
    // A source without an alias separator is neither a case nor a child.
    assert!(
        rejection("error DashboardError { from SessionError Session }").contains("expected `as`")
    );
}

/// Error declarations and qualified result syntax keep complete original spans.
#[test]
fn typed_returns_and_result_arms_retain_written_syntax() {
    let source = "error FrameError { BadMagic, Oversized(required u64) }\nfunc validate returns Result< u64, FrameError > { return error FrameError.Oversized(4097) }\nfunc caller returns u64 {\n    let value = validate()\n        on ok(length) then length\n        on FrameError.Oversized(required) then required\n        on error(reason) then 1\n    return value\n}";
    let module = parse(source).expect("valid typed syntax");
    assert_eq!(module.errors[0].name, "FrameError");
    let case = &module.errors[0].cases[1];
    assert_eq!(case.name, "Oversized");
    assert_eq!(
        &source[case.span.start..case.span.end],
        "Oversized(required u64)"
    );
    assert_eq!(case.payload.as_ref().unwrap().name, "required");
    let validate = &module.functions[0];
    assert_eq!(validate.result, "Result<u64,FrameError>");
    assert_eq!(
        &source[validate.result_span.start..validate.result_span.end],
        "Result< u64, FrameError >"
    );
    assert!(matches!(
        validate.body[0].kind,
        StatementKind::Return(Expr {
            kind: ExprKind::Error {
                payload: Some(_),
                ..
            },
            ..
        })
    ));
    // The lexical binding owns the complete handler rather than a second statement.
    let StatementKind::Let {
        value: Expr {
            kind: ExprKind::Handle { arms, .. },
            ..
        },
        ..
    } = &module.functions[1].body[0].kind
    else {
        panic!("expected a handler expression");
    };
    assert!(matches!(&arms[0].pattern, ResultPattern::Ok(Some(name)) if name == "length"));
    assert!(
        matches!(&arms[1].pattern, ResultPattern::Error { case: Expr { kind: ExprKind::Member { field, .. }, .. }, binding: Some(binding) } if field == "Oversized" && binding == "required")
    );
    assert!(matches!(&arms[2].pattern, ResultPattern::Fallback(Some(name)) if name == "reason"));
}

/// Only physical, consistently indented result arms extend a complete operand.
#[test]
fn result_arm_indentation_and_contextual_calls_are_distinct() {
    let wrong = "func main returns u64 {\n    return validate()\n    on ok(length) then length\n}";
    assert!(rejection(wrong).contains("indented deeper"));
    let inconsistent = "func main returns u64 {\n    return validate()\n        on ok(length) then length\n          on error then 0\n}";
    assert!(rejection(inconsistent).contains("same indentation"));
    let missing_then =
        "func main returns u64 {\n    return validate()\n        on ok(length) length\n}";
    assert!(rejection(missing_then).contains("expected `then`"));
    // A call with a contextual name remains a call in the ordinary grammar.
    let old = body("let value = 1\non(value)\nreturn ok (value)");
    assert!(matches!(
        old[1].kind,
        StatementKind::Expr(Expr {
            kind: ExprKind::Call { .. },
            ..
        })
    ));
    assert!(matches!(
        old[2].kind,
        StatementKind::Return(Expr {
            kind: ExprKind::Call { .. },
            ..
        })
    ));
}

/// Preserve the contextual owner prefix, both lexical scopes, and source ranges.
#[test]
fn scoped_region_statement_keeps_budget_and_outcomes_separate() {
    let source = "func main returns u64 {\n    with(3)\n    with core.region(\n        size(32),\n    ) {\n        core.construct(open_ticket(1))\n            on ok(ticket) then { core.print(ticket.id) }\n            on error then { return 0 }\n    }\n    else { core.print(1002) }\n    return 0\n}";
    let parsed = function(source);
    assert!(matches!(
        parsed.body[0].kind,
        StatementKind::Expr(Expr {
            kind: ExprKind::Call { .. },
            ..
        })
    ));
    let region = &parsed.body[1];
    let StatementKind::WithRegion {
        capacity,
        body,
        on_failure,
    } = &region.kind
    else {
        panic!("expected a scoped region");
    };
    assert_eq!(&source[capacity.span.start..capacity.span.end], "size(32)");
    assert!(matches!(body[0].kind, StatementKind::Construct { .. }));
    assert_eq!(
        &source[on_failure[0].span.start..on_failure[0].span.end],
        "core.print(1002)"
    );
    assert!(source[region.span.start..region.span.end].starts_with("with core.region("));
    assert!(source[region.span.start..region.span.end].ends_with("else { core.print(1002) }"));
    assert!(matches!(parsed.body[2].kind, StatementKind::Return(_)));
}

/// An owner requires one budget and an explicit braced acquisition failure path.
#[test]
fn scoped_region_statement_requires_a_budget_and_recovery() {
    let missing = "func main returns u64 {\n    with core.region(32) {}\n    return 0\n}";
    let diagnostic = parse(missing).expect_err("recovery must be written");
    assert!(diagnostic.message.contains("acquisition recovery"));
    assert_eq!(
        &missing[diagnostic.span.start..diagnostic.span.end],
        "with core.region(32) {}"
    );
    // Missing and surplus operands must fail inside the owner budget list.
    for invalid in [
        "func main returns u64 { with core.region() {} else {} }",
        "func main returns u64 { with core.region(32, 64) {} else {} }",
    ] {
        assert!(!rejection(invalid).is_empty());
    }
    assert!(
        rejection("func main returns u64 { with core.region(32) {} else return 0 }")
            .contains("expected `{`")
    );
}

#[test]
/// Delimiters and explicit header words permit continuation independent of indentation.
fn multiline_arguments_fields_and_trailing_commas_parse() {
    let source = "struct Pair {left u64, right u64,}\nfunc make\nwith noalloc,\n noblock\narg\n value\n u64\n as\n readonly\n copy\nreturns Pair\n{\nreturn Pair {\n left: value,\n right: core.wrapping_add(\nvalue,\n1,\n),\n}\n}";
    let function = function(source);
    assert_eq!(function.args[0].name, "value");
    // The return must retain record fields in source evaluation order.
    let StatementKind::Return(Expr {
        kind: ExprKind::Record { fields, .. },
        ..
    }) = &function.body[0].kind
    else {
        panic!("expected a record return");
    };
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["left", "right"]
    );
    // The second field must retain the ordinary namespace call and both arguments.
    let ExprKind::Call { args, .. } = &fields[1].value.kind else {
        panic!("expected an ordinary call");
    };
    assert_eq!(args.len(), 2);
}

#[test]
/// Batch names followed by loop bodies must remain names, never empty record literals.
fn for_loop_braces_are_not_record_initializers() {
    let statements = body(
        "for readonly entry in items { core.print(entry.value) }\nfor index in core.range(count) { core.print(index) }",
    );
    // Both loop forms share syntax while preserving explicit cursor permissions.
    let StatementKind::For {
        name,
        access,
        iterable,
        body,
    } = &statements[0].kind
    else {
        panic!("expected batch iteration");
    };
    assert_eq!(name, "entry");
    assert_eq!(access.as_deref(), Some("readonly"));
    assert!(matches!(&iterable.kind, ExprKind::Name(name) if name == "items"));
    assert_eq!(body.len(), 1);
    let StatementKind::For {
        access, iterable, ..
    } = &statements[1].kind
    else {
        panic!("expected count iteration");
    };
    assert!(access.is_none());
    assert!(matches!(iterable.kind, ExprKind::Call { .. }));
}

#[test]
/// A comparison's final named operand must leave the following brace for the if body.
fn named_condition_operands_and_else_branches_parse() {
    let statements =
        body("if left == right\n{\n core.print(1)\n}\nelse\n{ core.print(2) }\nreturn 0");
    // The branch AST preserves its comparison and exactly one statement per arm.
    let StatementKind::If {
        condition,
        then_body,
        else_body,
    } = &statements[0].kind
    else {
        panic!("expected a conditional");
    };
    let ExprKind::Binary { op, right, .. } = &condition.kind else {
        panic!("expected comparison");
    };
    assert_eq!(*op, BinaryOp::Eq);
    assert!(matches!(&right.kind, ExprKind::Name(name) if name == "right"));
    assert_eq!(then_body.len(), 1);
    assert_eq!(else_body.len(), 1);
    assert!(matches!(statements[1].kind, StatementKind::Return(_)));
}

#[test]
/// Parentheses explicitly disambiguate a record expression inside a condition.
fn parenthesized_records_remain_available_in_conditions() {
    let statements = body("if (Flag { value: 1 }) == other { return 0 }");
    // Parentheses alter grouping while the source span still includes their delimiters.
    let StatementKind::If { condition, .. } = &statements[0].kind else {
        panic!("expected if");
    };
    let ExprKind::Binary { left, .. } = &condition.kind else {
        panic!("expected comparison");
    };
    assert!(matches!(left.kind, ExprKind::Record { .. }));
}

#[test]
/// Binding recovery preserves its body and does not consume a subsequent statement boundary.
fn fallible_binding_and_ordinary_binding_keep_distinct_scopes() {
    let statements = body(
        "let memory = core.region(8192) else { core.print(1); return 0 }\nlet count = 4\nvar total = 0\nreturn total",
    );
    // Only the acquisition binding has a recovery block; mutability is retained separately.
    let StatementKind::Let {
        name,
        mutable,
        otherwise,
        ..
    } = &statements[0].kind
    else {
        panic!("expected region binding");
    };
    assert_eq!(name, "memory");
    assert!(!mutable);
    assert_eq!(otherwise.as_ref().unwrap().len(), 2);
    assert!(matches!(
        &statements[1].kind,
        StatementKind::Let {
            otherwise: None,
            ..
        }
    ));
    assert!(matches!(
        &statements[2].kind,
        StatementKind::Let { mutable: true, .. }
    ));
    assert_eq!(statements.len(), 4);
}

#[test]
/// Assignments retain member/index projections and complete source ranges.
fn projected_assignment_and_nested_scope_parse() {
    let source = "func main returns u64 {\n{\n entry.payload[core.index()] = value\n}\nreturn 0\n}";
    let function = function(source);
    // An explicit scope is represented independently from the assignment's destination expression.
    let StatementKind::Scope(scoped) = &function.body[0].kind else {
        panic!("expected scope");
    };
    let StatementKind::Assign { target, value } = &scoped[0].kind else {
        panic!("expected assignment");
    };
    assert!(matches!(target.kind, ExprKind::Index { .. }));
    assert!(matches!(&value.kind, ExprKind::Name(name) if name == "value"));
    let span = scoped[0].span;
    assert_eq!(
        &source[span.start..span.end],
        "entry.payload[core.index()] = value"
    );
}

#[test]
/// The parser groups multiplication before addition and arithmetic before comparison.
fn arithmetic_precedence_preserves_operator_grouping() {
    let statements = body("return 1 + 2 * 3 == 7");
    // Multiplication groups inside addition, which groups inside comparison.
    let StatementKind::Return(Expr {
        kind:
            ExprKind::Binary {
                op: BinaryOp::Eq,
                left,
                ..
            },
        ..
    }) = &statements[0].kind
    else {
        panic!("expected equality");
    };
    let ExprKind::Binary {
        op: BinaryOp::Add,
        right,
        ..
    } = &left.kind
    else {
        panic!("expected addition");
    };
    assert!(matches!(
        right.kind,
        ExprKind::Binary {
            op: BinaryOp::Mul,
            ..
        }
    ));
}

#[test]
fn boolean_precedence_and_contextual_names_preserve_grouping() {
    let statements = body("return not left == right and ready() or not(false)");
    let StatementKind::Return(Expr {
        kind:
            ExprKind::Binary {
                op: BinaryOp::LogicalOr,
                left,
                right,
                ..
            },
        ..
    }) = &statements[0].kind
    else {
        panic!("expected disjunction");
    };
    assert!(matches!(right.kind, ExprKind::Call { .. }));
    let ExprKind::Binary {
        op: BinaryOp::LogicalAnd,
        left,
        ..
    } = &left.kind
    else {
        panic!("expected conjunction");
    };
    let ExprKind::Not { value, .. } = &left.kind else {
        panic!("expected negation");
    };
    assert!(matches!(
        value.kind,
        ExprKind::Binary {
            op: BinaryOp::Eq,
            ..
        }
    ));

    let statements =
        body("let not = true\nlet result = not and not (false)\nor()\nand()\nnot()\nreturn 0");
    assert_eq!(statements.len(), 6);
    let StatementKind::Let { value, .. } = &statements[1].kind else {
        panic!("expected binding");
    };
    let ExprKind::Binary {
        op: BinaryOp::LogicalAnd,
        left,
        right,
        ..
    } = &value.kind
    else {
        panic!("expected conjunction");
    };
    assert!(matches!(&left.kind, ExprKind::Name(name) if name == "not"));
    assert!(matches!(right.kind, ExprKind::Not { .. }));
}

#[test]
/// Equal-precedence operators remain left-associated, preserving written evaluation order.
fn operator_chains_are_left_associative() {
    let statements = body("return 8 - 3 - 1");
    // The AST is (8 - 3) - 1 rather than 8 - (3 - 1).
    let StatementKind::Return(Expr {
        kind: ExprKind::Binary { left, right, .. },
        ..
    }) = &statements[0].kind
    else {
        panic!("expected subtraction");
    };
    assert!(matches!(
        left.kind,
        ExprKind::Binary {
            op: BinaryOp::Sub,
            ..
        }
    ));
    assert!(matches!(right.kind, ExprKind::Number(1)));
}

#[test]
/// Newline/semicolon boundaries reject concatenated simple statements.
fn missing_statement_separators_are_rejected() {
    assert!(rejection("func main returns u64 { let x = 1 let y = 2 }").contains("newline"));
    assert!(rejection("func main returns u64 { core.print(1) return 0 }").contains("newline"));
    assert!(rejection("struct Pair { x u64 y u64 }").contains("newline"));
}

#[test]
/// Required header clauses fail explicitly rather than receiving implicit argument defaults.
fn incomplete_and_reordered_headers_are_rejected() {
    let sources = [
        "func main { return 0 }",
        "func main() returns u64 { return 0 }",
        "func main arg value u64 returns u64 { return 0 }",
        "func main arg value u64 as readonly returns u64 { return 0 }",
        "func main arg value u64 as readonly copy with noalloc returns u64 { return 0 }",
        "func main with noalloc with noblock returns u64 { return 0 }",
        "func main returns u64 returns u64 { return 0 }",
    ];
    // Every malformed header must fail without accepting a partial declaration.
    for source in sources {
        rejection(source);
    }
}

/// A signature requires `returns`, while a body continues to use `return`.
#[test]
fn signature_and_statement_use_distinct_keywords() {
    let parsed = function("func main returns u64 { return 0 }");
    assert_eq!(parsed.result, "u64");
    assert!(rejection("func main return u64 { return 0 }").contains("returns"));
    rejection("func main returns u64 { returns 0 }");
}

#[test]
/// Unsupported argument spellings and generic types produce explicit subset diagnostics.
fn unsupported_modes_and_parameterized_types_are_rejected() {
    assert!(
        rejection("func f arg x u64 as writeonly copy returns u64 { return 0 }").contains("access")
    );
    assert!(
        rejection("func f arg x u64 as readonly implicit returns u64 { return 0 }")
            .contains("passing")
    );
    assert!(rejection("struct Item { data Batch<u64> }").contains("parameterized types"));
    assert!(
        rejection("func f returns Option<Bytes> { return None }").contains("require `Option<u64>`")
    );
    assert!(
        rejection("func f arg value Option<u64> as readonly copy returns u64 { return 0 }")
            .contains("parameterized types")
    );
}

#[test]
/// Missing delimiters and dangling else clauses are syntax errors, not partial successes.
fn malformed_delimiters_and_trailing_source_are_rejected() {
    let sources = [
        "func main returns u64 {",
        "struct Item { value u64",
        "func main returns u64 { core.print(1] }",
        "func main returns u64 { return Item { value: 1 }",
        "func main returns u64 { else { return 0 } }",
        "func main returns u64 { return 0 } trailing",
        "func main returns u64 { let x = Item { value 1 } }",
    ];
    // Each input exercises a distinct enclosing boundary that must retain ownership of its delimiter.
    for source in sources {
        rejection(source);
    }
}

#[test]
/// Nested and long expressions fail before recursive AST visitors/drop threaten the host stack.
fn hostile_expression_nesting_is_bounded() {
    let deeply_nested = format!("{}0{}", "(".repeat(256), ")".repeat(256));
    let left_chain = std::iter::repeat_n("1", 300)
        .collect::<Vec<_>>()
        .join(" + ");
    let projection_chain = format!("value{}", ".field".repeat(300));
    let negation_chain = format!("{}true", "not ".repeat(256));
    for expression in [deeply_nested, left_chain, projection_chain, negation_chain] {
        assert!(
            rejection(&format!("func main returns u64 {{ return {expression} }}"))
                .contains("limit")
        );
    }
}

#[test]
/// Nested lexical scopes share the same bounded parser recursion as expressions.
fn hostile_block_nesting_is_bounded() {
    let source = format!(
        "func main returns u64 {{ {} return 0 {} }}",
        "{".repeat(256),
        "}".repeat(256)
    );
    assert!(rejection(&source).contains("limit"));
}
