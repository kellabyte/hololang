//! Token cursor and bounded parsing entry point for the experimental source path.
//!
//! Child modules separate declarations, expressions, and executable statements.
//! Parsing preserves byte spans and syntax only; lowering and the existing IR
//! checker remain responsible for semantic/effect/lifetime guarantees. Token and
//! AST vectors allocate compiler memory proportional to the admitted source.

#[path = "parse/declarations.rs"]
mod declarations;
#[path = "parse/expressions.rs"]
mod expressions;
#[path = "parse/naming.rs"]
mod naming;
#[path = "parse/statements.rs"]
mod statements;

use super::ast;
use crate::lex::{Token, TokenKind, lex};
use crate::source::{Diagnostic, Span};
use naming::NameKind;

/// Bound parser recursion before malformed source can exhaust the host stack.
const MAX_NESTING: usize = 128;

/// Mutable token cursor shared by focused grammar implementations.
struct Parser<'source> {
    /// Borrowed source text used to measure indentation only for `on` clauses.
    source: &'source str,
    /// Offset of the first source byte in the package's virtual source map.
    origin: usize,
    /// Owned token stream, always terminated by the lexer-provided EOF token.
    tokens: Vec<Token>,
    /// Index of the next unconsumed token.
    position: usize,
    /// Current recursive grammar depth, separate from source indentation.
    nesting: usize,
    /// Whether the active function body admits parenthesized success return payloads.
    typed_return: bool,
    /// Optional naming diagnostics over declarations in this original source.
    naming: Option<Vec<Diagnostic>>,
}

/// Parse experimental source into a spanned syntax tree without semantic lowering.
/// Returns the first lexical/grammar diagnostic; no native program is executed.
pub fn parse(source: &str) -> Result<ast::Module, Diagnostic> {
    parser_at(source, 0, false)?.module()
}

/// Parse standalone source and optionally collect naming diagnostics.
/// Parsing failures return the grammar diagnostic without any partial warnings.
pub(super) fn parse_with_naming(
    source: &str,
    naming: bool,
) -> Result<(ast::Module, Vec<Diagnostic>), Diagnostic> {
    let mut parser = parser_at(source, 0, naming)?;
    let module = parser.module()?;
    Ok((module, parser.naming.unwrap_or_default()))
}

/// Parse one standalone, project entry, or library file for whitespace tooling.
/// This only examines syntax. It does not need a library root or read dependencies.
pub(crate) fn for_format(source: &str) -> Result<ast::Module, Diagnostic> {
    let mut parser = parser_at(source, 0, false)?;
    parser.separators();
    // A `using` directive starts an entry's header, so one token selects it.
    if parser.at_word("using") {
        return parser
            .project_entry(PathMode::Package)
            .map(|(module, _)| module);
    }
    // A namespace header identifies a library even when all its declarations
    // are private. The formatter needs only the grammar, so no path is resolved.
    if parser.at_word("namespace") {
        return parser
            .library(PathMode::Package)
            .map(|library| library.module);
    }
    parser.module().or_else(|diagnostic| {
        // Private helpers can precede a public declaration. Retry only when the
        // ordinary grammar stopped on that introducer, and preserve its error
        // if the file still fails to parse as a library.
        if source.get(diagnostic.span.start..diagnostic.span.end) == Some("public") {
            library_with_naming(source, 0, false)
                .map(|(library, _)| library.module)
                .map_err(|_| diagnostic)
        } else {
            Err(diagnostic)
        }
    })
}

/// Grammar context for qualified names and imports in a source file.
#[derive(Clone, Copy)]
enum PathMode {
    /// One explicit root only, with library files closed to further imports.
    Rooted,
    /// Package-qualified names, with imported bindings scoped to each file.
    Package,
}

/// An import or a source namespace header naming one library namespace.
#[derive(Clone)]
pub(super) struct LeafPath {
    /// Owner at the start of the qualified name.
    pub package: String,
    /// Package-mode namespace segments between the package and its final qualifier.
    pub parents: Vec<String>,
    /// Leaf and default importer qualifier.
    pub leaf: String,
    /// Complete header range for diagnostics before or after resolution.
    pub span: Span,
}

impl LeafPath {
    /// Return the owned qualified name for graph lookup or diagnostics.
    pub(super) fn qualified(&self) -> String {
        let mut qualified = self.package.clone();
        // All segments form the lookup identity. Only the final one acts as
        // the importer's qualifier.
        for segment in self.parents.iter().chain(std::iter::once(&self.leaf)) {
            qualified.push('.');
            qualified.push_str(segment);
        }
        qualified
    }
}

/// Recognize one safe ASCII path component without copying its source spelling.
/// Leading digits, separators, and uppercase names never reach file lookup.
fn valid_path_segment(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Check a namespace leaf for both source declarations and source paths.
/// Reserved names would collide with the root, builtins, or literal syntax.
pub(super) fn valid_leaf_name(name: &str) -> bool {
    valid_path_segment(name) && !matches!(name, "core" | "holo" | "true" | "false")
}

/// Parsed library bodies and the source metadata the resolver consumes.
pub(super) struct Library {
    /// Functions, package records, and error families in order within each kind.
    pub module: ast::Module,
    /// Function names visible to importers of the library.
    pub public_names: Vec<String>,
    /// Record names visible to importers of a package library.
    pub public_records: Vec<String>,
    /// Enum names visible to importers of a package library.
    pub public_enums: Vec<String>,
    /// Error family names visible to importers of a package library.
    pub public_errors: Vec<String>,
    /// Optional source declaration to verify against the selected import.
    pub namespace: Option<LeafPath>,
    /// Names selected by this file's `using` directives, resolved after parsing.
    pub imports: Vec<LeafPath>,
}

/// Parse one rooted project entry and optionally collect naming diagnostics.
/// Returns imports without resolving dependencies or performing I/O.
pub(super) fn project_entry_with_naming(
    source: &str,
    naming: bool,
) -> Result<(ast::Module, Vec<LeafPath>, Vec<Diagnostic>), Diagnostic> {
    let mut parser = parser_at(source, 0, naming)?;
    let (module, imports) = parser.project_entry(PathMode::Rooted)?;
    Ok((module, imports, parser.naming.unwrap_or_default()))
}

/// Parse one rooted library leaf and optionally collect naming diagnostics.
/// The virtual byte offset preserves grammar and naming locations.
pub(super) fn library_with_naming(
    source: &str,
    offset: usize,
    naming: bool,
) -> Result<(Library, Vec<Diagnostic>), Diagnostic> {
    let mut parser = parser_at(source, offset, naming)?;
    let library = parser.library(PathMode::Rooted)?;
    Ok((library, parser.naming.unwrap_or_default()))
}

/// Parse the package entry and optionally collect naming diagnostics.
/// Source spans begin at zero, and dependency lookup belongs to the caller.
pub(super) fn package_entry_with_naming(
    source: &str,
    naming: bool,
) -> Result<(ast::Module, Vec<LeafPath>, Vec<Diagnostic>), Diagnostic> {
    let mut parser = parser_at(source, 0, naming)?;
    let (module, imports) = parser.project_entry(PathMode::Package)?;
    Ok((module, imports, parser.naming.unwrap_or_default()))
}

/// Parse a package library file at its offset and optionally check names.
/// Its namespace and imports are verified against the package graph afterward.
pub(super) fn package_library_with_naming(
    source: &str,
    offset: usize,
    naming: bool,
) -> Result<(Library, Vec<Diagnostic>), Diagnostic> {
    let mut parser = parser_at(source, offset, naming)?;
    let library = parser.library(PathMode::Package)?;
    Ok((library, parser.naming.unwrap_or_default()))
}

/// Lex one source once, shifting spans before the existing parser consumes
/// them. The project map validates that the file's end fits the host range.
fn parser_at(source: &str, offset: usize, naming: bool) -> Result<Parser<'_>, Diagnostic> {
    let mut tokens = lex(source).map_err(|mut error| {
        error.span.start += offset;
        error.span.end += offset;
        error
    })?;
    // Tokens, including EOF, stay in their owning file's virtual range.
    for token in &mut tokens {
        token.span.start += offset;
        token.span.end += offset;
    }
    Ok(Parser {
        source,
        origin: offset,
        tokens,
        position: 0,
        nesting: 0,
        typed_return: false,
        naming: naming.then(Vec::new),
    })
}

impl Parser<'_> {
    /// Borrow the current token; EOF remains stable after the final source byte.
    fn current(&self) -> &Token {
        &self.tokens[self.position]
    }

    /// Return the current source span without advancing or allocating.
    fn span(&self) -> Span {
        self.current().span
    }

    /// Advance one token while preserving the sentinel for safe error reporting.
    fn advance(&mut self) -> Token {
        let token = self.current().clone();
        // EOF must remain addressable even when a malformed production requests more tokens.
        if !matches!(token.kind, TokenKind::Eof) {
            self.position += 1;
        }
        token
    }

    /// Test a contextual keyword without reserving it globally in the lexer.
    fn at_word(&self, expected: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Ident(word) if word == expected)
    }

    /// Consume a contextual keyword when present, returning whether it matched.
    fn take_word(&mut self, expected: &str) -> bool {
        // A failed lookahead must not consume the token needed by another production.
        if self.at_word(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Require one contextual keyword and return its byte span or a local diagnostic.
    fn expect_word(&mut self, expected: &str) -> Result<Span, Diagnostic> {
        // Report the unexpected token rather than advancing through a malformed header.
        if !self.at_word(expected) {
            return Err(Diagnostic::new(
                self.span(),
                format!("expected `{expected}`"),
            ));
        }
        Ok(self.advance().span)
    }

    /// Test one punctuation token without consuming it.
    fn at_symbol(&self, expected: char) -> bool {
        matches!(self.current().kind, TokenKind::Symbol(symbol) if symbol == expected)
    }

    /// Consume punctuation only when it matches the requested grammar boundary.
    fn take_symbol(&mut self, expected: char) -> bool {
        // Nonmatching punctuation belongs to the caller's enclosing production.
        if self.at_symbol(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Require one punctuation token and retain the exact offending span on error.
    fn expect_symbol(&mut self, expected: char) -> Result<Span, Diagnostic> {
        // Missing delimiters terminate parsing rather than guessing a source transformation.
        if !self.at_symbol(expected) {
            return Err(Diagnostic::new(
                self.span(),
                format!("expected `{expected}`"),
            ));
        }
        Ok(self.advance().span)
    }

    /// Read one identifier token and its span; semantic names remain unresolved.
    fn identifier(&mut self) -> Result<(String, Span), Diagnostic> {
        let token = self.advance();
        // A keyword is still an identifier token here; enclosing productions select its role.
        match token.kind {
            TokenKind::Ident(name) => Ok((name, token.span)),
            _ => Err(Diagnostic::new(token.span, "expected an identifier")),
        }
    }

    /// Read a declaration or binding and record its written name when enabled.
    /// The result and grammar diagnostic remain identical to `identifier`.
    fn declared_identifier(&mut self, kind: NameKind) -> Result<(String, Span), Diagnostic> {
        let (name, span) = self.identifier()?;
        self.record_naming(&name, span, kind);
        Ok((name, span))
    }

    /// Record a name whose grammar role became clear after it was consumed.
    /// Disabled collection performs no style check or diagnostic allocation.
    fn record_naming(&mut self, name: &str, span: Span, kind: NameKind) {
        // A style diagnosis never changes the accepted identifier or AST.
        if let Some(warnings) = &mut self.naming
            && let Some(warning) = kind.warning(name, span)
        {
            warnings.push(warning);
        }
    }

    /// Skip physical line separators where the enclosing grammar permits continuation.
    fn newlines(&mut self) {
        // Each iteration consumes a token, guaranteeing termination at a real boundary.
        while matches!(self.current().kind, TokenKind::Newline) {
            self.advance();
        }
    }

    /// Skip complete-statement separators without making indentation semantic.
    fn separators(&mut self) {
        // Semicolons and newlines are equivalent only between complete statements/declarations.
        while matches!(
            self.current().kind,
            TokenKind::Newline | TokenKind::Symbol(';')
        ) {
            self.advance();
        }
    }

    /// Require a statement terminator or an enclosing block/EOF boundary.
    fn end_statement(&mut self) -> Result<(), Diagnostic> {
        // Adjacent statements need a visible separator; a closing brace terminates the last one.
        if matches!(
            self.current().kind,
            TokenKind::Newline | TokenKind::Symbol(';')
        ) {
            self.separators();
            Ok(())
        } else if self.at_symbol('}') || matches!(self.current().kind, TokenKind::Eof) {
            Ok(())
        } else {
            Err(Diagnostic::new(
                self.span(),
                "expected a newline or `;` after the statement",
            ))
        }
    }

    /// Run a recursive grammar production with a fixed host-stack depth bound.
    /// Both success and failure restore the caller's depth; source ownership is unchanged.
    fn nested<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T, Diagnostic>,
    ) -> Result<T, Diagnostic> {
        // Reject excessive nesting before recursing, rather than relying on a host stack overflow.
        if self.nesting >= MAX_NESTING {
            return Err(Diagnostic::new(
                self.span(),
                "source nesting exceeds the experimental parser limit of 128",
            ));
        }
        self.nesting += 1;
        let result = parse(self);
        self.nesting -= 1;
        result
    }
}
