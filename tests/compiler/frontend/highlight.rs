//! Lossless tooling regressions independent of strict parsing and native lowering.
//!
//! Coverage checks source partitioning on malformed UTF-8-adjacent text, deep
//! comments, lexical presentation hints, and strict compiler compatibility. Tests
//! consume borrowed ranges rather than substituting a second highlighting engine;
//! no runtime throughput or editor semantic-resolution guarantee follows.

use holo::highlight::{HighlightKind, highlight};
use holo::lex::{LexemeKind, TokenKind, lex, scan};

/// Return the highlighting category of the first lexeme exactly equal to `word`.
/// Panics if the fixture omits it; collected display text is borrowed, never rewritten.
fn category(source: &str, word: &str) -> HighlightKind {
    highlight(source)
        .find(|token| &source[token.span.start..token.span.end] == word)
        .unwrap_or_else(|| panic!("missing fixture word {word:?}"))
        .kind
}

/// Tool scanning partitions every source byte, including invalid Unicode scalars and EOF trivia.
#[test]
fn lossless_scan_partitions_malformed_source() {
    let source = "\tfunc 🥚 main\r\n// 注釈\r\na/* outer /* inner */\n*/ != 0xG \"tail\r";
    let mut end = 0;
    let mut rebuilt = String::new();
    // Adjacent, nonempty, UTF-8-aligned ranges ensure exact reconstruction without hidden gaps.
    for token in scan(source) {
        assert_eq!(token.span.start, end);
        assert!(token.span.end > token.span.start);
        let text = source.get(token.span.start..token.span.end).unwrap();
        rebuilt.push_str(text);
        end = token.span.end;
    }
    assert_eq!(end, source.len());
    assert_eq!(rebuilt, source);
    assert!(scan("").next().is_none());
    assert!(highlight("").next().is_none());
    let invalid: Vec<_> = scan(source)
        .filter(|token| token.kind == LexemeKind::Invalid)
        .map(|token| &source[token.span.start..token.span.end])
        .collect();
    assert_eq!(invalid, ["🥚", "\"tail", "\r"]);
}

/// Scanner recovery must continue after unsupported scalars without accepting them for compilation.
#[test]
fn bad_scalars_do_not_hide_later_highlights() {
    let source = "🥚\" unfinished\nfunc later returns u64 { return 0 }";
    assert_eq!(category(source, "🥚"), HighlightKind::Invalid);
    assert_eq!(category(source, "later"), HighlightKind::Function);
    assert_eq!(category(source, "u64"), HighlightKind::Type);
    let error = lex(source).unwrap_err();
    assert_eq!(error.span.start, 0);
    assert_eq!(error.span.end, "🥚".len());
}

/// Quoted text stays one highlighted token even with escaped quotes and Unicode.
#[test]
fn quoted_text_preserves_source_spelling() {
    let literal = r#""héllo, \"Holo\"""#;
    let source = format!("core.print({literal})");
    assert_eq!(category(&source, literal), HighlightKind::Text);
}

#[test]
fn byte_string_highlighting_includes_the_prefix_and_escapes() {
    let literal = r#"b"a\xFF""#;
    assert_eq!(
        category(&format!("let value = {literal}"), literal),
        HighlightKind::Text
    );
}

/// Iterative nested comments retain whole source extents and report an unfinished suffix visibly.
#[test]
fn comments_retain_nesting_unicode_and_original_line_boundaries() {
    let source = "/* outer\r\n /* 深 */\n*/tail // end";
    let tokens: Vec<_> = scan(source).collect();
    assert_eq!(tokens[0].kind, LexemeKind::BlockComment);
    assert_eq!(tokens[0].span.end, source.find("tail").unwrap());
    assert_eq!(category(source, "tail"), HighlightKind::Identifier);
    let strict: Vec<_> = lex(source)
        .unwrap()
        .into_iter()
        .filter(|token| token.kind == TokenKind::Newline)
        .collect();
    assert_eq!(strict.len(), 2);
    assert_eq!(&source[strict[0].span.start..strict[0].span.end], "\r\n");
    assert_eq!(&source[strict[1].span.start..strict[1].span.end], "\n");
    let incomplete = "/* /* 深 */\n";
    let unfinished = scan(incomplete).next().unwrap();
    assert_eq!(unfinished.kind, LexemeKind::Invalid);
    assert_eq!(unfinished.span.end, incomplete.len());
    assert!(
        lex(incomplete)
            .unwrap_err()
            .message
            .contains("unterminated")
    );
    let deep = format!("{}{}tail", "/*".repeat(8192), "*/".repeat(8192));
    assert_eq!(scan(&deep).count(), 2);
    assert_eq!(highlight(&deep).count(), 2);
}

/// Presentation hints separate functions, parameters, fields, contracts, and ordinary names.
#[test]
fn local_categories_are_useful_without_semantic_checking() {
    let source = "struct Entry { payload Bytes }\n\
        func make_entry\n with noblock\n arg id u64 as readonly copy\n returns Entry\n\
        { let result = core.bytes(4, 0xAF); result.payload; return Entry { payload: result } }";
    assert_eq!(category(source, "struct"), HighlightKind::Keyword);
    assert_eq!(category(source, "returns"), HighlightKind::Keyword);
    assert_eq!(category(source, "Entry"), HighlightKind::Type);
    assert_eq!(category(source, "make_entry"), HighlightKind::Function);
    assert_eq!(category(source, "noblock"), HighlightKind::Contract);
    assert_eq!(category(source, "id"), HighlightKind::Parameter);
    assert_eq!(category(source, "core"), HighlightKind::Builtin);
    assert_eq!(category(source, "bytes"), HighlightKind::Function);
    assert_eq!(category(source, "0xAF"), HighlightKind::Number);
    assert_eq!(category(source, "result"), HighlightKind::Identifier);
    let fields: Vec<_> = highlight(source)
        .filter(|token| &source[token.span.start..token.span.end] == "payload")
        .map(|token| token.kind)
        .collect();
    // Type-shaped field declarations, projections, and initializer labels get field hints.
    assert_eq!(
        fields,
        [
            HighlightKind::Property,
            HighlightKind::Property,
            HighlightKind::Property
        ]
    );
    assert_eq!(category("func return", "return"), HighlightKind::Function);
    assert_eq!(category("entry.func", "func"), HighlightKind::Property);
    assert_eq!(category("if (value)", "if"), HighlightKind::Keyword);
    assert_eq!(category("return (value)", "return"), HighlightKind::Keyword);
    assert_eq!(category("returns bool", "bool"), HighlightKind::Type);
}

#[test]
fn boolean_operators_preserve_contextual_function_hints() {
    let source = "return not (ready and found) or false";
    for operator in ["not", "and", "or"] {
        assert_eq!(category(source, operator), HighlightKind::Operator);
    }
    assert_eq!(
        category("return not(value)", "not"),
        HighlightKind::Function
    );
    assert_eq!(category("return or(value)", "or"), HighlightKind::Function);
    assert_eq!(
        category("return core.and(1, 2)", "and"),
        HighlightKind::Function
    );
}

/// Public visibility receives a keyword hint before function and record declarations.
/// Ordinary names, including the retired marker, retain their lexical categories.
#[test]
fn public_visibility_is_a_contextual_hint() {
    assert_eq!(
        category("public func helper", "public"),
        HighlightKind::Keyword
    );
    assert_eq!(
        category("public struct Header", "public"),
        HighlightKind::Keyword
    );
    assert_eq!(
        category("public\nstruct Header", "public"),
        HighlightKind::Identifier
    );
    assert_eq!(category("public(1)", "public"), HighlightKind::Function);
    assert_eq!(
        category("let public = 1", "public"),
        HighlightKind::Identifier
    );
    assert_eq!(
        category("export func helper", "export"),
        HighlightKind::Identifier
    );
}

/// Recognize the local library header shape while preserving ordinary uses of its marker.
#[test]
fn namespace_header_is_a_contextual_hint() {
    assert_eq!(
        category("// bytes\r\n  namespace holo.bytes", "namespace"),
        HighlightKind::Keyword
    );
    assert_eq!(
        category("namespace telemetry.frame", "namespace"),
        HighlightKind::Keyword
    );
    assert_eq!(
        category("namespace holo .", "namespace"),
        HighlightKind::Keyword
    );
    assert_eq!(
        category("namespace(value)", "namespace"),
        HighlightKind::Function
    );
    assert_eq!(
        category("func namespace", "namespace"),
        HighlightKind::Function
    );
    assert_eq!(
        category("let namespace = 1", "namespace"),
        HighlightKind::Identifier
    );
    assert_eq!(
        category("namespace\nholo.bytes", "namespace"),
        HighlightKind::Identifier
    );
    assert_eq!(
        category("entry.namespace", "namespace"),
        HighlightKind::Property
    );
}

/// Highlight a `using` header in isolation and after a namespace while leaving
/// local bindings and the retired directive spelling in their ordinary categories.
#[test]
fn using_header_is_a_contextual_hint() {
    assert_eq!(
        category("using holo.bytes", "using"),
        HighlightKind::Keyword
    );
    assert_eq!(
        category("namespace app.entry\nusing app.db", "using"),
        HighlightKind::Keyword
    );
    assert_eq!(
        category("let using = 1", "using"),
        HighlightKind::Identifier
    );
    assert_eq!(
        category("let import = 1", "import"),
        HighlightKind::Identifier
    );
    assert_eq!(
        category("import holo.bytes", "import"),
        HighlightKind::Identifier
    );
}

/// Recognize new error syntax while preserving ordinary names and calls.
#[test]
fn union_and_propagation_keywords_are_contextual_hints() {
    assert_eq!(
        category("public error Failure {}", "public"),
        HighlightKind::Keyword
    );
    assert_eq!(
        category("error Failure {}", "error"),
        HighlightKind::Keyword
    );
    assert_eq!(
        category("error Failure { from service.Failure as Service }", "from"),
        HighlightKind::Keyword
    );
    assert_eq!(
        category("error Failure { from }", "from"),
        HighlightKind::Identifier
    );
    assert_eq!(
        category("let item = try load()", "try"),
        HighlightKind::Keyword
    );
    assert_eq!(category("let item = try()", "try"), HighlightKind::Function);
}

/// Stable selector names round-trip uniquely and match dense table discriminants.
#[test]
fn style_categories_have_unique_dense_names() {
    // Dense indexing lets consumers resolve styles once rather than hash every source token.
    for (index, kind) in HighlightKind::ALL.into_iter().enumerate() {
        assert_eq!(kind as usize, index);
        assert_eq!(HighlightKind::from_name(kind.name()), Some(kind));
        assert_eq!(
            HighlightKind::ALL
                .iter()
                .filter(|other| other.name() == kind.name())
                .count(),
            1
        );
    }
    assert_eq!(HighlightKind::from_name("Keyword"), None);
    assert_eq!(HighlightKind::from_name("unknown"), None);
}

/// Exhaustive short inputs exercise recovery, pair lookahead, and Unicode-boundary invariants.
#[test]
fn short_inputs_always_partition_for_scan_and_highlight() {
    let alphabet = ["a", "0", "/", "*", "\n", "\r", "🥚", " ", "="];
    // Every combination through four scalars is independent of the compiler's grammar acceptance.
    for length in 0..=4 {
        for mut code in 0..alphabet.len().pow(length) {
            let mut source = String::new();
            // Base-N decoding exhausts the finite search space without recursive host calls.
            for _ in 0..length {
                source.push_str(alphabet[code % alphabet.len()]);
                code /= alphabet.len();
            }
            let scanned: Vec<_> = scan(&source).collect();
            let highlighted: Vec<_> = highlight(&source).collect();
            assert_eq!(scanned.len(), highlighted.len());
            let mut end = 0;
            // One style per lexical range must preserve the exact original coverage and order.
            for (raw, styled) in scanned.iter().zip(highlighted) {
                assert_eq!(raw.span, styled.span);
                assert_eq!(styled.span.start, end);
                assert!(styled.span.end > end);
                assert!(source.get(styled.span.start..styled.span.end).is_some());
                end = styled.span.end;
            }
            assert_eq!(end, source.len());
        }
    }
}
