//! Source/token boundary regressions independent of parser and native lowering.
//!
//! These tests pin byte ranges, u64 literal validation, comment/newline behavior,
//! and UTF-8-safe diagnostic rendering. They exercise compiler input processing;
//! no generated program or native timing claim follows from passing them.

use holo::lex::{TokenKind, lex};
use holo::source::{Diagnostic, Span};

/// Preferred multiline headers retain identifiers, punctuation, and EOF byte identity.
#[test]
fn headers_and_eof_have_exact_ranges() {
    let source = "func foo\n arg value u64 as readonly copy\n returns u64\n{ return value; }";
    let tokens = lex(source).unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Ident("func".into()));
    assert_eq!(tokens[0].span, Span { start: 0, end: 4 });
    assert_eq!(tokens[2].kind, TokenKind::Newline);
    assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof);
    assert_eq!(
        tokens.last().unwrap().span,
        Span {
            start: source.len(),
            end: source.len()
        }
    );
    // Every scanner range can be sliced directly from the original UTF-8 input.
    for token in tokens {
        assert!(source.get(token.span.start..token.span.end).is_some());
    }
}

/// CRLF consumes two original bytes but contributes one parser-visible boundary.
#[test]
fn crlf_and_line_comments_preserve_newlines() {
    let tokens = lex("a // 🥚 ignored\r\nb\n").unwrap();
    let kinds: Vec<_> = tokens.iter().map(|token| &token.kind).collect();
    assert_eq!(
        kinds,
        vec![
            &TokenKind::Ident("a".into()),
            &TokenKind::Newline,
            &TokenKind::Ident("b".into()),
            &TokenKind::Newline,
            &TokenKind::Eof
        ]
    );
    assert_eq!(tokens[1].span.end - tokens[1].span.start, 2);
}

/// Nested comments are iterative and preserve boundaries needed for statement parsing.
#[test]
fn nested_block_comments_keep_internal_boundaries() {
    let tokens = lex("a/* outer\n /* 嵌套 */\r\n*/b").unwrap();
    let kinds: Vec<_> = tokens.iter().map(|token| &token.kind).collect();
    assert_eq!(
        kinds,
        vec![
            &TokenKind::Ident("a".into()),
            &TokenKind::Newline,
            &TokenKind::Newline,
            &TokenKind::Ident("b".into()),
            &TokenKind::Eof
        ]
    );
    let error = lex("a /* unclosed /* nested */").unwrap_err();
    assert_eq!(error.span, Span { start: 2, end: 26 });
    assert!(error.message.contains("unterminated"));
}

/// Full u64 values are accepted without choosing source-language arithmetic overflow semantics.
#[test]
fn decimal_and_hex_literals_are_exact() {
    let tokens = lex("0 42 18446744073709551615 0xffffffffffffffff 0XAB").unwrap();
    let values: Vec<_> = tokens
        .into_iter()
        .filter_map(|token| match token.kind {
            TokenKind::Number(value) => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(values, vec![0, 42, u64::MAX, u64::MAX, 171]);
}

/// Overflow and malformed digit sequences fail as whole candidates instead of wrapping/splitting.
#[test]
fn malformed_and_overflow_literals_are_rejected() {
    // Each case has a distinct malformed boundary: prefix, digit, suffix, separator, or overflow.
    for source in [
        "0x",
        "0xG",
        "0x12oo",
        "12abc",
        "1_000",
        "18446744073709551616",
        "0x10000000000000000",
    ] {
        let error = lex(source).unwrap_err();
        assert_eq!(
            error.span,
            Span {
                start: 0,
                end: source.len()
            }
        );
    }
}

/// Multi-character comparisons precede their single-character punctuation prefixes.
#[test]
fn operators_are_not_split() {
    let kinds: Vec<_> = lex("== != <= >= = < > {}()[],.:;+-*/&^")
        .unwrap()
        .into_iter()
        .map(|token| token.kind)
        .collect();
    assert_eq!(
        &kinds[..7],
        &[
            TokenKind::EqualEqual,
            TokenKind::NotEqual,
            TokenKind::LessEqual,
            TokenKind::GreaterEqual,
            TokenKind::Symbol('='),
            TokenKind::Symbol('<'),
            TokenKind::Symbol('>')
        ]
    );
    assert_eq!(kinds.last(), Some(&TokenKind::Eof));
}

/// Unsupported Unicode reports one complete scalar and renders without byte slicing panics.
#[test]
fn unicode_errors_have_character_ranges() {
    let source = "ok\n\t🥚";
    let error = lex(source).unwrap_err();
    assert_eq!(error.span, Span { start: 4, end: 8 });
    let rendered = error.render("sample.holo", source);
    assert!(rendered.contains("sample.holo:2:2: error:"));
    assert!(rendered.contains("2 |     🥚"));
    assert!(rendered.contains("    ^"));
}

/// Public spans can be malformed; rendering remains safe at mid-scalar offsets and EOF.
#[test]
fn diagnostics_defend_against_invalid_spans() {
    let source = "é\nlast\r\n";
    let middle = Diagnostic::new(Span { start: 1, end: 1 }, "bad scalar");
    assert!(middle.render("utf8", source).contains("utf8:1:1: error:"));
    let eof = Diagnostic::new(
        Span {
            start: usize::MAX,
            end: 0,
        },
        "missing token",
    );
    assert!(eof.render("utf8", source).contains("utf8:3:1: error:"));
    assert!(eof.render("utf8", source).contains('^'));
    assert_eq!(middle.to_string(), "bad scalar");
}

/// Empty input still has a stable EOF token and a renderable insertion position.
#[test]
fn empty_input_is_well_defined() {
    let tokens = lex("").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, TokenKind::Eof);
    assert_eq!(tokens[0].span, Span { start: 0, end: 0 });
    assert!(
        Diagnostic::new(tokens[0].span, "expected func")
            .render("empty", "")
            .contains("empty:1:1")
    );
}

/// Small exhaustive combinations stress delimiter, newline, operator, and UTF-8 interactions.
/// The assertions concern externally useful range invariants, not scanner internals.
#[test]
fn short_inputs_always_report_valid_utf8_ranges() {
    let alphabet = ["a", "0", "/", "*", "\n", "\r", "🥚", " ", "="];
    // Enumerate every short string so accidental input-generator constraints cannot
    // exclude comment/operator boundaries or Unicode-adjacent malformed tokens.
    for length in 0..=4 {
        for mut code in 0..alphabet.len().pow(length) {
            let mut source = String::new();
            // Base-N decoding produces each combination once without a recursive generator.
            for _ in 0..length {
                source.push_str(alphabet[code % alphabet.len()]);
                code /= alphabet.len();
            }
            match lex(&source) {
                Ok(tokens) => {
                    let mut previous_end = 0;
                    // Public tokens must remain ordered and directly sliceable by clients.
                    for token in &tokens {
                        assert!(token.span.start >= previous_end, "{source:?}");
                        assert!(source.get(token.span.start..token.span.end).is_some());
                        previous_end = token.span.end;
                    }
                    assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof);
                    assert_eq!(previous_end, source.len());
                }
                Err(error) => {
                    assert!(source.get(error.span.start..error.span.end).is_some());
                    assert!(error.render("generated", &source).contains(": error:"));
                }
            }
        }
    }
}

/// Deep comment nesting must use bounded iterative state rather than host call frames.
#[test]
fn deeply_nested_comments_do_not_recurse() {
    let source = format!("{}{}tail", "/*".repeat(4096), "*/".repeat(4096));
    let tokens = lex(&source).unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].kind, TokenKind::Ident("tail".into()));
    assert_eq!(tokens[1].kind, TokenKind::Eof);
}
