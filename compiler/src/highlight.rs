//! Borrowed, tolerant highlighting categories for terminal and editor adapters.
//!
//! One shared lossless scanner partitions source, with a small lexical-context
//! classifier for declaration names, arguments, calls, and fields. These are
//! presentation hints, not resolved symbol kinds or language acceptance. The
//! iterator owns no token strings, trees, or output buffers. Lookahead revisits
//! at most two following significant tokens and their trivia per identifier.
//! Total work remains linear in source size without a throughput guarantee.

use crate::lex::{LexemeKind, Scanner, scan};
use crate::source::Span;

/// Stable presentation categories, independent of RGB values or terminal styling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HighlightKind {
    /// Original whitespace and line boundaries.
    Plain,
    /// Syntactic/control/header word, classified without symbol resolution.
    Keyword,
    /// Built-in type or a conventionally capitalized type-like identifier.
    Type,
    /// Function declaration name or identifier immediately followed by a call opener.
    Function,
    /// Parameter name immediately following a header `arg` word.
    Parameter,
    /// Field-like name after a dot, before a colon, or preceding a type-like name.
    Property,
    /// Whole numeric candidate; range validity is checked only by strict compilation.
    Number,
    /// Complete line or nested block comment.
    Comment,
    /// Arithmetic, comparison, assignment, or bitwise punctuation.
    Operator,
    /// Structural delimiters, commas, dots, colons, and separators.
    Punctuation,
    /// Unresolved ordinary name without a stronger lexical hint.
    Identifier,
    /// Unsupported scalar, unfinished quoted line, or unfinished block comment.
    Invalid,
    /// Experimental compiler namespace `core`.
    Builtin,
    /// Known execution-contract spelling, without asserting that the contract is proved.
    Contract,
    /// Complete quoted text candidate, with syntax validity left to compilation.
    Text,
}

impl HighlightKind {
    /// Every category in discriminant order, for dense, fixed-size style tables.
    pub const ALL: [Self; 15] = [
        Self::Plain,
        Self::Keyword,
        Self::Type,
        Self::Function,
        Self::Parameter,
        Self::Property,
        Self::Number,
        Self::Comment,
        Self::Operator,
        Self::Punctuation,
        Self::Identifier,
        Self::Invalid,
        Self::Builtin,
        Self::Contract,
        Self::Text,
    ];

    /// Return this category's stable lowercase stylesheet selector without allocating.
    pub const fn name(self) -> &'static str {
        // Every discriminant has one spelling; styles do not embed language semantics.
        match self {
            Self::Plain => "plain",
            Self::Keyword => "keyword",
            Self::Type => "type",
            Self::Function => "function",
            Self::Parameter => "parameter",
            Self::Property => "property",
            Self::Number => "number",
            Self::Comment => "comment",
            Self::Operator => "operator",
            Self::Punctuation => "punctuation",
            Self::Identifier => "identifier",
            Self::Invalid => "invalid",
            Self::Builtin => "builtin",
            Self::Contract => "contract",
            Self::Text => "text",
        }
    }

    /// Resolve one exact lowercase stylesheet category; unknown names return `None`.
    /// The fixed table is bounded independently of source size and allocates nothing.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// One nonempty original-source extent and its presentation category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Highlight {
    /// UTF-8-aligned half-open original source range.
    pub span: Span,
    /// Presentation category to resolve through the consumer's theme.
    pub kind: HighlightKind,
}

/// Return an allocation-free iterator covering every original source byte once.
/// Invalid/unfinished input remains displayable. The returned categories never
/// claim semantic validity, uniqueness, reference permissions, or checked effects.
pub fn highlight(source: &str) -> Highlighter<'_> {
    Highlighter {
        source,
        scanner: scan(source),
        previous: None,
    }
}

/// Borrowed classification state, with fixed-size context and no source-sized storage.
pub struct Highlighter<'a> {
    /// Immutable source used to inspect lexeme spelling by exact span.
    source: &'a str,
    /// Shared monotone lossless lexical iterator.
    scanner: Scanner<'a>,
    /// Most recent nontrivia lexeme, retaining only its kind and source extent.
    previous: Option<(LexemeKind, Span)>,
}

impl Iterator for Highlighter<'_> {
    type Item = Highlight;

    /// Classify and consume one lexeme, retaining trivia without changing context.
    /// Returns `None` at stable EOF and allocates no names, spans, or collections.
    fn next(&mut self) -> Option<Self::Item> {
        let lexeme = self.scanner.next()?;
        let kind = match lexeme.kind {
            LexemeKind::Whitespace | LexemeKind::Newline => HighlightKind::Plain,
            LexemeKind::LineComment | LexemeKind::BlockComment => HighlightKind::Comment,
            LexemeKind::Number => HighlightKind::Number,
            LexemeKind::Text | LexemeKind::Bytes => HighlightKind::Text,
            LexemeKind::Invalid => HighlightKind::Invalid,
            LexemeKind::EqualEqual
            | LexemeKind::NotEqual
            | LexemeKind::LessEqual
            | LexemeKind::GreaterEqual
            | LexemeKind::Symbol('=' | '<' | '>' | '+' | '-' | '*' | '/' | '%' | '&' | '^') => {
                HighlightKind::Operator
            }
            LexemeKind::Symbol(_) => HighlightKind::Punctuation,
            LexemeKind::Identifier => self.identifier(lexeme.span),
        };
        // Only trivia preserves prior lexical context; bad scalars break declaration hints.
        if !trivia(lexeme.kind) {
            self.previous = Some((lexeme.kind, lexeme.span));
        }
        Some(Highlight {
            span: lexeme.span,
            kind,
        })
    }
}

impl Highlighter<'_> {
    /// Classify the name at `span` using bounded lexical context and next-token shape.
    /// No declarations are resolved; names stay borrowed from the original source.
    fn identifier(&self, span: Span) -> HighlightKind {
        let word = &self.source[span.start..span.end];
        let previous = self
            .previous
            .map(|(kind, span)| (kind, &self.source[span.start..span.end]));
        // Explicit declaration/field context takes precedence over keyword-like names,
        // because Holo's current parser treats words contextually rather than reserving them.
        match previous {
            Some((LexemeKind::Identifier, "func")) => return HighlightKind::Function,
            Some((LexemeKind::Identifier, "struct")) => return HighlightKind::Type,
            Some((LexemeKind::Identifier, "arg")) => return HighlightKind::Parameter,
            _ => {}
        }
        let mut following_tokens = self.scanner.clone().filter(|lexeme| !trivia(lexeme.kind));
        let following = following_tokens.next();
        if matches!(previous, Some((LexemeKind::Identifier, "enum")))
            && following.is_some_and(|next| next.kind == LexemeKind::Symbol('{'))
        {
            return HighlightKind::Type;
        }
        let call = matches!(
            following.map(|lexeme| lexeme.kind),
            Some(LexemeKind::Symbol('('))
        );
        // Explicit field syntax permits keyword-shaped names. A projected call is
        // shown as a function; arbitrary projected names remain field-like hints.
        if matches!(previous, Some((LexemeKind::Symbol('.'), _)))
            || matches!(
                following.map(|lexeme| lexeme.kind),
                Some(LexemeKind::Symbol(':'))
            )
        {
            return if call {
                HighlightKind::Function
            } else {
                HighlightKind::Property
            };
        }
        // Known spellings aid incomplete-source display; strict parsing still owns acceptance.
        match word {
            "func" | "struct" | "with" | "arg" | "returns" | "return" | "as" | "readonly"
            | "readwrite" | "copy" | "reference" | "let" | "var" | "if" | "else" | "for" | "in" => {
                HighlightKind::Keyword
            }
            // A modifier belongs to a same-line declaration.
            // In other contexts `public` can still be an ordinary identifier.
            "public"
                if following.is_some_and(|next| {
                    next.kind == LexemeKind::Identifier
                        && matches!(
                            &self.source[next.span.start..next.span.end],
                            "func" | "struct" | "enum" | "error"
                        )
                        && !self.source[span.end..next.span.start].contains('\n')
                }) =>
            {
                HighlightKind::Keyword
            }
            // A contextual namespace header starts a statement and precedes a
            // same-line lowercase package prefix. The leaf may be unfinished.
            "namespace" | "using"
                if self.previous.is_none_or(|(kind, previous)| {
                    (word == "using" && kind == LexemeKind::Symbol(';'))
                        || self.source[previous.end..span.start].contains('\n')
                }) && following.is_some_and(|root| {
                    root.kind == LexemeKind::Identifier
                        && self.source.as_bytes()[root.span.start].is_ascii_lowercase()
                        && following_tokens.next().is_some_and(|dot| {
                            dot.kind == LexemeKind::Symbol('.')
                                && !self.source[span.end..dot.span.start].contains('\n')
                        })
                }) =>
            {
                HighlightKind::Keyword
            }
            // In declarations, a child source starts after a case boundary.
            // A bare or payload-bearing case named `from` stays an ordinary name.
            "from"
                if self.previous.is_none_or(|(kind, previous)| {
                    matches!(kind, LexemeKind::Symbol('{' | ',' | ';'))
                        || self.source[previous.end..span.start].contains('\n')
                }) && following.is_some_and(|next| {
                    next.kind == LexemeKind::Identifier
                        && !self.source[span.end..next.span.start].contains('\n')
                }) =>
            {
                HighlightKind::Keyword
            }
            // Propagation starts an assigned operand with a following name.
            // The contextual parser still accepts ordinary calls named `try`.
            "try"
                if matches!(previous, Some((LexemeKind::Symbol('='), _)))
                    && following.is_some_and(|next| {
                        next.kind == LexemeKind::Identifier
                            && !self.source[span.end..next.span.start].contains('\n')
                    }) =>
            {
                HighlightKind::Keyword
            }
            "enum"
                if following.is_some_and(|next| next.kind == LexemeKind::Identifier)
                    && following_tokens
                        .next()
                        .is_some_and(|next| next.kind == LexemeKind::Symbol('{')) =>
            {
                HighlightKind::Keyword
            }
            // Match contextual return/arm spellings and the two-token shape
            // of an error declaration, leaving an ordinary `error()` call alone.
            "error"
                if matches!(previous, Some((LexemeKind::Identifier, "on")))
                    || (following.is_some_and(|next| next.kind == LexemeKind::Identifier)
                        && (matches!(previous, Some((LexemeKind::Identifier, "return")))
                            || following_tokens
                                .next()
                                .is_some_and(|next| next.kind == LexemeKind::Symbol('{')))) =>
            {
                HighlightKind::Keyword
            }
            // Hosted control words are contextual; existing calls can use them as names.
            "loop" | "refill" if !call => HighlightKind::Keyword,
            "break" | "continue"
                if self.previous.is_none_or(|(kind, before)| {
                    matches!(kind, LexemeKind::Symbol('{' | '}' | ';'))
                        || self.source[before.end..span.start].contains('\n')
                }) && following.is_none_or(|next| {
                    matches!(next.kind, LexemeKind::Symbol(';' | '}'))
                        || self.source[span.end..next.span.start].contains('\n')
                }) =>
            {
                HighlightKind::Keyword
            }
            // A word between operands is an infix operator. Preserve the
            // spelling as a function or name when it starts an operand.
            "and" | "or"
                if self.previous.is_some_and(|(kind, before)| {
                    !self.source[before.end..span.start].contains('\n')
                        && match kind {
                            LexemeKind::Identifier => !matches!(
                                &self.source[before.start..before.end],
                                "let" | "var" | "return" | "if" | "for" | "in" | "then"
                            ),
                            LexemeKind::Number
                            | LexemeKind::Bytes
                            | LexemeKind::Symbol(')' | ']' | '}') => true,
                            _ => false,
                        }
                }) =>
            {
                HighlightKind::Operator
            }
            "not"
                if following.is_some_and(|next| {
                    !self.source[span.end..next.span.start].contains('\n')
                        && match next.kind {
                            LexemeKind::Identifier => !matches!(
                                &self.source[next.span.start..next.span.end],
                                "and" | "or" | "else"
                            ),
                            LexemeKind::Number | LexemeKind::Bytes => true,
                            LexemeKind::Symbol('(') => span.end < next.span.start,
                            _ => false,
                        }
                }) =>
            {
                HighlightKind::Operator
            }
            "noalloc" | "noblock" => HighlightKind::Contract,
            "u64" | "bool" | "Bytes" => HighlightKind::Type,
            "core" => HighlightKind::Builtin,
            _ => {
                // Known words take precedence over call shape, so `if (...)` stays
                // a keyword. An ordinary call still wins over capitalization.
                if call {
                    return HighlightKind::Function;
                }
                // Field declarations use `name Type`; only same-line type hints
                // are used, avoiding accidental field hints across statement boundaries.
                let followed_by_type = following.is_some_and(|next| {
                    next.kind == LexemeKind::Identifier
                        && !self.source[span.end..next.span.start].contains('\n')
                        && type_word(&self.source[next.span.start..next.span.end])
                });
                if followed_by_type {
                    HighlightKind::Property
                } else if type_word(word) {
                    HighlightKind::Type
                } else {
                    HighlightKind::Identifier
                }
            }
        }
    }
}

/// Recognize built-in or conventionally capitalized type-like spellings.
/// This is a nonempty ASCII-name hint, not symbol resolution or naming validation.
fn type_word(word: &str) -> bool {
    matches!(word, "u64" | "bool" | "Bytes") || word.as_bytes()[0].is_ascii_uppercase()
}

/// Identify trivia whose contents never select a declaration/call/field name.
/// Newlines remain visible spans even though this local presentation hint crosses them.
fn trivia(kind: LexemeKind) -> bool {
    matches!(
        kind,
        LexemeKind::Whitespace
            | LexemeKind::Newline
            | LexemeKind::LineComment
            | LexemeKind::BlockComment
    )
}
