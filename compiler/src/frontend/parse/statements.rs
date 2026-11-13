//! Scoped statements and control-flow syntax for the experimental frontend.
//!
//! This module preserves explicit source scopes, sequencing, and recovery blocks.
//! It does not infer region ownership or allocation effects: lowering translates
//! admitted forms into checked operations, retaining the existing verifier as the
//! authority for lifetime and memory safety. Parser recursion is bounded.

use super::{NameKind, Parser};
use crate::frontend::ast::{Expr, ExprKind, ResultBlockArm, Statement, StatementKind};
use crate::lex::{Token, TokenKind};
use crate::source::{Diagnostic, Span};

impl Parser<'_> {
    /// Parse one brace-delimited sequence, returning owned statements and its end offset.
    /// Nested blocks consume compiler memory only and fail locally on EOF/depth limits.
    pub(super) fn block(&mut self) -> Result<(Vec<Statement>, usize), Diagnostic> {
        self.nested(|parser| {
            parser.expect_symbol('{')?;
            parser.separators();
            let mut body = Vec::new();
            // Every statement consumes tokens; an unmatched EOF is diagnosed before another attempt.
            while !parser.at_symbol('}') {
                if matches!(parser.current().kind, TokenKind::Eof) {
                    return Err(Diagnostic::new(
                        parser.span(),
                        "expected `}` to end the block",
                    ));
                }
                body.push(parser.statement()?);
                parser.separators();
            }
            Ok((body, parser.expect_symbol('}')?.end))
        })
    }

    /// Parse the next statement, preserving mutation spelling and expression order.
    /// Returns its complete byte range; semantic validity is deliberately not inferred.
    fn statement(&mut self) -> Result<Statement, Diagnostic> {
        let start = self.span().start;
        // Contextual introducers select explicit statement grammar; other expressions remain unresolved.
        if self.at_word("let") || self.at_word("var") {
            self.binding()
        } else if self.take_word("return") {
            let value = self.return_operand()?;
            let end = value.span.end;
            self.end_statement()?;
            Ok(Statement {
                kind: StatementKind::Return(value),
                span: Span { start, end },
            })
        } else if self.at_word("if") {
            self.conditional()
        } else if self.at_word("for") {
            self.iteration()
        } else if self.at_with_owner("region") {
            self.with_region()
        } else if self.at_with_owner("tcp_stream") {
            self.with_tcp_stream()
        } else if self.at_word("loop")
            && self
                .tokens
                .get(self.position + 1)
                .is_some_and(|token| token.kind == TokenKind::Symbol('{'))
        {
            // A brace on the same line identifies the contextual keyword.
            // A newline can still end an existing expression called loop.
            self.expect_word("loop")?;
            let (body, end) = self.block()?;
            Ok(Statement {
                kind: StatementKind::Loop(body),
                span: Span { start, end },
            })
        } else if (self.at_word("break") || self.at_word("continue"))
            && self.tokens.get(self.position + 1).is_some_and(|token| {
                matches!(
                    token.kind,
                    TokenKind::Newline | TokenKind::Symbol(';' | '}') | TokenKind::Eof
                )
            })
        {
            // A bare control word ends the statement. Calls and assignments can
            // still use the same spelling as an ordinary name.
            let kind = if self.at_word("break") {
                StatementKind::Break
            } else {
                StatementKind::Continue
            };
            let span = self.advance().span;
            self.end_statement()?;
            Ok(Statement { kind, span })
        } else if self.at_word("try")
            && matches!(
                self.tokens.get(self.position + 1).map(|token| &token.kind),
                Some(TokenKind::Ident(_))
            )
        {
            let value = self.binding_operand()?;
            let span = value.span;
            self.end_statement()?;
            Ok(Statement {
                kind: StatementKind::Expr(value),
                span,
            })
        } else if self.at_word("refill")
            && matches!(
                self.tokens.get(self.position + 1).map(|token| &token.kind),
                Some(TokenKind::Ident(_))
            )
        {
            self.refill()
        } else if self.at_symbol('{') {
            let (body, end) = self.block()?;
            Ok(Statement {
                kind: StatementKind::Scope(body),
                span: Span { start, end },
            })
        } else if self.at_text_print() {
            self.text_print()
        } else if self.at_word("else") || self.at_word("func") || self.at_word("struct") {
            Err(Diagnostic::new(
                self.span(),
                "unexpected declaration or `else` inside a statement block",
            ))
        } else {
            let target = self.expression(true)?;
            // A single equals sign is assignment; comparisons stay within the expression grammar.
            let (kind, end) = if self.take_symbol('=') {
                self.newlines();
                let value = self.expression(true)?;
                let end = value.span.end;
                (StatementKind::Assign { target, value }, end)
            } else {
                self.block_handler(target)?
            };
            self.end_statement()?;
            Ok(Statement {
                kind,
                span: Span { start, end },
            })
        }
    }

    /// Recognize a literal print operand without changing numeric call syntax.
    /// Lookahead skips leading argument newlines and does not advance the cursor.
    fn at_text_print(&self) -> bool {
        matches!(
            &self.tokens[self.position..],
            [
                Token { kind: TokenKind::Ident(core), .. },
                Token { kind: TokenKind::Symbol('.'), .. },
                Token { kind: TokenKind::Ident(print), .. },
                Token { kind: TokenKind::Symbol('('), .. },
                rest @ ..
            ] if core == "core" && print == "print" && rest.iter()
                .find(|token| token.kind != TokenKind::Newline)
                .is_some_and(|token| matches!(token.kind, TokenKind::Text(_)))
        )
    }

    /// Parse exactly one quoted print operand with the usual call delimiters.
    /// The literal has no expression form, so it cannot be bound or passed elsewhere.
    fn text_print(&mut self) -> Result<Statement, Diagnostic> {
        let start = self.expect_word("core")?.start;
        self.expect_symbol('.')?;
        self.expect_word("print")?;
        self.expect_symbol('(')?;
        self.newlines();
        let TokenKind::Text(text) = self.advance().kind else {
            unreachable!("literal print lookahead")
        };
        self.newlines();
        self.take_symbol(',');
        self.newlines();
        let end = self.expect_symbol(')')?.end;
        self.end_statement()?;
        Ok(Statement {
            kind: StatementKind::PrintText(text),
            span: Span { start, end },
        })
    }

    /// Recognize a named core owner introducer without consuming an ordinary
    /// `with` call or name. Token slices never cross statement breaks.
    fn at_with_owner(&self, operation: &str) -> bool {
        matches!(
            &self.tokens[self.position..],
            [
                Token { kind: TokenKind::Ident(with), .. },
                Token { kind: TokenKind::Ident(core), .. },
                Token { kind: TokenKind::Symbol('.'), .. },
                Token { kind: TokenKind::Ident(owner), .. },
                Token { kind: TokenKind::Symbol('('), .. },
                ..
            ] if with == "with" && core == "core" && owner == operation
        )
    }

    /// Parse the lexical binding and bounded brace body for the host-admitted
    /// stream. The empty call syntax accepts no descriptor or address from source.
    fn with_tcp_stream(&mut self) -> Result<Statement, Diagnostic> {
        let start = self.expect_word("with")?.start;
        self.expect_word("core")?;
        self.expect_symbol('.')?;
        self.expect_word("tcp_stream")?;
        self.expect_symbol('(')?;
        self.newlines();
        self.expect_symbol(')')?;
        self.expect_word("as")?;
        let (stream, _) = self.declared_identifier(NameKind::Binding)?;
        self.newlines();
        let (body, end) = self.block()?;
        Ok(Statement {
            kind: StatementKind::TcpScope { stream, body },
            span: Span { start, end },
        })
    }

    /// Parse one byte budget and two mandatory owner outcome bodies.
    /// Each expression and child statement retains its source span. No binding
    /// is introduced here, and the normal expression/block limits bound parsing.
    fn with_region(&mut self) -> Result<Statement, Diagnostic> {
        let start = self.expect_word("with")?.start;
        self.expect_word("core")?;
        self.expect_symbol('.')?;
        self.expect_word("region")?;
        self.expect_symbol('(')?;
        self.newlines();
        let capacity = self.expression(true)?;
        self.newlines();
        // A trailing comma keeps ordinary call spelling. A second operand
        // fails at the closing delimiter because there is exactly one budget.
        self.take_symbol(',');
        self.newlines();
        self.expect_symbol(')')?;
        self.newlines();
        let (body, end) = self.block()?;
        // Name the owner statement when recovery was omitted. A following
        // ordinary statement remains outside both outcome scopes.
        if !self.take_else() {
            return Err(Diagnostic::new(
                Span { start, end },
                "with core.region requires explicit `else { ... }` acquisition recovery",
            ));
        }
        self.newlines();
        let (on_failure, end) = self.block()?;
        Ok(Statement {
            kind: StatementKind::WithRegion {
                capacity,
                body,
                on_failure,
            },
            span: Span { start, end },
        })
    }

    /// Parse outcome blocks following a standalone expression.
    /// Nested blocks use the same depth and separator rules as other statements.
    fn block_handler(&mut self, subject: Expr) -> Result<(StatementKind, usize), Diagnostic> {
        let constructor = is_construct(&subject);
        let mut arms = Vec::new();
        let mut column = None;
        // Each written arm contributes one lexical block and its own source range.
        while let Some((start, pattern)) =
            self.result_arm_header(subject.span.start, &mut column, constructor)?
        {
            self.newlines();
            let (body, end) = self.block()?;
            arms.push(ResultBlockArm {
                pattern,
                body,
                span: Span { start, end },
            });
        }
        if let Some(end) = arms.last().map(|arm| arm.span.end) {
            let kind = if constructor {
                StatementKind::Construct { subject, arms }
            } else {
                StatementKind::HandleResult { subject, arms }
            };
            Ok((kind, end))
        } else if constructor {
            Err(Diagnostic::new(
                subject.span,
                "core.construct requires `on` outcome blocks",
            ))
        } else {
            let end = subject.span.end;
            Ok((StatementKind::Expr(subject), end))
        }
    }

    /// Parse one attempt followed by two mandatory, separately bound outcomes.
    /// Outcome names belong to their own blocks; types and ownership are checked later.
    fn refill(&mut self) -> Result<Statement, Diagnostic> {
        let start = self.expect_word("refill")?.start;
        let slot = self.expression(false)?;
        self.expect_symbol(',')?;
        self.newlines();
        let limit = self.expression(false)?;
        // The optional contextual owner leaves the existing operand and arm
        // syntax intact. Requiring a name rules out implicit acquisition here.
        let stream = if self.take_word("from") {
            Some(self.identifier()?)
        } else {
            None
        };
        self.expect_word("as")?;
        let (data_name, _) = self.declared_identifier(NameKind::Binding)?;
        self.newlines();
        let (data_body, _) = self.block()?;
        self.newlines();
        self.expect_word("else")?;
        let (status_name, _) = self.declared_identifier(NameKind::Binding)?;
        self.newlines();
        let (other_body, end) = self.block()?;
        Ok(Statement {
            kind: StatementKind::Refill {
                slot,
                limit,
                stream,
                data_name,
                data_body,
                status_name,
                other_body,
            },
            span: Span { start, end },
        })
    }

    /// Parse a local binding with an optional explicit failure/recovery block.
    /// The initializer is stored once; mutability and recovery meaning are checked later.
    fn binding(&mut self) -> Result<Statement, Diagnostic> {
        let start = self.span().start;
        let mutable = self.take_word("var");
        // The introducer is either var (already consumed) or the caller-established let.
        if !mutable {
            self.expect_word("let")?;
        }
        // Keep `readwrite` available as a local name when followed by `=`.
        let writable_view = self.at_word("readwrite")
            && matches!(
                self.tokens.get(self.position + 1).map(|token| &token.kind),
                Some(TokenKind::Ident(_))
            );
        if writable_view {
            self.expect_word("readwrite")?;
        }
        let (name, _) = self.declared_identifier(NameKind::Binding)?;
        self.expect_symbol('=')?;
        self.newlines();
        let value = self.binding_operand()?;
        let mut end = value.span.end;
        // Looking across a newline must restore it when no recovery clause follows.
        let otherwise = if self.take_else() {
            self.newlines();
            let (body, block_end) = self.block()?;
            end = block_end;
            Some(body)
        } else {
            None
        };
        self.end_statement()?;
        Ok(Statement {
            kind: StatementKind::Let {
                name,
                mutable,
                writable_view,
                value,
                otherwise,
            },
            span: Span { start, end },
        })
    }

    /// Parse an if/else block without confusing its opening brace with a record literal.
    /// Returns both branches in source order, with an empty absent else branch.
    fn conditional(&mut self) -> Result<Statement, Diagnostic> {
        let start = self.expect_word("if")?.start;
        let condition = self.expression(false)?;
        self.newlines();
        let (then_body, mut end) = self.block()?;
        // Only a written else contributes a second scope; no implicit recovery is introduced.
        let else_body = if self.take_else() {
            self.newlines();
            let (body, block_end) = self.block()?;
            end = block_end;
            body
        } else {
            Vec::new()
        };
        Ok(Statement {
            kind: StatementKind::If {
                condition,
                then_body,
                else_body,
            },
            span: Span { start, end },
        })
    }

    /// Parse one sequential for form with optional explicit element-access spelling.
    /// Iterable meaning (count or batch) belongs to typed lowering rather than syntax.
    fn iteration(&mut self) -> Result<Statement, Diagnostic> {
        let start = self.expect_word("for")?.start;
        // Access is explicit when written; omission preserves count-loop syntax unchanged.
        let access = if self.at_word("readonly") || self.at_word("readwrite") {
            Some(self.identifier()?.0)
        } else {
            None
        };
        let (name, _) = self.declared_identifier(NameKind::Binding)?;
        self.expect_word("in")?;
        let iterable = self.expression(false)?;
        self.newlines();
        let (body, end) = self.block()?;
        Ok(Statement {
            kind: StatementKind::For {
                name,
                access,
                iterable,
                body,
            },
            span: Span { start, end },
        })
    }

    /// Consume an else after optional newlines, restoring the cursor when absent.
    /// Semicolons deliberately end the previous statement and are not crossed.
    fn take_else(&mut self) -> bool {
        let saved = self.position;
        self.newlines();
        // Failed optional lookahead must not erase the visible statement separator.
        if self.take_word("else") {
            true
        } else {
            self.position = saved;
            false
        }
    }
}

/// Recognize the contextual core call that admits statement arms.
/// Other calls preserve their original statement grammar and resolution.
fn is_construct(expr: &Expr) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Call { callee, .. }
            if matches!(
                &callee.kind,
                ExprKind::Member { base, field }
                    if field == "construct"
                        && matches!(&base.kind, ExprKind::Name(name) if name == "core")
            )
    )
}
