//! Lossless source scanning and strict compiler tokens for experimental Holo.
//!
//! One allocation-free byte scanner serves display tools and strict compilation.
//! Tool lexemes partition the original UTF-8 source, including comments and bad
//! scalars; the compiler adapter validates literals, owns identifiers, and restores
//! comment-internal newlines. Neither mode interprets contextual keywords. Scanning
//! is iterative; source bytes are traversed linearly, with comments revisited only
//! by the strict adapter to retain the parser's existing statement boundaries.

use crate::source::{Diagnostic, Span};
use std::sync::Arc;

/// One owned lexical value and its exact source range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Token classification, independent of parser keyword/context decisions.
    pub kind: TokenKind,
    /// Half-open byte extent in the original UTF-8 source.
    pub span: Span,
}

/// Lexical forms needed by preferred headers and the experimental core calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// ASCII identifier; keywords are interpreted by the parser.
    Ident(String),
    /// Exact unsigned 64-bit decimal or hexadecimal value.
    Number(u64),
    /// Decoded text for the experimental literal print statement.
    Text(String),
    /// Decoded bytes for a readonly byte string literal.
    Bytes(Arc<[u8]>),
    /// One recognized punctuation character.
    Symbol(char),
    /// `==` comparison.
    EqualEqual,
    /// `!=` comparison.
    NotEqual,
    /// `<=` comparison.
    LessEqual,
    /// `>=` comparison.
    GreaterEqual,
    /// One LF or CRLF line boundary, including inside a block comment.
    Newline,
    /// Empty range immediately after the final source byte.
    Eof,
}

/// Borrowed-source lexical category, without owned names or numeric interpretation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexemeKind {
    /// An ASCII identifier; contextual keywords have this category too.
    Identifier,
    /// Whole integer candidate, including malformed suffixes; strict lexing validates it.
    Number,
    /// Complete quoted text candidate; strict lexing validates the escapes.
    Text,
    /// Complete `b"..."` candidate; strict lexing validates bytes and escapes.
    Bytes,
    /// One recognized punctuation character.
    Symbol(char),
    /// The complete `==` operator.
    EqualEqual,
    /// The complete `!=` operator.
    NotEqual,
    /// The complete `<=` operator.
    LessEqual,
    /// The complete `>=` operator.
    GreaterEqual,
    /// Non-newline ASCII whitespace admitted by the compiler.
    Whitespace,
    /// One LF or CRLF boundary with its original byte extent.
    Newline,
    /// A line comment, excluding its terminating line boundary.
    LineComment,
    /// A complete block comment, including nested comments and line boundaries.
    BlockComment,
    /// One unsupported scalar, unfinished quoted line, or unterminated block comment.
    Invalid,
}

/// One lossless lexical range; its text stays borrowed from the caller's source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lexeme {
    /// Category of the complete original source extent.
    pub kind: LexemeKind,
    /// Nonempty, UTF-8-aligned half-open byte extent.
    pub span: Span,
}

/// Create an allocation-free iterator over every byte of immutable UTF-8 `source`.
/// Invalid input yields invalid ranges and scanning continues; empty input yields
/// no lexemes. This is lexical recovery for tooling, not compiler acceptance.
pub fn scan(source: &str) -> Scanner<'_> {
    Scanner {
        source,
        position: 0,
    }
}

/// Monotone borrowed scanner; cloning copies only a source pointer and byte cursor.
#[derive(Clone)]
pub struct Scanner<'a> {
    /// Original source, never copied or modified by scanning.
    source: &'a str,
    /// Next unconsumed byte; always a UTF-8 scalar boundary between iterations.
    position: usize,
}

impl Iterator for Scanner<'_> {
    type Item = Lexeme;

    /// Return the next nonempty range, or EOF, without allocating or failing.
    /// Every branch consumes bytes; nested comments use a counter rather than recursion.
    fn next(&mut self) -> Option<Self::Item> {
        // EOF is stable; no synthetic zero-length item can stall downstream iterators.
        if self.position == self.source.len() {
            return None;
        }
        let start = self.position;
        let bytes = self.source.as_bytes();
        let following = bytes.get(start + 1).copied();
        // Long ASCII runs are grouped, while multi-byte invalid scalars retain full ranges.
        let kind = match (bytes[start], following) {
            (b' ' | b'\t' | b'\x0b' | b'\x0c', _) => {
                self.position += 1;
                // A whitespace run stops before line boundaries to preserve grammar information.
                while self.position < bytes.len()
                    && matches!(bytes[self.position], b' ' | b'\t' | b'\x0b' | b'\x0c')
                {
                    self.position += 1;
                }
                LexemeKind::Whitespace
            }
            (b'\n', _) => {
                self.position += 1;
                LexemeKind::Newline
            }
            (b'\r', Some(b'\n')) => {
                self.position += 2;
                LexemeKind::Newline
            }
            (b'/', Some(b'/')) => {
                self.position += 2;
                // ASCII newline bytes cannot occur within a multi-byte UTF-8 scalar.
                while self.position < bytes.len() && !matches!(bytes[self.position], b'\n' | b'\r')
                {
                    self.position += 1;
                }
                LexemeKind::LineComment
            }
            (b'/', Some(b'*')) => self.block_comment(),
            (b'b', Some(b'"')) => {
                self.position += 1;
                self.quoted(LexemeKind::Bytes)
            }
            (b'"', _) => self.quoted(LexemeKind::Text),
            (b'a'..=b'z' | b'A'..=b'Z' | b'_', _) => {
                self.identifier_run();
                LexemeKind::Identifier
            }
            (b'0'..=b'9', _) => {
                self.identifier_run();
                LexemeKind::Number
            }
            (b'=', Some(b'=')) => self.pair(LexemeKind::EqualEqual),
            (b'!', Some(b'=')) => self.pair(LexemeKind::NotEqual),
            (b'<', Some(b'=')) => self.pair(LexemeKind::LessEqual),
            (b'>', Some(b'=')) => self.pair(LexemeKind::GreaterEqual),
            (
                b'{' | b'}' | b'(' | b')' | b'[' | b']' | b',' | b'.' | b':' | b';' | b'=' | b'<'
                | b'>' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'^',
                _,
            ) => {
                self.position += 1;
                LexemeKind::Symbol(char::from(bytes[start]))
            }
            _ => {
                self.position += self.source[start..]
                    .chars()
                    .next()
                    .expect("nonempty UTF-8 suffix")
                    .len_utf8();
                LexemeKind::Invalid
            }
        };
        Some(Lexeme {
            kind,
            span: Span {
                start,
                end: self.position,
            },
        })
    }
}

impl Scanner<'_> {
    /// Consume a known pair and return `kind`; no text or iterator items are allocated.
    fn pair(&mut self, kind: LexemeKind) -> LexemeKind {
        self.position += 2;
        kind
    }

    /// Consume one nonempty ASCII identifier-byte run from its first known valid byte.
    /// The cursor remains at an ASCII boundary or the start of the next Unicode scalar.
    fn identifier_run(&mut self) {
        self.position += 1;
        // Every iteration consumes an ASCII continuation and terminates at finite input end.
        while self.position < self.source.len()
            && identifier_byte(self.source.as_bytes()[self.position])
        {
            self.position += 1;
        }
    }

    /// Consume one double-quoted candidate or an invalid prefix up to the line break.
    /// Scan bytes without allocating. Escaped quotes never close the candidate.
    fn quoted(&mut self, kind: LexemeKind) -> LexemeKind {
        let bytes = self.source.as_bytes();
        self.position += 1;
        let mut escaped = false;
        // Every byte advances the cursor unless the next line belongs to the outer scanner.
        while let Some(&byte) = bytes.get(self.position) {
            if matches!(byte, b'\n' | b'\r') {
                break;
            }
            self.position += 1;
            // A backslash protects exactly its next byte from delimiter recognition.
            if byte == b'"' && !escaped {
                return kind;
            }
            escaped = byte == b'\\' && !escaped;
        }
        LexemeKind::Invalid
    }

    /// Consume a complete nested block comment or return one invalid range through EOF.
    /// The nesting counter is bounded by input length and uses no host-stack recursion.
    fn block_comment(&mut self) -> LexemeKind {
        self.position += 2;
        let mut depth = 1usize;
        let bytes = self.source.as_bytes();
        // ASCII delimiters cannot overlap UTF-8 continuation bytes, permitting byte scanning.
        while self.position < bytes.len() {
            match (bytes[self.position], bytes.get(self.position + 1).copied()) {
                (b'/', Some(b'*')) => {
                    depth += 1;
                    self.position += 2;
                }
                (b'*', Some(b'/')) => {
                    depth -= 1;
                    self.position += 2;
                    // Only the outer terminator ends the comment; nested suffixes remain trivia.
                    if depth == 0 {
                        return LexemeKind::BlockComment;
                    }
                }
                _ => self.position += 1,
            }
        }
        LexemeKind::Invalid
    }
}

/// Scan `source`, returning ordered owned tokens or the first byte-precise error.
/// Source remains unchanged; unsupported Unicode is rejected as a whole scalar,
/// while UTF-8 text is allowed in comments and text literals. Names and output tokens
/// allocate compiler memory; lossless scanning itself remains allocation-free.
pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    let mut tokens = Vec::new();
    // The shared scanner advances monotonically; the adapter preserves prior strict errors.
    for lexeme in scan(source) {
        let span = lexeme.span;
        let text = &source[span.start..span.end];
        let kind = match lexeme.kind {
            LexemeKind::Identifier => TokenKind::Ident(text.to_owned()),
            LexemeKind::Number => TokenKind::Number(integer(text, span)?),
            LexemeKind::Text => TokenKind::Text(decode_text(text, span)?),
            LexemeKind::Bytes => TokenKind::Bytes(decode_bytes(text, span)?),
            LexemeKind::Symbol(symbol) => TokenKind::Symbol(symbol),
            LexemeKind::EqualEqual => TokenKind::EqualEqual,
            LexemeKind::NotEqual => TokenKind::NotEqual,
            LexemeKind::LessEqual => TokenKind::LessEqual,
            LexemeKind::GreaterEqual => TokenKind::GreaterEqual,
            LexemeKind::Newline => TokenKind::Newline,
            LexemeKind::Whitespace | LexemeKind::LineComment => continue,
            LexemeKind::BlockComment => {
                comment_newlines(text, span.start, &mut tokens);
                continue;
            }
            LexemeKind::Invalid => {
                // Delimited invalid ranges get diagnostics for their opening construct.
                if text.starts_with("/*") {
                    return Err(Diagnostic::new(span, "unterminated block comment"));
                }
                if text.starts_with('"') {
                    return Err(Diagnostic::new(span, "unterminated text literal"));
                }
                if text.starts_with("b\"") {
                    return Err(Diagnostic::new(span, "unterminated byte string literal"));
                }
                let character = text.chars().next().expect("nonempty invalid lexeme");
                return Err(Diagnostic::new(
                    span,
                    format!("unsupported source character {character:?}"),
                ));
            }
        };
        tokens.push(Token { kind, span });
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span {
            start: source.len(),
            end: source.len(),
        },
    });
    Ok(tokens)
}

/// Decode ASCII and explicit byte escapes from a complete byte string candidate.
fn decode_bytes(text: &str, span: Span) -> Result<Arc<[u8]>, Diagnostic> {
    let body = &text[2..text.len() - 1];
    let mut decoded = Vec::with_capacity(body.len());
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        let byte = match character {
            '\\' => match characters.next() {
                Some('"') => b'"',
                Some('\\') => b'\\',
                Some('n') => b'\n',
                Some('r') => b'\r',
                Some('t') => b'\t',
                Some('0') => 0,
                Some('x') => {
                    let digit = |character: Option<char>| character.and_then(|ch| ch.to_digit(16));
                    let (Some(high), Some(low)) =
                        (digit(characters.next()), digit(characters.next()))
                    else {
                        return Err(Diagnostic::new(
                            span,
                            "byte string `\\x` escape requires two hexadecimal digits",
                        ));
                    };
                    ((high << 4) | low) as u8
                }
                _ => {
                    return Err(Diagnostic::new(
                        span,
                        "unsupported byte string literal escape",
                    ));
                }
            },
            ascii @ ' '..='~' => ascii as u8,
            _ => {
                return Err(Diagnostic::new(
                    span,
                    "byte string literals require printable ASCII or explicit escapes",
                ));
            }
        };
        decoded.push(byte);
    }
    Ok(decoded.into())
}

/// Decode one complete quoted candidate, preserving UTF-8 and rejecting NUL.
/// The result owns at most the byte length of the literal, and failures discard it.
fn decode_text(text: &str, span: Span) -> Result<String, Diagnostic> {
    let body = &text[1..text.len() - 1];
    let mut decoded = String::with_capacity(body.len());
    let mut characters = body.chars();
    // A plain scalar is copied directly, while each escape consumes one extra scalar.
    while let Some(character) = characters.next() {
        match character {
            '\0' => return Err(Diagnostic::new(span, "text literals cannot contain NUL")),
            '\\' => decoded.push(match characters.next() {
                Some('"') => '"',
                Some('\\') => '\\',
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                _ => return Err(Diagnostic::new(span, "unsupported text literal escape")),
            }),
            plain => decoded.push(plain),
        }
    }
    Ok(decoded)
}

/// Preserve every parser-visible LF/CRLF within a complete comment at `offset`.
/// Mutates only compiler token storage, with one linear pass over comment bytes.
fn comment_newlines(text: &str, offset: usize, tokens: &mut Vec<Token>) {
    let bytes = text.as_bytes();
    let mut position = 0;
    // Each branch advances one or two bytes; original source byte identities remain exact.
    while position < bytes.len() {
        let start = position;
        match bytes[position] {
            b'\r' if bytes.get(position + 1) == Some(&b'\n') => position += 2,
            b'\n' => position += 1,
            _ => {
                position += 1;
                continue;
            }
        }
        tokens.push(Token {
            kind: TokenKind::Newline,
            span: Span {
                start: offset + start,
                end: offset + position,
            },
        });
    }
}

/// Validate a whole `candidate` integer and return its exact unsigned value.
/// Errors own a diagnostic at `span`; checked arithmetic rejects overflow before
/// any literal can enter the AST. Success allocates nothing and changes no source.
fn integer(candidate: &str, span: Span) -> Result<u64, Diagnostic> {
    let (digits, base) = if let Some(digits) = candidate
        .strip_prefix("0x")
        .or_else(|| candidate.strip_prefix("0X"))
    {
        (digits, 16u64)
    } else {
        (candidate, 10u64)
    };
    // Empty hex prefixes must not silently become zero-valued literals.
    if digits.is_empty() {
        return Err(Diagnostic::new(span, "hexadecimal literal requires digits"));
    }
    let mut value = 0u64;
    // Checked arithmetic rejects overflow before a wrapped literal can enter the AST.
    for byte in digits.bytes() {
        let digit = match byte {
            b'0'..=b'9' => u64::from(byte - b'0'),
            b'a'..=b'f' if base == 16 => u64::from(byte - b'a' + 10),
            b'A'..=b'F' if base == 16 => u64::from(byte - b'A' + 10),
            _ => return Err(Diagnostic::new(span, "invalid digit in integer literal")),
        };
        // Decimal alphabetic suffixes and hexadecimal-invalid digits are errors.
        if digit >= base {
            return Err(Diagnostic::new(span, "invalid digit in integer literal"));
        }
        value = value
            .checked_mul(base)
            .and_then(|prefix| prefix.checked_add(digit))
            .ok_or_else(|| Diagnostic::new(span, "integer literal exceeds u64"))?;
    }
    Ok(value)
}

/// Classify ASCII identifier continuations without locale or Unicode normalization.
/// Returns a boolean with no allocations or source mutations.
fn identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
