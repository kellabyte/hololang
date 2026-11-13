//! Source-formatting conformance for the experimental parser-backed formatter.
//!
//! These checks protect comment/literal spelling, statement-newline meaning,
//! preferred headers, indentation, failure diagnostics, and idempotence. They use
//! compiler source APIs only, require no LLVM, and make no native performance claim.

use holo::format::format;

/// Require exact canonical output and a stable second formatting pass.
/// Borrows the input/expectation and panics with source diagnostics on rejection.
fn assert_format(source: &str, expected: &str) {
    let result = format(source).unwrap();
    assert_eq!(result, expected);
    assert_eq!(format(&result).unwrap(), result);
}

/// Compact functions become preferred vertical headers and explicit body lines.
#[test]
fn vertical_headers_and_structured_bodies() {
    assert_format(
        "func add with noalloc,noblock arg left u64 as readonly copy arg right u64 as readonly copy returns u64{return core.add_wrapping(left,right)}",
        "func add\n    with noalloc, noblock\n    arg left u64 as readonly copy\n    arg right u64 as readonly copy\n    returns u64\n{\n    return core.add_wrapping(left, right)\n}\n",
    );
}

/// Parsed record fields and statements determine layout even with contextual names.
#[test]
fn records_names_literals_and_expression_grouping_remain_intact() {
    assert_format(
        "struct HTTPData{ return u64, id u64 }\nfunc Main returns HTTPData{return HTTPData{return:0x00ff,id:(1+2)*3}}",
        "struct HTTPData {\n    return u64,\n    id u64\n}\nfunc Main\n    returns HTTPData\n{\n    return HTTPData {\n        return: 0x00ff,\n        id: (1 + 2) * 3\n    }\n}\n",
    );
}

/// Package records format with their visibility and declaration order intact.
#[test]
fn package_records_keep_visibility_and_source_order() {
    assert_format(
        "namespace telemetry.frame\npublic struct Header{ kind u64 }func helper returns u64{return 1}struct Local{ value u64 }",
        "namespace telemetry.frame\npublic struct Header {\n    kind u64\n}\nfunc helper\n    returns u64\n{\n    return 1\n}\nstruct Local {\n    value u64\n}\n",
    );
    // A standalone record is valid, but its visibility requires package context.
    assert!(format("public struct Header { kind u64 }").is_err());
    // Package identity is written once, before both kinds of declaration.
    let late_header =
        format("namespace telemetry.frame\nstruct Header { kind u64 }\nnamespace telemetry.other")
            .unwrap_err();
    assert!(late_header.message.contains("before library declarations"));
}

/// Preserve every namespace component and the separator before declarations.
#[test]
fn nested_package_namespace_headers_keep_their_paths() {
    assert_format(
        "namespace holo . net . tcp\nusing holo . net\npublic func protocol returns u64{return 1}",
        "namespace holo.net.tcp\nusing holo.net\npublic func protocol\n    returns u64\n{\n    return 1\n}\n",
    );
}

/// Formatting preserves the lexical TCP owner and the refill owner operand.
#[test]
fn tcp_owner_and_refill_keep_their_scope_and_operand() {
    let source = "func main returns u64 {with core.tcp_stream() as stream {refill slot.wire,4 from stream as bytes { core.print(1) } else status { core.print(status) }; let closed=core.close(stream);return 0 }}";
    let output = format(source).unwrap();
    assert!(output.contains("    with core.tcp_stream() as stream {\n"));
    assert!(output.contains("        refill slot.wire, 4 from stream as bytes {\n"));
    assert!(output.contains("        } else status {\n"));
    assert!(output.contains("        let closed = core.close(stream);\n"));
    assert_eq!(format(&output).unwrap(), output);
}

/// Format a generic return and result-arm continuation without moving the arm boundary.
#[test]
fn error_declarations_and_result_arms_keep_their_structure() {
    assert_format(
        "namespace telemetry.frame\npublic error FrameError{BadMagic,Oversized(required u64)}\npublic func validate returns Result < u64 , FrameError > {return ok (1+2)}\nfunc handle returns u64 {\n return validate()\n   on ok(length) then length\n   on FrameError.Oversized(required) then required\n   on error then 1\n}",
        "namespace telemetry.frame\npublic error FrameError {\n    BadMagic,\n    Oversized(required u64)\n}\npublic func validate\n    returns Result<u64, FrameError>\n{\n    return ok (1 + 2)\n}\nfunc handle\n    returns u64\n{\n    return validate()\n        on ok(length) then length\n        on FrameError.Oversized(required) then required\n        on error then 1\n}\n",
    );
}

/// Child declarations and dotted cases keep their source spelling and line boundaries.
#[test]
fn error_unions_keep_child_aliases_and_paths() {
    assert_format(
        "namespace app.dashboard\nusing app.session\npublic error DashboardError{from session . SessionError as Session,Local}\nfunc handle returns u64 {\n return load()\n   on ok(count) then count\n   on DashboardError . Session . PermissionDenied then 0\n   on error then 1\n}",
        "namespace app.dashboard\nusing app.session\npublic error DashboardError {\n    from session.SessionError as Session,\n    Local\n}\nfunc handle\n    returns u64\n{\n    return load()\n        on ok(count) then count\n        on DashboardError.Session.PermissionDenied then 0\n        on error then 1\n}\n",
    );
}

/// Contextual propagation and an ordinary helper named try format distinctly.
#[test]
fn propagation_and_ordinary_try_call_keep_distinct_syntax() {
    let source = "error Invalid { Empty }\nfunc verify returns Result<u64, Invalid> {let n=try check();return ok n}\nfunc plain returns u64 {let n=try();return n}";
    let output = format(source).unwrap();
    assert!(output.contains("    let n = try check();"));
    assert!(output.contains("    let n = try();"));
    assert_eq!(format(&output).unwrap(), output);
}

/// Colon fields and append arms keep distinct indentation and pattern syntax.
#[test]
fn typed_append_preserves_destination_and_rejection_indentation() {
    let source = "func build returns Result<Row,Invalid>{return ok Row:\n  // destination field\n  value=try child(1)\n}\nfunc main returns u64 {\n let code=core.append(rows,build())\n   on ok then 0\n   on Rejected Invalid.Bad(value) then value\n   on error then 1\n return code\n}";
    assert_format(
        source,
        "func build\n    returns Result<Row, Invalid>\n{\n    return ok Row:\n        // destination field\n        value = try child(1)\n}\nfunc main\n    returns u64\n{\n    let code = core.append(rows, build())\n        on ok then 0\n        on Rejected Invalid.Bad(value) then value\n        on error then 1\n    return code\n}\n",
    );
}

/// Comment bytes, UTF-8 payload, CRLF interiors, and semicolons survive layout.
#[test]
fn comments_and_literal_spellings_are_preserved() {
    let source = "// π heading\r\nfunc main returns u64{ /* nested /* λ */\r\n keep this indent */\n\tlet value=0x000A; // trailing\n return value }\r\n";
    let output = format(source).unwrap();
    assert!(output.starts_with("// π heading\nfunc main\n"));
    assert!(output.contains("/* nested /* λ */\r\n keep this indent */"));
    assert!(output.contains("let value = 0x000A; // trailing"));
    assert!(output.ends_with("    return value\n}\n"));
    assert_eq!(format(&output).unwrap(), output);
}

/// Newlines ending complete expressions never turn independent statements into calls.
#[test]
fn significant_newlines_and_explicit_postfix_calls_are_distinct() {
    let source = "func main returns u64{\ncall\n(other)\nreturn Item { value: 1 }(2)\n}";
    let output = format(source).unwrap();
    assert!(output.contains("    call\n    (other)\n"));
    assert!(output.contains("    }(2)\n"));
    assert_eq!(format(&output).unwrap(), output);
}

/// Recovery, branches, lists, indexing, and empty nested scopes remain syntactic.
#[test]
fn multiline_control_flow_and_lists_are_stable() {
    let source = "func main returns u64{let memory=core.region(32) else { return 0 }; if true { var x=core.bytes(\n2,\n3\n)\nx[0]=1 } else { {} }\nfor readonly item in memory { core.print(item.id) }\nreturn 0}";
    let output = format(source).unwrap();
    assert!(output.contains("let memory = core.region(32) else {\n        return 0\n    };"));
    assert!(output.contains("var x = core.bytes(\n            2,\n            3\n        )"));
    assert!(output.contains("x[0] = 1"));
    assert_eq!(format(&output).unwrap(), output);
}

/// Keep both owner scopes and the indentation of the single construction site.
#[test]
fn scoped_region_formats_with_its_recovery_and_construct_arms() {
    assert_format(
        "func main returns u64{with core . region (32) {\n core.construct(open_ticket(1))\n  on ok(ticket) then {core.print(ticket.id)}\n  on error then {return 0}\n}else {with(1002)}\nreturn 0}",
        "func main\n    returns u64\n{\n    with core.region(32) {\n        core.construct(open_ticket(1))\n            on ok(ticket) then {\n                core.print(ticket.id)\n            }\n            on error then {\n                return 0\n            }\n    } else {\n        with(1002)\n    }\n    return 0\n}\n",
    );
}

/// Real maintained examples preserve syntax and settle after one formatting pass.
#[test]
fn maintained_examples_are_formattable_and_idempotent() {
    let examples = [
        include_str!("../../fixtures/region.holo"),
        include_str!("../../fixtures/byte_input.holo"),
        include_str!("../../fixtures/recovery.holo"),
        include_str!("../../fixtures/gzip_header/main.holo"),
        include_str!("../../fixtures/typed_refill_frame.holo"),
    ];
    // The examples exercise nested, recovery, and refill forms; no semantic lowering
    // or generated program is substituted for formatting the actual source text.
    for source in examples {
        let output = format(source).unwrap();
        assert_eq!(format(&output).unwrap(), output);
    }
}

/// Header layout uses parsed positions even when names resemble header keywords.
#[test]
fn contextual_header_names_and_interleaved_declarations_do_not_panic() {
    let source = "func first returns u64 {}struct Return{arg u64,return u64}\nfunc second with return arg arg u64 as readonly copy returns u64 { return arg }\nfunc third returns u64 { return consume((Return{arg:1,return:2})) }";
    let output = format(source).unwrap();
    assert!(output.contains("func first\n    returns u64\n{\n}"));
    assert!(output.contains("with return\n    arg arg u64 as readonly copy\n"));
    let first = output.find("func first").unwrap();
    let record = output.find("struct Return").unwrap();
    let second = output.find("func second").unwrap();
    assert!(first < record && record < second);
    assert_eq!(format(&output).unwrap(), output);
}

/// Format the call without changing literal bytes, escapes, or grouping.
#[test]
fn literal_print_keeps_its_written_text() {
    assert_format(
        r#"func main returns u64{core.print("héllo // \"Holo\"");return 0}"#,
        "func main\n    returns u64\n{\n    core.print(\"héllo // \\\"Holo\\\"\");\n    return 0\n}\n",
    );
}

/// Empty/trivia files normalize predictably and malformed source returns diagnostics.
#[test]
fn empty_input_and_syntax_errors_have_explicit_behavior() {
    assert_format(" \t\n\n", "");
    assert_format("// text\n\n\n", "// text\n");
    let malformed = [
        "func main returns u64 { return 0",
        "func main returns u64 { return 1 let value = 2 }",
        "/* unterminated",
        "func main returns u64 { return 18446744073709551616 }",
        "func main returns u64 { return \"unsupported\" }",
    ];
    // Rejection belongs to the existing lexical/parser contract; no partial
    // formatted source or relaxed language mode is accepted for broken input.
    for source in malformed {
        let diagnostic = format(source).unwrap_err();
        assert!(!diagnostic.message.is_empty());
        assert!(diagnostic.span.start <= source.len());
    }
}
