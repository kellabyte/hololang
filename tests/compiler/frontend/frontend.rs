//! Source-to-checked-IR acceptance and rejection for the experimental frontend.
//!
//! These tests parse actual Holo text, check the resulting storage operations,
//! and verify source-local errors, including automatic scoped placement.
//! Native behavior is checked separately. Fixture
//! strings allocate test/compiler memory only and do not model runtime storage.

use holo::frontend::{SourceProgram, parse_experimental};
use holo::source::Diagnostic;

/// A small schema and destination producer shared by lifetime/access fixtures.
const DEFINITIONS: &str = r#"
// Construct an inline row with an unsigned identity and initialized bytes.
struct Row {
    id u64
    data Bytes
}
// Inputs are immutable unsigned copies; the result is constructed in its destination.
func row
    with noblock
    arg id u64 as readonly copy
    arg length u64 as readonly copy
    returns Row
{
    return Row { id: id, data: core.bytes(length, core.index()) }
}
"#;

/// Wrap supplied statements in an entry body with a populated, live outer batch.
/// Caller statements remain in that batch's lexical tail; this returns owned text.
fn populated(source: &str) -> String {
    format!(
        "{DEFINITIONS}\nfunc main\n    returns u64\n{{\nlet memory = core.region(8192) else {{ return 0 }}\nlet items = core.batch(memory, Row)\nlet initial = core.append(items, row(7, 4))\ncore.assert(initial == 0)\n{source}\nreturn 0\n}}"
    )
}

/// Parse and check a valid fixture, returning its immutable lowered source mapping.
/// Unexpected errors fail with the rendered source; no LLVM discovery is performed.
fn accepted(source: &str) -> SourceProgram {
    let program = parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("fixture.holo", source)));
    program
        .check()
        .unwrap_or_else(|error| panic!("{}", error.render("fixture.holo", source)));
    program
}

/// Return a syntax, subset, or checked-semantic error for a rejected source fixture.
/// The diagnostic must always identify a valid source range rather than an IR index.
fn rejected(source: &str) -> Diagnostic {
    // Lowering and checked validation are separate gates; either may reject its own obligations.
    let diagnostic = match parse_experimental(source) {
        Err(error) => error,
        Ok(program) => program
            .check()
            .expect_err("invalid source must be rejected"),
    };
    assert!(diagnostic.span.start <= diagnostic.span.end);
    assert!(diagnostic.span.end <= source.len());
    diagnostic
}

/// Optional arms are exhaustive and the presence payload is scoped to its arm.
#[test]
fn optional_returns_reject_invalid_construction_and_handling() {
    let optional = "func find returns Option<u64> { return Some(0) }";
    for (body, message) in [
        (
            "find()\n        on Some(_) then {}",
            "missing `on None` arm",
        ),
        (
            "find()\n        on Some(_) then {}\n        on Some(_) then {}",
            "duplicate `on Some` arm",
        ),
        (
            "find()\n        on Some then {}\n        on None then {}",
            "`on Some` requires a payload binding",
        ),
        (
            "find()\n        on Some(_) then {}\n        on None(_) then {}",
            "`None` has no payload to bind",
        ),
        (
            "find()\n        on ok(_) then {}\n        on None then {}",
            "use `on Some(value)` and `on None`",
        ),
        (
            "find()\n        on Some(value) then {}\n        on None then { core.print(value) }",
            "out-of-scope binding",
        ),
    ] {
        let source = format!("{optional}\nfunc main returns u64 {{\n    {body}\n    return 0\n}}");
        let error = rejected(&source);
        assert!(error.message.contains(message), "{body}: {error:?}");
    }
    for (body, message) in [
        ("return Some()", "expected 1 arguments"),
        ("return ok 1", "return `Some(value)`"),
        (
            "let value = try find()\n    return Some(value)",
            "`try` requires a Result operand",
        ),
    ] {
        let source = format!(
            "{optional}\nfunc probe returns Option<u64> {{\n    {body}\n}}\nfunc main returns u64 {{ return 0 }}"
        );
        let error = rejected(&source);
        assert!(error.message.contains(message), "{body}: {error:?}");
    }
    let mixed = format!(
        "{optional}\nerror Missing {{ None }}\nfunc probe returns Result<u64, Missing> {{ return find() }}\nfunc main returns u64 {{ return 0 }}"
    );
    assert!(
        rejected(&mixed)
            .message
            .contains("`Option<u64>` and `Result` are distinct")
    );
}

/// An optional binding cannot be reassigned, read as a scalar, or used outside its scope.
#[test]
fn saved_optional_scalars_require_an_immutable_lexical_binding() {
    for (body, message) in [
        ("var saved = find()", "with an immutable `let`"),
        (
            "let saved = find()\n    saved = 0",
            "binding cannot be reassigned",
        ),
        (
            "let saved = find()\n    core.print(saved)",
            "handle the saved `Option<u64>`",
        ),
        (
            "{ let saved = find() }\n    saved\n        on Some(_) then {}\n        on None then {}",
            "out-of-scope binding",
        ),
        (
            "let saved = 0\n    saved\n        on Some(_) then {}\n        on None then {}",
            "a named `on` subject must be a saved `Result` or `Option<u64>`",
        ),
        (
            "let saved = find()\n    saved\n        on Some(_) then {}",
            "missing `on None` arm",
        ),
    ] {
        let source = format!(
            "func find returns Option<u64> {{ return Some(0) }}\nfunc main returns u64 {{\n    {body}\n    return 0\n}}"
        );
        let error = rejected(&source);
        assert!(error.message.contains(message), "{body}: {error:?}");
    }
}

/// Named windows can be read and lent to either kind of synchronous callee.
#[test]
fn named_byte_views_can_be_observed_and_forwarded() {
    let entry = populated(
        r#"let row = core.borrow_readonly(items, 0)
let prefix = core.prefix(row.data, 3)
let tail = core.subrange(prefix, 1, 2)
core.assert(core.len(tail) == 2)
core.assert(tail[0] == 1)
core.assert(last(tail) == 2)
let copied = core.append(items, copy_view(tail))
core.assert(copied == 0)"#,
    );
    let helpers = r#"
func last arg input Bytes as readonly reference returns u64 {
    let tail = core.subrange(input, 1, 1)
    return tail[0]
}
func copy_view arg input Bytes as readonly reference returns Row {
    return Row { id: core.len(input), data: core.bytes(core.len(input), input[core.index()]) }
}
"#;
    accepted(&format!("{helpers}\n{entry}"));
}

#[test]
fn byte_strings_supply_readonly_views_and_constructor_inputs() {
    let source = r#"
struct Row { data Bytes }
func copy arg input Bytes as readonly reference returns Row {
    return Row { data: core.bytes(core.len(input), input[core.index()]) }
}
func fixed returns Row {
    return copy(core.subrange(b"prefix", 3, 3))
}
func first arg input Bytes as readonly reference returns u64 { return input[0] }
func literal_result returns Option<u64> { return Some(b"xyz"[1]) }
func literal_len returns u64 {
    let name = b"a\x00\xFF"
    let window = core.subrange(name, 1, 2)
    return core.len(window)
}
func forward_len returns u64 { return literal_len() }
func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let items = core.batch(memory, Row)
    let status = core.append(items, fixed())
    core.assert(status == 0)
    let copied = core.append(items, copy(core.prefix(b"source", 1)))
    core.assert(copied == 0)
    let named = b"\0\xFF"
    core.assert(core.len(b"") == 0)
    core.assert(core.len(named) == forward_len())
    core.assert(named[1] == 255)
    core.assert(first(core.subrange(b"abc", 1, 2)) == b"bc"[0])
    core.assert(not b"abc"[0] == 122)
    let result = literal_result()
        on Some(value) then value
        on None then 0
    core.assert(result == 121)
    return 0
}
"#;
    accepted(source);
}

#[test]
fn byte_strings_cannot_supply_writable_storage_or_scalars() {
    for (body, expected) in [
        (r#"b"one"[0] = 1"#, "readonly byte string literal"),
        (
            r#"let data = b"one"; data[0] = 1"#,
            "readonly local byte view",
        ),
        (r#"let readwrite data = b"one""#, "literals are readonly"),
        (
            r#"let readwrite data = core.prefix(b"one", 1)"#,
            "literals are readonly",
        ),
        (r#"var data = b"one""#, "byte views cannot be rebound"),
        (r#"core.print(write(b"one"))"#, "literals are readonly"),
        (r#"core.print(b"one")"#, "cannot be used as a u64"),
    ] {
        let source = format!(
            "func write arg dest Bytes as readwrite reference returns u64 {{ return core.len(dest) }}\nfunc main returns u64 {{ {body}; return 0 }}"
        );
        assert!(rejected(&source).message.contains(expected), "{body}");
    }
}

/// The local binding belongs to its block and can only lend readonly access.
#[test]
fn named_byte_views_reject_rebinding_writes_and_escape() {
    for (body, message) in [
        (
            "var view = core.prefix(row.data, 1)",
            "byte views cannot be rebound",
        ),
        (
            "let view = core.prefix(row.data, 1)\nview[0] = 9",
            "cannot write through a readonly local byte view",
        ),
        (
            "let view = core.prefix(row.data, 1)\ncore.print(edit(view))",
            "local byte view is readonly",
        ),
        (
            "let view = core.prefix(row.data, 1)\nlet saved = view",
            "cannot become a scalar or escape",
        ),
        (
            "{ let view = core.prefix(row.data, 1) }\ncore.print(core.len(view))",
            "out-of-scope binding",
        ),
    ] {
        let entry = populated(&format!(
            "let row = core.borrow_readwrite(items, 0)\n{body}"
        ));
        let source = format!(
            "func edit arg input Bytes as readwrite reference returns u64 {{ return core.len(input) }}\n{entry}"
        );
        let error = rejected(&source);
        assert!(error.message.contains(message), "{body}: {}", error.message);
    }
}

/// Writable views cannot acquire more access than their source or change binding.
#[test]
fn named_writable_byte_views_reject_permission_escalation() {
    for (body, message) in [
        (
            "let row = core.borrow_readonly(items, 0)\nlet readwrite view = core.prefix(row.data, 1)",
            "expected a readwrite reference",
        ),
        (
            "let row = core.borrow_readwrite(items, 0)\nlet readonly = core.prefix(row.data, 2)\nlet readwrite view = core.prefix(readonly, 1)",
            "local byte view is readonly",
        ),
        (
            "let row = core.borrow_readwrite(items, 0)\nlet readwrite view = core.prefix(row.data, 1)\nview = 0",
            "binding cannot be reassigned",
        ),
        ("let readwrite value = 0", "`let readwrite` requires"),
        (
            "let row = core.borrow_readwrite(items, 0)\nvar readwrite view = core.prefix(row.data, 1)",
            "`let readwrite` requires",
        ),
    ] {
        let error = rejected(&populated(body));
        assert!(error.message.contains(message), "{body}: {}", error.message);
    }
    let readonly_parameter = r#"
func edit arg input Bytes as readonly reference returns u64 {
    let readwrite view = core.prefix(input, 1)
    return core.len(view)
}
"#;
    let source = format!("{readonly_parameter}\n{}", populated(""));
    assert!(
        rejected(&source)
            .message
            .contains("expected a readwrite Bytes parameter")
    );
}

/// A bounded batch view can lend an element with its own access or a weaker mode.
/// The original owner can continue to operate once the synchronous call returns.
#[test]
fn borrowed_batch_frontend_checks_forwarding_iteration_and_mutation() {
    let helpers = r#"
func sum
    with noalloc, noblock
    arg rows Batch<Row> as readonly reference
    returns u64
{
    var total = 0
    for readonly item in rows {
        total = core.add_wrapping(total, item.id)
    }
    return total
}
func update
    with noalloc, noblock
    arg rows Batch<Row> as readwrite reference
    returns u64
{
    let item = core.borrow_readwrite(rows, 0)
    item.id = 9
    return sum(rows)
}
"#;
    let main = populated("core.assert(update(items) == 9)\ncore.assert(core.len(items) == 1)");
    accepted(&format!("{helpers}\n{main}"));
}

/// Readonly paths cannot be upgraded, and even writable views have no owner authority.
#[test]
fn borrowed_batch_frontend_rejects_access_upgrades_and_publication() {
    for (access, statement, message) in [
        (
            "readonly",
            "let item = core.borrow_readwrite(rows, 0)",
            "readonly batch view",
        ),
        (
            "readonly",
            "for readwrite item in rows {}",
            "readonly batch view",
        ),
        ("readonly", "update(rows)", "readonly batch view"),
        (
            "readwrite",
            "let status = core.append(rows, row(1, 2))",
            "borrowed batch view",
        ),
        (
            "readwrite",
            "core.reset(rows)",
            "unavailable in ordinary borrowed functions",
        ),
    ] {
        let helper = format!(
            "func update arg rows Batch<Row> as readwrite reference returns u64 {{ return core.len(rows) }}\nfunc test arg rows Batch<Row> as {access} reference returns u64 {{\n{statement}\nreturn 0\n}}"
        );
        let source = format!("{helper}\n{}", populated("core.print(test(items))"));
        assert!(rejected(&source).message.contains(message), "{statement}");
    }

    let source = format!(
        "struct Other {{ id u64; data Bytes }}\nfunc count arg rows Batch<Other> as readonly reference returns u64 {{ return core.len(rows) }}\n{}",
        populated("core.print(count(items))")
    );
    let diagnostic = rejected(&source);
    assert!(diagnostic.message.contains("expected a batch of `Other`"));
}

/// Statement arms use the expression form's coverage rules and lexical payloads.
#[test]
fn typed_result_blocks_reject_missing_arms_and_escaping_bindings() {
    let declarations = "error HeaderError { Short(required u64); BadMagic }\nfunc header returns Result<u64, HeaderError> { return error HeaderError.Short(4) }\n";
    for (body, message) in [
        (
            "header()\n        on ok then {}\n        on Short(_) then {}",
            "HeaderError.BadMagic",
        ),
        (
            "header()\n        on ok(value) then {}\n        on error then { core.print(value) }",
            "value",
        ),
        (
            "header()\n        on ok then {}\n        on Short(required) then {}\n        on error then {}\n    core.print(required)",
            "required",
        ),
    ] {
        let source =
            format!("{declarations}func main returns u64 {{\n    {body}\n    return 0\n}}");
        let diagnostic = rejected(&source);
        assert!(diagnostic.message.contains(message), "{diagnostic:?}");
    }
}

/// Two nominal families share one spelling. The finite cases also exercise
/// coverage, exact forwarding, and the absence of implicit scalar conversion.
#[test]
fn typed_results_reject_unhandled_and_wrong_family_values() {
    let declarations = r#"error HeaderError { BadMagic; Short }
error Other { BadMagic }
func header returns Result<u64, HeaderError> { return error HeaderError.BadMagic }
"#;
    let cases = [
        (
            "let code = header()\n    core.print(code)",
            "handle the saved `Result`",
        ),
        ("var code = header()", "with an immutable `let`"),
        ("header()", "save the call with `let`"),
        (
            "let code = header()\n    code\n        on ok then {}\n        on Other.BadMagic then {}\n        on error then {}",
            "expected error family `HeaderError`",
        ),
        (
            "let code = header()\n        on ok(value) then value\n        on BadMagic then 1",
            "HeaderError.Short",
        ),
        (
            "let code = header()\n        on ok(value) then value\n        on Other.BadMagic then 1\n        on error then 2",
            "expected error family `HeaderError`",
        ),
        (
            "let code = header()\n        on BadMagic then 1\n        on error then 2",
            "requires an `on ok`",
        ),
        (
            "let code = header()\n        on ok then 0\n        on ok(value) then value\n        on error then 2",
            "duplicate `on ok` arm",
        ),
        (
            "let code = header()\n        on ok(_) then true\n        on error then 2",
            "all boolean result arms",
        ),
        (
            "let code = header()\n        on ok(value) then value\n        on error(reason) then reason",
            "typed error cannot be used as a u64",
        ),
    ];
    // Each fixture differs only in its source-level use of the same checked result.
    for (body, message) in cases {
        let source =
            format!("{declarations}func main returns u64 {{\n    {body}\n    return 0\n}}\n");
        let diagnostic = rejected(&source);
        assert!(diagnostic.message.contains(message), "{diagnostic:?}");
    }
    // Equal case spellings never permit forwarding across different result families.
    let source = format!(
        "{declarations}func forward returns Result<u64, Other> {{ return header() }}\nfunc main returns u64 {{ return 0 }}\n"
    );
    assert!(rejected(&source).message.contains("cannot forward Result"));
    for (output, body, message) in [
        (
            "Result<u64, Outer>",
            "let saved = header()\n    return saved",
            "cannot forward Result from error family `HeaderError` as `Outer`",
        ),
        (
            "Option<u64>",
            "let saved = header()\n    return saved",
            "must have the exact return type",
        ),
        (
            "Result<u64, HeaderError>",
            "{ let saved = header() }\n    return saved",
            "unknown or out-of-scope binding",
        ),
        (
            "Result<u64, HeaderError>",
            "let scalar = 1\n    return scalar",
            "a named result return requires a saved",
        ),
    ] {
        let source = format!(
            "{declarations}error Outer {{ from HeaderError as Header }}\nfunc forward returns {output} {{\n    {body}\n}}\nfunc main returns u64 {{ return 0 }}\n"
        );
        let diagnostic = rejected(&source);
        assert!(diagnostic.message.contains(message), "{diagnostic:?}");
    }
}

/// Declared union members and handled paths are closed and nominal. Reject
/// invalid graphs, implicit transitive lifts, and incomplete leaf handling.
#[test]
fn error_unions_check_declarations_lifting_and_exhaustiveness() {
    let declarations = r#"
error Outer { Local; from Middle as MiddleCase }
error Middle { from Inner as InnerCase }
error Inner { Bad(code u64); Empty }
func inner returns Result<u64, Inner> { return error Inner.Bad(10) }
func middle returns Result<u64, Middle> {
    let value = try inner()
    return ok value
}
"#;
    let outer = r#"func outer returns Result<u64, Outer> {
    let value = try middle()
    return ok value
}
"#;
    let complete = "on ok then 0\non Local then 1\non MiddleCase.InnerCase.Bad(code) then code\non Outer.MiddleCase.InnerCase.Empty then 2";
    let handle = |arms: &str| {
        let arms = arms.replace('\n', "\n        ");
        format!(
            "func main returns u64 {{\n    let value = outer()\n        {arms}\n    return 0\n}}"
        )
    };
    accepted(&format!("{declarations}{outer}{}", handle(complete)));
    // Wrong, missing, and duplicated paths must fail even if a later arm
    // could otherwise accept the tag. The duplicate uses another spelling.
    for (arms, expected) in [
        (
            complete.replace("MiddleCase.InnerCase.Bad(code)", "Bad(code)"),
            "unknown error case `Outer.Bad`",
        ),
        (
            complete.replace("Bad(code) then code", "Bad then 3"),
            "requires a payload binding",
        ),
        (
            complete.replace("on Outer.MiddleCase.InnerCase.Empty then 2", ""),
            "missing error case `Outer.MiddleCase.InnerCase.Empty`",
        ),
        (
            format!("{complete}\non Outer.MiddleCase.InnerCase.Bad(_) then 3"),
            "duplicate error case",
        ),
    ] {
        let diagnostic = rejected(&format!("{declarations}{outer}{}", handle(arms.as_str())));
        assert!(diagnostic.message.contains(expected), "{diagnostic:?}");
    }
    // An explicit try names one immediate route. Plain return still requires
    // the exact result type even when a child edge is present.
    for (body, expected) in [
        (
            "let value = try inner()\nreturn ok value",
            "`Inner` as `Outer` with `try`",
        ),
        (
            "return middle()",
            "cannot forward Result from error family `Middle` as `Outer`",
        ),
    ] {
        let source = format!(
            "{declarations}func outer returns Result<u64, Outer> {{\n{body}\n}}\nfunc main returns u64 {{ return 0 }}"
        );
        let diagnostic = rejected(&source);
        assert!(diagnostic.message.contains(expected), "{diagnostic:?}");
    }
    // Verify declaration checks on unused families, including the expanded
    // one-byte limit even when the parent has only two written members.
    let leaves = (0..255)
        .map(|index| format!("Case{index}"))
        .collect::<Vec<_>>()
        .join("; ");
    for (types, expected) in [
        (
            "error A { from Missing as X }".to_owned(),
            "unknown child error family `Missing`",
        ),
        (
            "error A { from B as X }\nerror B { from A as Y }".to_owned(),
            "cyclic error union",
        ),
        (
            "error A { from B as X; from B as Y }\nerror B { Failure }".to_owned(),
            "included more than once",
        ),
        (
            "error A { X; from B as X }\nerror B { Failure }".to_owned(),
            "duplicate or reserved error case or alias `X`",
        ),
        (
            format!("error A {{ First; from B as X }}\nerror B {{ {leaves} }}"),
            "1 through 255 cases after expanding children",
        ),
    ] {
        let diagnostic = rejected(&format!("{types}\nfunc main returns u64 {{ return 0 }}"));
        assert!(diagnostic.message.contains(expected), "{diagnostic:?}");
    }
}

/// Fixed-record typed append fixture with a payload and a second nominal family.
const APPEND_RESULT: &str = r#"
error Failure { Bad(value u64) }
error Other { Bad(value u64) }
struct Item { value u64 }
func item
    arg value u64 as readonly copy
    returns Result<Item, Failure>
{
    if value == 9 { return error Failure.Bad(value) }
    return ok Item:
        value = value
}
"#;

/// Insert append decisions in the lexical tail of an acquired item batch.
/// Tests use the same source and checked representation as a CLI build.
fn append_fixture(tail: &str) -> String {
    format!(
        "{APPEND_RESULT}func main returns u64 {{\n    let region = core.region(64) else {{ return 0 }}\n    let items = core.batch(region, Item)\n{tail}\n}}"
    )
}

/// Build source cases around a live free region and a nominal typed constructor.
fn construct_fixture(tail: &str) -> String {
    format!(
        "{APPEND_RESULT}func main returns u64 {{\n    let region = core.region(64) else {{ return 0 }}\n{tail}\n    return 0\n}}"
    )
}

/// Scoped result arms are exhaustive, carry only the active binding, and must
/// be block statements. Same-tail retry is legal after a rejected attempt.
#[test]
fn scoped_constructor_checks_its_outcome_syntax_and_coverage() {
    let source = construct_fixture(
        r#"    core.construct(region, item(9))
        on ok(value) then { core.print(value.value) }
        on Rejected Failure.Bad(id) then {
            core.construct(region, item(3))
                on ok(_) then { core.print(id) }
                on error then {}
        }
        on Full then {}
        on SizeOverflow then {}"#,
    );
    accepted(&source);
    // Each source change leaves one independent syntax or coverage error.
    for (old, new, message) in [
        ("on SizeOverflow then {}", "", "SizeOverflow"),
        ("on Full then {}", "on ok then {}", "duplicate"),
        ("Failure.Bad(id)", "Other.Bad(id)", "Failure"),
        (
            "on Full then {}",
            "on error(reason) then {}",
            "does not bind a value",
        ),
        ("on Full then {}", "on error then {}", "last arm"),
        ("on Full then {}", "on Full then 0", "expected"),
    ] {
        let diagnostic = rejected(&source.replace(old, new));
        assert!(diagnostic.message.contains(message), "{diagnostic:?}");
    }
    let unhandled = rejected(&construct_fixture("    core.construct(region, item(0))"));
    assert!(
        unhandled.message.contains("outcome blocks"),
        "{unhandled:?}"
    );
}

/// Check the success root at the real source boundary, including borrowed
/// byte projections, outer alias rebinding, and two forms of tail competition.
#[test]
fn scoped_constructor_reference_cannot_escape_or_share_its_tail() {
    let statement = r#"    core.construct(region, item(0))
        on ok(value) then { core.print(value.value) }
        on error then {}"#;
    // Each invalid path is checked even when its arm would be unreachable.
    for (tail, expected) in [
        (format!("{statement}\n    core.print(value.value)"), "value"),
        (
            statement.replace(
                "on error then {}",
                "on error then { core.print(value.value) }",
            ),
            "value",
        ),
        (
            statement.replace("core.print(value.value)", "value.value = 1"),
            "readonly",
        ),
        (
            statement.replace(
                "core.print(value.value)",
                "let alias = core.alias_readwrite(value)",
            ),
            "readonly",
        ),
        (
            format!("    let items = core.batch(region, Item)\n{statement}"),
            "live batch owner",
        ),
        (
            statement.replace(
                "on ok(value) then { core.print(value.value) }",
                "on ok then { let items = core.batch(region, Item) }",
            ),
            "live scoped record owner",
        ),
        (
            format!(
                "    let outer = core.region(64) else {{ return 0 }}\n    let items = core.batch(outer, Item)\n    let placed = core.append(items, item(0))\n        on ok then 0\n        on error then 1\n    var saved = core.borrow_readonly(items, 0)\n{}",
                statement.replace("core.print(value.value)", "saved = value")
            ),
            "outlive",
        ),
    ] {
        let diagnostic = rejected(&construct_fixture(&tail));
        assert!(diagnostic.message.contains(expected), "{diagnostic:?}");
    }
    // A synchronous projection is supported, but the scoped owner cannot
    // supply a borrowed Bytes operand after its success scope has ended.
    let bytes = r#"
struct Payload { bytes Bytes }
error Broken { Invalid }
func payload returns Result<Payload, Broken> {
    return ok Payload:
        bytes = core.bytes(1, 42)
}
func first arg bytes Bytes as readonly reference returns u64 { return bytes[0] }
func main returns u64 {
    let memory = core.region(64) else { return 0 }
    core.construct(memory, payload())
        on ok(value) then { core.print(first(value.bytes)) }
        on error then {}
    return 0
}
"#;
    accepted(bytes);
    let out_of_scope = bytes.replace(
        "    return 0\n}",
        "    core.print(first(value.bytes))\n    return 0\n}",
    );
    let diagnostic = rejected(&out_of_scope);
    assert!(diagnostic.message.contains("value"), "{diagnostic:?}");
}

/// One repeated source site selects its owner while a named construction
/// continues to use its separate region. A contextual word is also an
/// ordinary function name and may appear alongside a function effect clause.
#[test]
fn source_scoped_placement_checks_contextual_syntax_and_continuation() {
    let source = format!(
        "{APPEND_RESULT}{}",
        r#"
func with returns u64 { return 64 }
func budget with noalloc returns u64 { return with() }
func main returns u64 {
    let other = core.region(64) else { return 0 }
    var last = 0
    with core.region(budget()) {
        for index in core.range(2) {
            core.construct(item(index))
                on ok(value) then { last = value.value }
                on error then {}
        }
        core.construct(other, item(3))
            on ok(value) then { core.print(value.value) }
            on error then {}
    } else {
        last = 1
    }
    core.print(last)
    return 0
}
"#,
    );
    accepted(&source);
}

/// Automatic construction needs exactly one eligible site under the
/// acquired scope. Invalid paths cannot borrow a region through lexical
/// nesting or fall through from a separate recovery arm.
#[test]
fn source_scoped_placement_rejects_missing_extra_and_misplaced_sites() {
    let site = "core.construct(item(0))\n    on ok then {}\n    on error then {}";
    // Keep the constructor and handlers fixed while varying the scope path.
    let cases = [
        (
            site.to_owned(),
            "automatic core.construct requires an eligible path",
            "core.construct",
        ),
        (
            "with core.region(64) {} else {}".to_owned(),
            "requires one automatic core.construct",
            "with core.region",
        ),
        (
            format!("with core.region(64) {{\n{site}\n{site}\n}} else {{}}"),
            "only one automatic core.construct",
            "core.construct",
        ),
        (
            format!("with core.region(64) {{ if 0 == 1 {{\n{site}\n}} }} else {{}}"),
            "automatic core.construct requires an eligible path",
            "core.construct",
        ),
        (
            format!("with core.region(64) {{ loop {{\n{site}\n}} }} else {{}}"),
            "automatic core.construct requires an eligible path",
            "core.construct",
        ),
        (
            format!("with core.region(64) {{\n{site}\n}} else {{\n{site}\n}}"),
            "automatic core.construct requires an eligible path",
            "core.construct",
        ),
        (
            format!("{{ with core.region(64) {{\n{site}\n}} else {{}} }}"),
            "only directly in experimental main",
            "with core.region",
        ),
        (
            format!(
                "with core.region(64) {{\n{site}\n}} else {{}}\nwith core.region(64) {{\n{site}\n}} else {{}}"
            ),
            "only one with core.region",
            "with core.region",
        ),
    ];
    // Require each error at the construct or owner that caused it.
    for (body, message, snippet) in cases {
        let source = format!("{APPEND_RESULT}func main returns u64 {{\n{body}\nreturn 0\n}}");
        let diagnostic = rejected(&source);
        assert!(
            diagnostic.message.contains(message),
            "{}",
            diagnostic.render("scope.holo", &source),
        );
        assert!(
            source[diagnostic.span.start..diagnostic.span.end].contains(snippet),
            "{}",
            diagnostic.render("scope.holo", &source),
        );
    }
}

/// The owner contributes two lexical branches. Neither success locals nor
/// references borrowed from the constructed record may escape them.
#[test]
fn source_scoped_placement_keeps_bindings_in_their_lexical_arms() {
    let main = r#"
func main returns u64 {
    let other = core.region(64) else { return 0 }
    let items = core.batch(other, Item)
    let initial = core.append(items, item(1))
        on ok then 0
        on error then 1
    core.assert(initial == 0)
    var saved = core.borrow_readonly(items, 0)
    with core.region(64) {
        let local = 3
        core.construct(item(local))
            on ok(value) then { core.print(value.value) }
            on error then {}
    } else {}
    core.print(saved.value)
    return 0
}
"#;
    let source = format!("{APPEND_RESULT}{main}");
    accepted(&source);
    // Check the ordinary lexical diagnostics alongside owner lifetime rejection.
    for (old, new, expected) in [
        ("} else {}", "} else { core.print(local) }", "local"),
        ("core.print(saved.value)", "core.print(local)", "local"),
        ("core.print(value.value)", "saved = value", "outlive"),
        ("core.print(value.value)", "value.value = 1", "readonly"),
        (
            "on error then {}",
            "on error then { core.print(value.value) }",
            "value",
        ),
    ] {
        let invalid = source.replace(old, new);
        let diagnostic = rejected(&invalid);
        assert!(
            diagnostic.message.contains(expected),
            "{}",
            diagnostic.render("scope.holo", &invalid),
        );
    }
}

/// The constructor's family is preserved, append success has unit shape,
/// and the checker requires all recoverable alternatives or a fallback.
#[test]
fn typed_append_requires_exhaustive_and_nominal_arms() {
    let source = append_fixture(
        "    let result = core.append(items, item(9))\n        on ok then 0\n        on Rejected Failure.Bad(value) then value\n        on Full then 1\n        on SizeOverflow then 2\n    core.assert(result == 9)\n    return 0",
    );
    accepted(&source);
    // An immediate append return keeps the existing cleanup-aware zero exit.
    accepted(&append_fixture(
        "    return core.append(items, item(9))\n        on ok then 0\n        on error then 0",
    ));
    // Each change isolates one shape, coverage, or entry restriction.
    for (old, new, expected) in [
        (
            "on ok then",
            "on ok(value) then",
            "append success has no payload",
        ),
        (
            "Failure.Bad(value) then",
            "Other.Bad(value) then",
            "expected error family",
        ),
        (
            "Failure.Bad(value) then",
            "Failure.Bad then",
            "requires a payload binding",
        ),
        ("\n        on Full then 1", "", "missing append Full arm"),
        (
            "on Full then",
            "on SizeOverflow then",
            "duplicate append SizeOverflow arm",
        ),
        (
            "let result",
            "var result",
            "typed append requires an entry `let`",
        ),
    ] {
        let modified = source.replace(old, new);
        let diagnostic = rejected(&modified);
        assert!(diagnostic.message.contains(expected), "{diagnostic:?}");
    }
    // A payload cannot escape its selected arm even though all arms yield u64.
    let escaped = source.replace("core.assert(result == 9)", "core.print(value)");
    assert!(
        rejected(&escaped)
            .message
            .contains("unknown or out-of-scope binding")
    );
    // Typed results cannot be erased to numeric statuses by omitting the handler.
    let unhandled = append_fixture("    let status = core.append(items, item(0))\n    return 0");
    assert!(
        rejected(&unhandled)
            .message
            .contains("requires immediate `on` arms")
    );
    // Main's direct return syntax remains limited to successful process exit.
    let nonzero = append_fixture(
        "    return core.append(items, item(9))\n        on ok then 0\n        on error then 1",
    );
    assert!(
        rejected(&nonzero)
            .message
            .contains("literal zero in every arm")
    );
}

/// Typed nested construction writes through its parent's destination. A child
/// with a byte tail still needs explicit propagation and exact family identity.
#[test]
fn typed_nested_append_preserves_the_destination_and_family() {
    let parent = "struct Parent { item Item }\nfunc parent returns Result<Parent, Failure> {\n    return ok Parent:\n        item = try item(1)\n}\n";
    let entry = "func main returns u64 { return 0 }";
    accepted(&format!("{APPEND_RESULT}{parent}{entry}"));
    // The child's result must keep both the expected schema and exact family.
    let wrong_family = parent.replace("Result<Parent, Failure>", "Result<Parent, Other>");
    assert!(
        rejected(&format!("{APPEND_RESULT}{wrong_family}{entry}"))
            .message
            .contains("expected `Result<Item,Other>`")
    );
    // Explicit try distinguishes propagation from a plain constructor call.
    let implicit = parent.replace("try item", "item");
    assert!(
        rejected(&format!("{APPEND_RESULT}{implicit}{entry}"))
            .message
            .contains("typed nested record fields require")
    );
    // The typed producer can initialize owned bytes through the normal path.
    let dynamic = "struct Dynamic { tail Bytes }\nfunc dynamic returns Result<Dynamic, Failure> {\n    return ok Dynamic:\n        tail = core.bytes(2, 0)\n}\n";
    accepted(&format!("{APPEND_RESULT}{dynamic}{entry}"));
}

/// Exact typed and numeric returns lower to one destination edge. The
/// outgoing scalar and readonly byte actuals retain their written order.
#[test]
fn whole_constructor_results_forward_in_the_same_destination() {
    use holo::ir::{Argument, Build, BytesSource, Expr, Op, ResultValue};

    let source = format!(
        "{APPEND_RESULT}\n{}",
        r#"
func forward_item arg value u64 as readonly copy returns Result<Item, Failure> {
    return item(value)
}
struct BytesRow { wire Bytes }
func bytes_row arg seed u64 as readonly copy arg input Bytes as readonly reference returns BytesRow {
    return BytesRow { wire: core.bytes(core.len(input), core.add_wrapping(seed, input[core.index()])) }
}
func forward_bytes arg input Bytes as readonly reference arg seed u64 as readonly copy returns BytesRow {
    return bytes_row(seed, input)
}
func scalar_value arg value u64 as readonly copy returns Result<u64, Failure> { return ok value }
func forward_scalar arg value u64 as readonly copy returns Result<u64, Failure> { return scalar_value(value) }
func main returns u64 { return 0 }
"#
    );
    let program = accepted(&source);
    let producers = &program.program().producers;
    let typed = producers
        .iter()
        .find(|item| item.name == "forward_item")
        .unwrap();
    assert!(matches!(
        typed.body.as_slice(),
        [Build::Forward { producer, args }]
            if producer == "item" && matches!(args.as_slice(), [Argument::Scalar(Expr::Param(0))])
    ));
    let numeric = producers
        .iter()
        .find(|item| item.name == "forward_bytes")
        .unwrap();
    assert!(matches!(
        numeric.body.as_slice(),
        [Build::Forward { producer, args }] if producer == "bytes_row"
            && matches!(args.as_slice(), [Argument::Scalar(Expr::Param(1)), Argument::Bytes(BytesSource::Param(0))])
    ));
    // Scalar exact forwarding still uses the separate ordinary result path.
    assert!(program.program().body.iter().any(|operation| matches!(
        operation,
        Op::Function(function) if function.name == "forward_scalar"
            && matches!(function.body.as_slice(), [Op::ReturnResult(ResultValue::Call { function, .. })] if function == "scalar_value")
    )));
}

/// Whole-result forwarding does not change nominal identity, add a child
/// lift, support producer prelude effects, or bypass the bounded call graph.
#[test]
fn whole_constructor_returns_require_an_exact_single_acyclic_call() {
    let types = "struct OtherItem { value u64 }\nerror Parent { from Failure as Child }\n";
    let entry = "\nfunc main returns u64 { return 0 }\n";
    // Every invalid wrapper is checked even though entry does not call it.
    for (wrapper, expected) in [
        (
            "func wrap returns Result<OtherItem, Failure> { return item(1) }",
            "expected exact result `Result<OtherItem,Failure>`",
        ),
        (
            "func wrap returns Result<Item, Other> { return item(1) }",
            "expected exact result `Result<Item,Other>`",
        ),
        (
            "func wrap returns Result<Item, Parent> { return item(1) }",
            "expected exact result `Result<Item,Parent>`",
        ),
        (
            "func wrap returns Item { return item(1) }",
            "expected exact result `Item`",
        ),
        (
            "func wrap returns Result<Item, Failure> {\n    if 0 == 1 { return error Failure.Bad(0) }\n    return item(1)\n}",
            "must be the function's only statement",
        ),
        (
            "func wrap returns Result<Item, Failure> { return wrap() }",
            "recursive function graph",
        ),
        (
            "func wrap returns Result<Item, Failure> { return nested() }\nfunc nested returns Result<Item, Failure> { return wrap() }",
            "recursive function graph",
        ),
        (
            "func wrap returns Result<Item, Failure> { return item() }",
            "expects 1 arguments",
        ),
        (
            "func wrap returns Result<Item, Failure> { return ok item(1) }",
            "requires `return ok Record:`",
        ),
    ] {
        let error = rejected(&format!("{APPEND_RESULT}{types}{wrapper}{entry}"));
        assert!(error.message.contains(expected), "{error:?}");
    }
}

/// Byte-bearing success does not allow returning a borrow or prebuilt record.
/// Shape and lifetime diagnostics remain source-located through this path.
#[test]
fn typed_byte_producer_requires_owned_initialized_output() {
    let source = include_str!("../../fixtures/typed_frame.holo");
    accepted(source);
    // Replacing explicit initialization with a borrow would expose the input
    // rather than constructing the field at the receiving destination.
    let borrowed = source.replace(
        "core.bytes(input[2], input[core.add_wrapping(3, core.index())])",
        "input",
    );
    assert!(
        rejected(&borrowed)
            .message
            .contains("a Bytes field requires")
    );
    // Error payloads stay in the copied u64 lane after the input borrow ends.
    let escaped = source.replace("FrameError.Truncated(3)", "FrameError.Truncated(input)");
    assert!(
        rejected(&escaped)
            .message
            .contains("a readonly Bytes input requires")
    );
    // Passing an enclosing record cannot silently become a byte descriptor.
    let mismatched = source.replace("receive(1, short.data)", "receive(1, short)");
    assert!(rejected(&mismatched).message.contains("Bytes field"));
}

/// Payload presence and type belong to the declared case, and a binding never
/// escapes its selected arm. No native build is needed for these rejections.
#[test]
fn error_payloads_require_the_declared_shape_and_lexical_binding() {
    let family = "error FrameError { Truncated(required u64); BadMagic }";
    let helper =
        "func read returns Result<u64, FrameError> { return error FrameError.Truncated(5) }";
    let main = "func main returns u64 { return 0 }";
    // Changing only the declaration or return edge makes the wrong shape observable.
    for (declaration, returned, expected) in [
        (
            "error FrameError { Truncated(required Bytes) }",
            "FrameError.Truncated(1)",
            "one named `u64` field",
        ),
        (family, "FrameError.Truncated", "requires a `u64` payload"),
        (family, "FrameError.BadMagic(1)", "has no payload"),
        (family, "FrameError.Truncated(false)", "boolean values"),
    ] {
        let source = format!(
            "{declaration}\nfunc read returns Result<u64, FrameError> {{ return error {returned} }}\n{main}"
        );
        let diagnostic = rejected(&source);
        assert!(diagnostic.message.contains(expected), "{diagnostic:?}");
    }
    // A payload case needs a binding or explicit discard. Only its arm can use the name.
    for (pattern, arm, subsequent, expected) in [
        ("Truncated", "9", "0", "requires a payload binding"),
        ("BadMagic(value)", "value", "0", "has no payload to bind"),
        (
            "Truncated(required)",
            "required",
            "required",
            "unknown or out-of-scope binding",
        ),
    ] {
        let source = format!(
            "{family}\n{helper}\nfunc main returns u64 {{\n    let result = read()\n        on ok(value) then value\n        on {pattern} then {arm}\n        on error then 0\n    core.print({subsequent})\n    return 0\n}}"
        );
        let diagnostic = rejected(&source);
        assert!(diagnostic.message.contains(expected), "{diagnostic:?}");
    }
}

/// Propagation binds only an immutable success and rejects undeclared families.
#[test]
fn try_requires_an_immutable_binding_and_an_admitted_family() {
    let declarations = r#"error FrameError { Short }
error Other { Short }
func header returns Result<u64, FrameError> { return ok 18446744073709551615 }
func scalar returns u64 { return 1 }
func optional returns Option<u64> { return Some(7) }
"#;
    let source = |header: &str, binding: &str, result: &str| {
        format!("{declarations}{header} {{\n    {binding}\n    {result}\n}}\n")
    };
    let helper = "func probe returns Result<u64, FrameError>";
    let valid = source(helper, "let value = try header()", "return ok value");
    accepted(&format!("{valid}func main returns u64 {{ return 0 }}"));
    // These cases isolate the caller-family, call shape and binding restrictions.
    for (header, binding, result, expected) in [
        (
            "func main returns u64",
            "let value = try header()",
            "return 0",
            "requires a scalar typed-result function",
        ),
        (
            "func main returns u64",
            "let value = try scalar()",
            "return 0",
            "requires a scalar typed-result function",
        ),
        (
            "func probe returns Result<u64, Other>",
            "let value = try header()",
            "return ok value",
            "cannot forward Result from error family `FrameError` as `Other`",
        ),
        (
            helper,
            "var value = try header()",
            "return ok value",
            "bind a propagated scalar with `let",
        ),
        (
            helper,
            "let value = try scalar()",
            "return ok value",
            "expected a direct call returning a typed Result",
        ),
        (
            helper,
            "let value = try header()\n    value = 0",
            "return ok value",
            "cannot be reassigned",
        ),
        (
            helper,
            "let saved = optional()\n    let value = try saved",
            "return ok value",
            "requires a Result operand",
        ),
        (
            "func probe returns Option<u64>",
            "let saved = header()\n    let value = try saved",
            "return Some(value)",
            "requires a Result operand",
        ),
        (
            "func probe returns Result<u64, Other>",
            "let saved = header()\n    let value = try saved",
            "return ok value",
            "cannot forward Result from error family `FrameError` as `Other`",
        ),
        (
            helper,
            "let saved = scalar()\n    let value = try saved",
            "return ok value",
            "requires a saved `Result`",
        ),
        (
            helper,
            "{ let saved = header() }\n    let value = try saved",
            "return ok value",
            "out-of-scope binding",
        ),
        (
            "func main returns u64",
            "let saved = header()\n    let value = try saved",
            "return 0",
            "requires a scalar typed-result function",
        ),
    ] {
        let mut input = source(header, binding, result);
        if header != "func main returns u64" {
            input.push_str("func main returns u64 { return 0 }");
        }
        let diagnostic = rejected(&input);
        assert!(diagnostic.message.contains(expected), "{diagnostic:?}");
    }
}

#[test]
/// The maintained source example must enter the same checked path as structured IR.
fn source_example_parses_and_checks() {
    let source = include_str!("../../fixtures/region.holo");
    let program = accepted(source);
    assert_eq!(program.program().records.len(), 1);
    assert!(!program.program().producers.is_empty());
    assert!(!program.program().body.is_empty());
}

#[test]
/// Compatible readonly/writable aliases and retained references remain valid during growth.
fn aliases_and_growth_are_accepted_without_uniqueness() {
    accepted(&populated(
        "{\nlet first = core.borrow_readwrite(items, 0)\nlet reader = core.alias_readonly(first)\nlet original = core.address(first)\nlet status = core.append(items, row(8, 5))\ncore.assert(status == 0)\nfirst.id = 9\nfirst.data[1] = 77\ncore.assert(reader.id == 9)\ncore.assert(core.address(reader) == original)\n}\ncore.reset(items)",
    ));
}

#[test]
/// Reference bindings can be rebound when their new storage root has a sufficient lifetime.
fn same_root_reference_rebinding_is_accepted() {
    accepted(&populated(
        "let status = core.append(items, row(8, 2))\ncore.assert(status == 0)\nvar selected = core.borrow_readonly(items, 0)\nlet other = core.borrow_readonly(items, 1)\nselected = other\ncore.assert(selected.id == 8)",
    ));
}

#[test]
/// A readonly-path write is diagnosed at its source statement after IR checking.
fn readonly_write_has_precise_source_location() {
    let source = populated("let reader = core.borrow_readonly(items, 0)\nreader.id = 9");
    let program =
        parse_experimental(&source).expect("readonly writes are syntactically representable");
    let error = program
        .check()
        .expect_err("readonly write must fail checking");
    assert!(error.message.contains("readonly"));
    assert_eq!(&source[error.span.start..error.span.end], "reader.id = 9");
    let line = source[..error.span.start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    assert!(
        error
            .render("fixture.holo", &source)
            .contains(&format!("fixture.holo:{line}:"))
    );
}

#[test]
/// Alias creation cannot upgrade readonly access into writable access.
fn readonly_alias_cannot_be_upgraded() {
    let source = populated(
        "let reader = core.borrow_readonly(items, 0)\nlet writer = core.alias_readwrite(reader)",
    );
    let error = rejected(&source);
    assert!(error.message.contains("readonly"));
    assert!(source[error.span.start..error.span.end].contains("core.alias_readwrite"));
}

#[test]
/// Reset while a reference remains in lexical scope must fail at the reset itself.
fn reset_with_live_reference_is_rejected() {
    let source = populated("let reader = core.borrow_readonly(items, 0)\ncore.reset(items)");
    let program = parse_experimental(&source).expect("reset syntax lowers before root checking");
    let error = program
        .check()
        .expect_err("live reference must prevent reset");
    assert!(error.message.contains("reference"));
    assert_eq!(
        &source[error.span.start..error.span.end],
        "core.reset(items)"
    );
}

#[test]
/// A reference from an inner owner cannot be installed in an outer binding.
fn reference_assignment_cannot_escape_its_region() {
    let source = populated(
        "var selected = core.borrow_readonly(items, 0)\n{\nlet inner = core.region(128) else { return 0 }\nlet inner_items = core.batch(inner, Row)\nlet inner_status = core.append(inner_items, row(99, 1))\ncore.assert(inner_status == 0)\nlet inside = core.borrow_readonly(inner_items, 0)\nselected = inside\n}",
    );
    let program =
        parse_experimental(&source).expect("reference assignment reaches the root checker");
    let error = program
        .check()
        .expect_err("short-lived root must not escape");
    assert_eq!(
        &source[error.span.start..error.span.end],
        "selected = inside"
    );
    assert!(
        error.message.contains("lifetime")
            || error.message.contains("outlive")
            || error.message.contains("escape")
    );
}

#[test]
/// A name introduced inside explicit braces is unavailable after that source scope.
fn inner_bindings_do_not_leak_into_the_outer_scope() {
    let source =
        populated("{ let hidden = core.borrow_readonly(items, 0) }\ncore.print(hidden.id)");
    let error = rejected(&source);
    assert!(error.message.contains("hidden"));
}

#[test]
/// Unknown APIs, dynamic targets, and unsettled arithmetic never acquire silent fallbacks.
fn unavailable_operations_are_explicitly_rejected() {
    let cases = [
        ("core.mystery(1)", "mystery"),
        ("let value = missing(1)", "missing"),
        ("let value = 1 + 2", "arithmetic"),
        ("let value = core.index()", "index"),
        ("let value = core.add_wrapping(1)", "argument"),
    ];
    // Each case must fail independently, so one early error cannot hide another accepted fallback.
    for (source, fragment) in cases {
        let error = rejected(&populated(source));
        assert!(error.message.contains(fragment), "{}", error.message);
    }
}

#[test]
/// The initial entry boundary requires explicit recovery and supports only return zero.
fn acquisition_and_entry_limits_are_not_silently_weakened() {
    let sources = [
        "func main returns u64 { let memory = core.region(64); return 0 }",
        "func main returns u64 { let memory = core.region(64) else { core.print(1) }; return 0 }",
        "func main returns u64 { { let memory = core.region(64) else {}; return 0 } }",
        "func main returns u64 { return 1 }",
        "func main with noblock returns u64 { return 0 }",
        "func main arg value u64 as readonly copy returns u64 { return 0 }",
        "func main returns Bytes { return 0 }",
    ];
    // Explicit unsupported entry cases must not compile merely because the backend can return i32.
    for source in sources {
        rejected(source);
    }
}

#[test]
/// Nonterminating recovery runs with enclosing bindings and rejoins outside its block.
/// Empty handlers and conditional entry completion use the same checked region IR.
fn acquisition_recovery_may_finish_or_terminate_without_exposing_the_owner() {
    accepted(&populated(
        r#"
        var recovered = 0
        let reader = core.borrow_readonly(items, 0)
        {
            let inner = core.region(64) else {
                recovered = 1
                core.assert(reader.id == 7)
                if reader.data[0] == 1 { return 0 }
            }
            let inner_items = core.batch(inner, Row)
            let added = core.append(inner_items, row(9, 1))
            core.assert(added == 0)
        }
        core.print(recovered)
        core.assert(reader.id == 7)
        { let empty_handler = core.region(1) else {} }
        {
            // Before the new owner exists, the same spelling resolves outward.
            let memory = core.region(64) else {
                core.assert(core.used(memory) > 0)
            }
        }
        "#,
    ));
    accepted(include_str!("../../fixtures/recovery.holo"));
    accepted(&populated(
        r#"
        let writer = core.borrow_readwrite(items, 0)
        let reader = core.alias_readonly(writer)
        var expected = 7
        {
            let inner = core.region(64) else {
                writer.id = 9
                expected = 9
            }
        }
        // Readonly access observes a permitted mutation through its writable alias.
        core.assert(reader.id == expected)
        "#,
    ));
}

#[test]
/// A failed owner and its success-only tail do not exist in recovery.
/// Recovery-only and success-only names also cannot escape their lexical block.
fn acquisition_recovery_preserves_failed_owner_and_branch_scopes() {
    let cases = [
        (
            "{\nlet absent = core.region(64) else {\ncore.print(core.used(absent))\n}\n}",
            "absent",
        ),
        (
            "{\nlet inner = core.region(64) else {\ncore.print(success_only)\n}\nlet success_only = 1\n}",
            "success_only",
        ),
        (
            "{\nlet inner = core.region(64) else {\nlet hidden = 1\n}\ncore.print(hidden)\n}",
            "hidden",
        ),
        (
            "{\nlet inner = core.region(64) else {}\nlet hidden = 1\n}\ncore.print(hidden)",
            "hidden",
        ),
    ];
    // Each independent fixture must reject the unavailable name; accepting one
    // cannot be hidden by a different earlier scope error in a combined program.
    for (body, name) in cases {
        let source = populated(body);
        let diagnostic = rejected(&source);
        assert!(diagnostic.message.contains(name), "{}", diagnostic.message);
        assert!(source[diagnostic.span.start..diagnostic.span.end].contains(name));
    }
}

#[test]
/// Recovery cannot reset a referenced root or install a shorter-lived retry borrow.
/// Failure branches preserve the same alias/lifetime obligations as success code.
fn acquisition_recovery_cannot_invalidate_or_escape_enclosing_references() {
    let reset = populated(
        r#"
        let reader = core.borrow_readonly(items, 0)
        {
            let inner = core.region(64) else {
                core.reset(items)
            }
        }
        core.print(reader.id)
        "#,
    );
    let diagnostic = rejected(&reset);
    assert!(
        diagnostic.message.contains("reference"),
        "{}",
        diagnostic.message
    );
    assert_eq!(
        &reset[diagnostic.span.start..diagnostic.span.end],
        "core.reset(items)"
    );

    let escape = populated(
        r#"
        var selected = core.borrow_readwrite(items, 0)
        {
            let inner = core.region(64) else {
                let replacement = core.region(64) else { return 0 }
                let retries = core.batch(replacement, Row)
                let appended = core.append(retries, row(19, 1))
                core.assert(appended == 0)
                let inside = core.borrow_readwrite(retries, 0)
                selected = inside
            }
        }
        core.print(selected.id)
        "#,
    );
    let diagnostic = rejected(&escape);
    assert!(
        diagnostic.message.contains("lifetime")
            || diagnostic.message.contains("outlive")
            || diagnostic.message.contains("escape"),
        "{}",
        diagnostic.message
    );
    assert_eq!(
        &escape[diagnostic.span.start..diagnostic.span.end],
        "selected = inside"
    );
}

#[test]
/// Contract checking validates unused helper graphs and treats bump storage as allocation.
fn contracts_do_not_trust_unknown_calls_or_dynamic_construction() {
    let sources = [
        "func hidden with noalloc, noblock returns u64 { return unknown() }\nfunc main returns u64 { return 0 }",
        "struct Empty {}\nfunc empty with noalloc returns Empty { return Empty {} }\nfunc main returns u64 { return 0 }",
        "func hidden with imaginary returns u64 { return 0 }\nfunc main returns u64 { return 0 }",
        "func hidden with noalloc, noalloc returns u64 { return 0 }\nfunc main returns u64 { return 0 }",
    ];
    // All declarations are obligations, including definitions unreachable from main.
    for source in sources {
        rejected(source);
    }
}

#[test]
/// Pure transitive helpers and nested destination-producing calls check with explicit contracts.
fn nested_helpers_and_nested_record_literals_are_accepted() {
    accepted(
        r#"
// Closed scalar helpers retain explicit wrapping semantics and no runtime effects.
func double with noalloc, noblock arg value u64 as readonly copy returns u64 {
    return core.mul_wrapping(value, 2)
}
func next with noalloc, noblock arg value u64 as readonly copy returns u64 {
    return core.add_wrapping(double(value), 1)
}
// Inline nested records have no separate object ownership or allocation headers.
struct Child { value u64 }
struct Parent { child Child }
func parent with noblock arg value u64 as readonly copy returns Parent {
    return Parent { child: Child { value: next(value) } }
}
func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let items = core.batch(memory, Parent)
    let status = core.append(items, parent(3))
    core.assert(status == 0)
    let first = core.borrow_readonly(items, 0)
    core.assert(first.child.value == 7)
    return 0
}
"#,
    );
}

#[test]
/// Schema and constructor mistakes fail even when unused; source types are not guessed.
fn invalid_unused_definitions_are_rejected() {
    let definitions = [
        "struct Wrong { value u32 }",
        "struct Loop { next Loop }",
        "struct Item { value u64 value u64 }",
        "struct Item { value u64 }\nfunc item returns Item { return Item {} }",
        "struct Item { value u64 }\nfunc item returns Item { return Item { wrong: 1 } }",
        "func recursive returns u64 { return recursive() }",
        "func changes arg value u64 as readwrite copy returns u64 { return value }",
    ];
    // Resolve every declaration independently of reachability and reject incompatible checked forms.
    for definition in definitions {
        rejected(&format!(
            "{definition}\nfunc main returns u64 {{ return 0 }}"
        ));
    }
}

#[test]
/// Source immutability must remain checked even though the underlying IR has mutable scalar slots.
fn immutable_scalar_assignment_is_rejected() {
    let error = rejected("func main returns u64 { let value = 1; value = 2; return 0 }");
    assert!(error.message.contains("immutable") || error.message.contains("var"));
    accepted("func main returns u64 { var value = 1; value = 2; core.print(value); return 0 }");
}

#[test]
/// A reusable caller slot, narrowed prefix, and nested mutable calls reach checked IR.
fn ordinary_borrowed_buffer_example_checks() {
    let input = accepted(include_str!("../../fixtures/borrowed_buffer.holo"));
    assert!(
        input
            .program()
            .body
            .iter()
            .any(|op| matches!(op, holo::ir::Op::Function(_)))
    );
}

#[test]
fn boolean_returns_work_in_conditions_and_immutable_bindings() {
    let source = r#"
func matches_one with noalloc, noblock arg value u64 as readonly copy returns bool {
    return value == 1
}
func forwards arg value u64 as readonly copy returns bool {
    let matches = matches_one(value)
    return matches
}
func literal returns bool { return true }
func failed returns Option<u64> { return None }
func decision returns bool {
    let answered = failed()
        on Some(_) then true
        on None then false
    let saved = answered
    return saved
}
func handled returns bool {
    return failed()
        on Some(_) then false
        on None then true
}
func outer with noalloc, noblock returns u64 { return core.add_wrapping(inner(), 2) }
func inner returns u64 { return code() }
func code returns u64 {
    if not forwards(2) and forwards(1) or false { return 7 } else { return 0 }
}
func main returns u64 {
    let matches = forwards(1)
    let compared = outer() == 9 and not decision()
    let copied = matches
    core.assert(copied)
    core.assert(compared)
    core.assert(literal())
    core.assert(false or true and not false)
    core.assert(handled())
    if decision() { core.assert(false) }
    if forwards(2) { core.assert(false) }
    forwards(3)
    return 0
}
"#;
    accepted(source);
}

#[test]
fn boolean_returns_reject_integer_conversion_and_rebinding() {
    let predicate = "func predicate returns bool { return true }";
    for (source, message) in [
        ("func invalid returns bool { return 1 }", "expected a bool"),
        (
            "func invalid returns bool { return false and 1 }",
            "expected a bool",
        ),
        (
            "func invalid returns bool { return 1 or true }",
            "expected a bool",
        ),
        (
            "func invalid returns bool { return not 1 }",
            "expected a bool",
        ),
        (
            "func invalid returns u64 { return predicate() }",
            "used as a u64",
        ),
        (
            "func invalid returns bool { if true { return false } }",
            "every normal path",
        ),
        (
            "func invalid returns bool { return invalid() }",
            "recursive",
        ),
        (
            "func invalid returns bool { return predicate(1) }",
            "expected 0 arguments",
        ),
        (
            "error Failure { Missing }\nfunc invalid returns Result<bool, Failure> { return ok true }",
            "unsupported or unknown return type",
        ),
    ] {
        let source = format!("{predicate}\n{source}\nfunc main returns u64 {{ return 0 }}");
        assert!(rejected(&source).message.contains(message), "{source}");
    }
    for (body, message) in [
        ("core.print(true or false)", "used as a u64"),
        ("core.print(predicate())", "used as a u64"),
        (
            "let result = predicate(); core.print(result)",
            "used as a u64",
        ),
        (
            "let result = 1; if result {}",
            "expected a comparison or boolean condition",
        ),
        ("var result = predicate()", "bind a bool with `let`"),
    ] {
        let source = format!("{predicate}\nfunc main returns u64 {{ {body}; return 0 }}");
        assert!(rejected(&source).message.contains(message), "{source}");
    }
}

#[test]
fn boolean_parameters_work_in_ordinary_helpers_and_forwarding_calls() {
    accepted(
        r#"
error Choice { Missing }
func forwarded returns u64 { return core.add_wrapping(select(1 < 2), 1) }
func with_local returns u64 {
    let selected = core.add_wrapping(select(false), 1)
    return selected
}
func with_branch returns u64 {
    if select(true) == 7 { return 1 }
    return 0
}
func select with noalloc, noblock arg flag bool as readonly copy returns u64 {
    if flag { return 7 }
    return 3
}
func negate arg flag bool as readonly copy returns bool { return not flag }
func optional arg flag bool as readonly copy returns Option<u64> {
    if flag { return Some(7) }
    return None
}
func tagged arg flag bool as readonly copy returns Result<u64, Choice> {
    if flag { return ok 7 }
    return error Choice.Missing
}
func main returns u64 {
    let checked = optional(negate(false))
        on Some(_) then true
        on None then false
    let value = tagged(checked)
        on ok(value) then value
        on Missing then 0
    core.assert(value == 7)
    core.assert(forwarded() == 8)
    core.assert(with_local() == 4)
    core.assert(with_branch() == 1)
    core.assert(negate(false or 2 < 1))
    return 0
}
"#,
    );
}

#[test]
fn boolean_parameters_require_readonly_copy_and_keep_their_type() {
    for (definition, entry, expected) in [
        (
            "func invalid arg flag bool as readwrite copy returns bool { return flag }",
            "",
            "bool parameters require `as readonly copy`",
        ),
        (
            "func invalid arg flag bool as readonly reference returns bool { return flag }",
            "",
            "bool parameters require `as readonly copy`",
        ),
        (
            "struct Row { value u64 }\nfunc invalid arg flag bool as readonly copy returns Row { return Row { value: 1 } }",
            "",
            "bool parameters are supported only by ordinary functions",
        ),
        (
            "func invalid arg flag bool as readonly copy returns u64 { return flag }",
            "",
            "bool binding cannot be used as a u64",
        ),
        (
            "func invalid arg flag bool as readonly copy returns bool { flag = true; return flag }",
            "",
            "binding cannot be reassigned",
        ),
        (
            "func negate arg flag bool as readonly copy returns bool { return not flag }",
            "core.assert(negate(1))",
            "expected a bool expression",
        ),
        (
            "func select arg flag bool as readonly copy returns u64 { if flag { return 7 } return 3 }\nstruct Row { value u64 }\nfunc invalid returns Row { return Row { value: select(true) } }",
            "",
            "cannot be called from a pure helper or destination constructor",
        ),
    ] {
        let source = format!("{definition}\nfunc main returns u64 {{ {entry}\nreturn 0 }}");
        let error = rejected(&source);
        assert!(error.message.contains(expected), "{source}: {error:?}");
    }
}

#[test]
/// Unsupported effects and borrowed escapes reject even in unused definitions.
fn ordinary_borrowed_functions_reject_invalid_modes_and_effects() {
    let definitions = [
        (
            "func invalid arg input Bytes as readonly reference returns u64 { input[0] = 1; return 0 }",
            "readonly byte parameter",
        ),
        (
            "func invalid arg input Bytes as readonly reference returns u64 { return input }",
            "escape",
        ),
        (
            "func invalid arg input Bytes as readonly reference returns u64 { core.assert(true) }",
            "every normal path",
        ),
        (
            "func invalid with noalloc arg input Bytes as readonly reference returns u64 { return unknown(input) }",
            "unknown scalar helper",
        ),
        (
            "func invalid with noblock arg input Bytes as readonly reference returns u64 { core.print(1); return 0 }",
            "unavailable",
        ),
        (
            "func invalid arg input Bytes as readonly reference returns u64 { return invalid(input) }",
            "recursive",
        ),
        (
            "struct State { bytes Bytes }\nfunc invalid arg state State as readwrite reference returns u64 { return 0 }",
            "fixed",
        ),
    ];
    // Exact diagnostic fragments distinguish the intended rule from unrelated errors.
    for (definition, message) in definitions {
        let error = rejected(&format!(
            "{definition}\nfunc main returns u64 {{ return 0 }}"
        ));
        assert!(
            error.message.contains(message),
            "{definition}: {}",
            error.message
        );
    }
}

#[test]
/// A readonly state actual cannot receive writable authority through a call.
fn ordinary_call_requires_writable_state() {
    let source = include_str!("../../fixtures/borrowed_buffer.holo").replace(
        "consume(state, core.prefix(buffer.wire, 2))",
        "consume(observer, core.prefix(buffer.wire, 2))",
    );
    let error = rejected(&source);
    assert!(error.message.contains("live readwrite reference"));
    assert_eq!(&source[error.span.start..error.span.end], "observer");
}

#[test]
/// The checker's function-local operation index points at the rejected store.
fn ordinary_readonly_alias_diagnostic_names_the_helper_store() {
    let source = r#"
struct State { value u64 }
func invalid arg state State as readwrite reference returns u64 {
    let reader = core.alias_readonly(state)
    reader.value = 3
    return 0
}
func main returns u64 { return 0 }
"#;
    let error = rejected(source);
    assert!(error.message.contains("readonly"));
    assert_eq!(
        &source[error.span.start..error.span.end],
        "reader.value = 3"
    );
}

/// Exercise received-view invalidation through aliases and names in sibling scopes.
/// Each mutation rejects at its source operation, independent of native execution.
#[test]
fn hosted_views_reject_same_path_refill_reset_and_escape() {
    let nested = "refill alias.data, 1 as again {} else code {}";
    // Keep the origin live even when an inner scalar hides the received name.
    for (body, other, following, message) in [
        (nested, "", "", "while its received view is live"),
        (
            "{ let chunk = 7; refill alias.data, 1 as again {} else code {} }",
            "",
            "",
            "while its received view is live",
        ),
        (
            "alias = row; refill alias.data, 1 as again {} else code {}",
            "",
            "",
            "while its received view is live",
        ),
        ("core.reset(items)", "", "", "while a reference is live"),
        ("let saved = chunk", "", "", "escape"),
        ("", "core.print(chunk)", "", "out-of-scope binding"),
        ("", "", "core.print(status)", "out-of-scope binding"),
        ("", "status = 0", "", "cannot be reassigned"),
    ] {
        let source = populated(&format!(
            "let row = core.borrow_readwrite(items, 0)\nvar alias = core.alias_readwrite(row)\nrefill row.data, 1 as chunk {{ {body} }} else status {{ {other} }}\n{following}"
        ));
        let error = rejected(&source);
        assert!(
            error.message.contains(message),
            "{source}: {}",
            error.message
        );
    }
}

/// A second receive after the first lexical view ends is allowed, as is lending
/// the exact dynamic extent to an ordinary readonly byte formal.
#[test]
fn hosted_views_forward_and_end_at_the_branch_boundary() {
    let entry = populated(
        r#"let row = core.borrow_readwrite(items, 0)
        loop {
            refill row.data, 2 as chunk {
                core.print(inspect(chunk))
            } else status {
                if status == 0 { return 0 }
                let ready = core.wait_input()
                core.print(ready)
            }
            refill row.data, 2 as chunk { core.print(inspect(chunk)) } else status { return 0 }
        }"#,
    );
    accepted(&format!(
        "func inspect arg bytes Bytes as readonly reference returns u64 {{ return core.len(bytes) }}\n{entry}"
    ));
}

/// Entry lowering can lend a fresh refill name to append. The same spelling
/// cannot widen, recreate, or keep that capability after the data branch.
#[test]
fn hosted_views_lend_to_producers_only_in_the_data_branch() {
    let copy = "let copied = core.append(items, copy_chunk(chunk))";
    let narrowed = "let copied = core.append(items, copy_chunk(core.prefix(chunk, 1)))";
    let named =
        "let window = core.prefix(chunk, 1); let copied = core.append(items, copy_chunk(window))";
    let assemble = |data: &str, other: &str, after: &str| {
        let entry = populated(&format!(
            "let row = core.borrow_readwrite(items, 0)\nlet alias = core.alias_readwrite(row)\nrefill row.data, 2 as chunk {{ {data} }} else status {{ {other} }}\n{after}"
        ));
        format!(
            "func copy_chunk arg bytes Bytes as readonly reference returns Row {{\nreturn Row {{ id: core.len(bytes), data: core.bytes(core.len(bytes), bytes[core.index()]) }}\n}}\n{entry}"
        )
    };
    accepted(&assemble(copy, "", ""));
    accepted(&assemble(narrowed, "", ""));
    accepted(&assemble(named, "", ""));

    // Keep the surrounding producer and source owner fixed while varying the
    // attempted escape, widening, or invalidation of the refill capability.
    for (data, other, after, expected) in [
        ("", copy, "", "out-of-scope binding"),
        ("", "", copy, "out-of-scope binding"),
        ("", "", narrowed, "out-of-scope binding"),
        (
            named,
            "",
            "core.print(core.len(window))",
            "out-of-scope binding",
        ),
        (
            "let copied = core.append(items, copy_chunk(chunk)); refill alias.data, 1 as again {} else code {}",
            "",
            "",
            "while its received view is live",
        ),
    ] {
        let source = assemble(data, other, after);
        let error = rejected(&source);
        assert!(
            error.message.contains(expected),
            "{source}: {}",
            error.message
        );
    }
}

/// Distinct inline byte fields in the same batch can be refilled while the
/// first field's received view is still active.
#[test]
fn hosted_views_allow_refill_of_a_disjoint_field() {
    let source = populated(
        "let row = core.borrow_readwrite(items, 0)\nrefill row.data, 1 as chunk { refill row.spare, 1 as other {} else code {} } else status {}",
    )
    .replace("data Bytes\n", "data Bytes\n    spare Bytes\n")
    .replace("core.bytes(length, core.index()) }", "core.bytes(length, core.index()), spare: core.bytes(4, 0) }");
    accepted(&source);
}

/// Named refill destinations use their captured field and declared permission.
#[test]
fn hosted_refill_requires_a_writable_named_view() {
    let source = |body: &str| {
        populated(&format!(
            "let row = core.borrow_readwrite(items, 0)\nlet alias = core.alias_readwrite(row)\n{body}"
        ))
    };
    let writable = "let readwrite tail = core.subrange(alias.data, 1, 2)";
    let refill = "refill tail, 1 as chunk {} else status {}";
    accepted(&source(&format!("{writable}\n{refill}")));

    let readonly = source(&format!(
        "let tail = core.subrange(alias.data, 1, 2)\n{refill}"
    ));
    let error = rejected(&readonly);
    assert!(error.message.contains("byte view is readonly"));
    assert_eq!(&readonly[error.span.start..error.span.end], "tail");

    let nested = source(&format!(
        "{writable}\nrefill row.data, 1 as first {{ {refill} }} else first_status {{}}"
    ));
    let error = rejected(&nested);
    assert!(error.message.contains("while its received view is live"));
}

/// Readonly slot access and entry-only host effects remain checked.
#[test]
fn hosted_input_requires_writable_slot_and_entry() {
    let error = rejected(&populated(
        "let row = core.borrow_readonly(items, 0)\nrefill row.data, 1 as chunk {} else status {}",
    ));
    assert!(error.message.contains("readonly"), "{}", error.message);
    let error = rejected(
        "func invalid with noblock arg bytes Bytes as readonly reference returns u64 { let status = core.wait_input(); return status }\nfunc main returns u64 { return 0 }",
    );
    assert!(error.message.contains("unavailable"), "{}", error.message);
}
