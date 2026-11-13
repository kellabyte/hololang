//! Ordered expression parsing and bounded AST construction.
//!
//! Postfix operations bind tighter than infix operators. This module preserves
//! operators even when their source arithmetic policy is unavailable; lowering
//! must reject unsupported semantics instead of silently choosing wrapping math.
//! Record-literal parsing is disabled before control-flow braces to avoid treating
//! a loop's batch name as the beginning of a record initializer.

use super::{MAX_NESTING, NameKind, Parser};
use crate::frontend::ast::{BinaryOp, Expr, ExprKind, FieldInit, ResultArm, ResultPattern};
use crate::lex::TokenKind;
use crate::source::{Diagnostic, Span};

/// Comparisons bind tighter than negation, and conjunction binds more loosely.
const NOT_PRECEDENCE: u8 = 3;

impl Parser<'_> {
    /// Parse one expression, optionally admitting an unparenthesized record literal.
    /// Returns owned syntax or a source-local diagnostic without performing effects.
    pub(super) fn expression(&mut self, allow_record: bool) -> Result<Expr, Diagnostic> {
        let expression = self.nested(|parser| parser.binary(0, allow_record))?;
        validate_shape(&expression)?;
        Ok(expression)
    }

    /// Parse a binding operand with an optional contextual propagation prefix.
    /// An adjacent identifier starts a call or saved-result operand. `try(...)`
    /// remains an ordinary call to preserve existing source.
    pub(super) fn binding_operand(&mut self) -> Result<Expr, Diagnostic> {
        // Only a binding can introduce the finite propagation form.
        if !self.at_word("try")
            || !matches!(
                self.tokens.get(self.position + 1).map(|token| &token.kind),
                Some(TokenKind::Ident(_))
            )
        {
            return self.handled_expression();
        }
        let start = self.advance().span.start;
        let operand = self.expression(true)?;
        let span = Span {
            start,
            end: operand.span.end,
        };
        let expression = Expr {
            kind: ExprKind::Try(Box::new(operand)),
            span,
        };
        validate_shape(&expression)?;
        Ok(expression)
    }

    /// Parse one return operand, recognizing contextual success and error prefixes.
    /// Legacy calls such as `ok(x)` still parse as calls, with their original spans.
    pub(super) fn return_operand(&mut self) -> Result<Expr, Diagnostic> {
        let keyword = self.span();
        let following = self.tokens.get(self.position + 1);
        // A separated parenthesized payload only has prefix meaning in a Result body.
        // That preserves ordinary calls written with a space in legacy scalar bodies.
        let success_prefix = self.at_word("ok")
            && following.is_some_and(|token| {
                matches!(
                    token.kind,
                    TokenKind::Ident(_) | TokenKind::Number(_) | TokenKind::Bytes(_)
                ) || (self.typed_return
                    && token.kind == TokenKind::Symbol('(')
                    && keyword.end < token.span.start)
            });
        let error_prefix = self.at_word("error")
            && following.is_some_and(|token| matches!(token.kind, TokenKind::Ident(_)));
        // Keep unprefixed operands on the ordinary path, including exact-result forwarding.
        let (kind, end) = if success_prefix {
            self.advance();
            // Only a typed success can open an indented destination initializer.
            let value = if self.typed_return
                && matches!(self.current().kind, TokenKind::Ident(_))
                && self
                    .tokens
                    .get(self.position + 1)
                    .is_some_and(|token| token.kind == TokenKind::Symbol(':'))
            {
                self.destination_record(keyword.start)?
            } else {
                self.handled_expression()?
            };
            let end = value.span.end;
            (ExprKind::Ok(Box::new(value)), end)
        } else if error_prefix {
            self.advance();
            let case = self.case_name()?;
            // A qualified case may carry exactly one parenthesized operand.
            let (payload, end) = if self.take_symbol('(') {
                self.newlines();
                let value = self.expression(true)?;
                self.newlines();
                (Some(Box::new(value)), self.expect_symbol(')')?.end)
            } else {
                (None, case.span.end)
            };
            (
                ExprKind::Error {
                    case: Box::new(case),
                    payload,
                },
                end,
            )
        } else {
            return self.handled_expression();
        };
        let span = Span {
            start: keyword.start,
            end,
        };
        // The wrapper carries no semantic type; validation uses the actual function header.
        let expression = Expr { kind, span };
        validate_shape(&expression)?;
        Ok(expression)
    }

    /// Parse the fields after a typed success constructor's colon. A dedent
    /// leaves the physical separator for the enclosing statement parser.
    fn destination_record(&mut self, return_start: usize) -> Result<Expr, Diagnostic> {
        let (name, start) = self.identifier()?;
        let mut end = self.expect_symbol(':')?.end;
        let parent = self.indent_at(return_start);
        let mut column = None;
        let mut fields = Vec::new();
        // Each iteration consumes one line's initializer. Blank lines do not
        // affect the established field column, and a dedent ends the record.
        loop {
            let position = self.position;
            self.newlines();
            let span = self.span();
            let indent = self.indent_at(span.start);
            if self.position == position
                || indent <= parent
                || !matches!(self.current().kind, TokenKind::Ident(_))
            {
                self.position = position;
                break;
            }
            if *column.get_or_insert(indent) != indent {
                return Err(Diagnostic::new(
                    span,
                    "all destination fields must have the same indentation",
                ));
            }
            let (name, _) = self.identifier()?;
            self.expect_symbol('=')?;
            let value = self.binding_operand()?;
            end = value.span.end;
            fields.push(FieldInit {
                name,
                value,
                span: Span {
                    start: span.start,
                    end,
                },
            });
        }
        Ok(Expr {
            kind: ExprKind::Record {
                name,
                fields,
                indented: true,
            },
            span: Span {
                start: start.start,
                end,
            },
        })
    }

    /// Parse a return or binding operand followed by one indented list of result arms.
    /// Keeps a physical newline available to the statement parser when no arm follows.
    pub(super) fn handled_expression(&mut self) -> Result<Expr, Diagnostic> {
        let subject = self.expression(true)?;
        let mut arms = Vec::new();
        let mut column = None;
        // Each arm consumes its pattern, separator, and body before seeking another.
        while let Some((start, pattern)) =
            self.result_arm_header(subject.span.start, &mut column)?
        {
            let value = self.expression(true)?;
            let span = Span {
                start,
                end: value.span.end,
            };
            arms.push(ResultArm {
                pattern,
                value,
                span,
            });
        }
        // Avoid wrapping ordinary expressions, preserving their existing AST and grouping.
        let Some(last) = arms.last() else {
            return Ok(subject);
        };
        let span = Span {
            start: subject.span.start,
            end: last.span.end,
        };
        let expression = Expr {
            kind: ExprKind::Handle {
                subject: Box::new(subject),
                arms,
            },
            span,
        };
        validate_shape(&expression)?;
        Ok(expression)
    }

    /// Parse a shared `on` header and leave the selected expression or block unread.
    /// The first header establishes the list's indentation. Absent headers leave
    /// the original statement separator available without allocating syntax.
    pub(super) fn result_arm_header(
        &mut self,
        subject: usize,
        column: &mut Option<usize>,
    ) -> Result<Option<(usize, ResultPattern)>, Diagnostic> {
        // Only lists pay for a line-prefix lookup. Subsequent arms share the established column.
        if !self.take_result_arm() {
            return Ok(None);
        }
        let arm = self.span();
        let indent = self.indent_at(arm.start);
        if let Some(expected) = *column {
            if indent != expected {
                return Err(Diagnostic::new(
                    arm,
                    "all `on` arms must have the same indentation",
                ));
            }
        } else {
            if indent <= self.indent_at(subject) {
                return Err(Diagnostic::new(
                    arm,
                    "`on` must be indented deeper than its result expression",
                ));
            }
            *column = Some(indent);
        }
        self.expect_word("on")?;
        // Reserved patterns preserve the existing result grammar. A second case
        // name distinguishes a construction rejection from an ordinary case
        // called Rejected followed by `then` or a binding.
        let pattern = if self.take_word("ok") {
            ResultPattern::Ok(self.result_binding()?)
        } else if self.take_word("error") {
            ResultPattern::Fallback(self.result_binding()?)
        } else if self.at_word("Rejected")
            && matches!(
                self.tokens.get(self.position + 1).map(|token| &token.kind),
                Some(TokenKind::Ident(next)) if next != "then"
            )
        {
            self.advance();
            ResultPattern::Rejected {
                case: self.case_name()?,
                binding: self.result_binding()?,
            }
        } else {
            ResultPattern::Error {
                case: self.case_name()?,
                binding: self.result_binding()?,
            }
        };
        self.expect_word("then")?;
        Ok(Some((arm.start, pattern)))
    }

    /// Look past a real line boundary for a contextual arm introducer.
    /// Restores the cursor for unrelated syntax, including a legacy `on(...)` statement.
    fn take_result_arm(&mut self) -> bool {
        let position = self.position;
        self.newlines();
        // An identifier pattern distinguishes the arm from an ordinary call or projection.
        if self.position > position
            && self.at_word("on")
            && matches!(
                self.tokens.get(self.position + 1).map(|next| &next.kind),
                Some(TokenKind::Ident(_))
            )
        {
            return true;
        }
        self.position = position;
        false
    }

    /// Measure leading horizontal whitespace on the physical line containing `offset`.
    /// Virtual package spans share one origin. No source bytes are copied or mutated.
    fn indent_at(&self, offset: usize) -> usize {
        let prefix = &self.source[..offset - self.origin];
        let line = prefix.rsplit_once('\n').map_or(prefix, |(_, line)| line);
        line.bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t' | b'\x0b' | b'\x0c'))
            .count()
    }

    /// Read an optional single arm binding, leaving the `then` separator unconsumed.
    /// A missing closing parenthesis reports the unexpected token in the original source.
    fn result_binding(&mut self) -> Result<Option<String>, Diagnostic> {
        // A bare `ok` or `error` arm has no local binding.
        if !self.take_symbol('(') {
            return Ok(None);
        }
        let (name, _) = self.declared_identifier(NameKind::Binding)?;
        self.expect_symbol(')')?;
        Ok(Some(name))
    }

    /// Parse a case or family path, leaving payloads and contextual separators unread.
    /// The member chain retains exact spans and is bounded before recursive AST destruction.
    pub(super) fn case_name(&mut self) -> Result<Expr, Diagnostic> {
        let (name, span) = self.identifier()?;
        let mut expression = Expr {
            kind: ExprKind::Name(name),
            span,
        };
        let mut depth = 1;
        // Every iteration consumes a separator and identifier; excess depth fails locally.
        while self.take_symbol('.') {
            depth += 1;
            if depth > MAX_NESTING {
                return Err(Diagnostic::new(
                    self.span(),
                    "error case qualification exceeds the experimental parser limit",
                ));
            }
            let (field, end) = self.identifier()?;
            expression = Expr {
                kind: ExprKind::Member {
                    base: Box::new(expression),
                    field,
                },
                span: Span {
                    start: span.start,
                    end: end.end,
                },
            };
        }
        Ok(expression)
    }

    /// Parse left-associative infix operators using precedence climbing.
    /// Each recursive right operand has higher minimum precedence, preserving order.
    fn binary(&mut self, minimum: u8, allow_record: bool) -> Result<Expr, Diagnostic> {
        let mut left = self.prefix(allow_record)?;
        let mut count = 0;
        // Each iteration consumes an operator/right operand; a line ends a complete expression.
        while let Some((operator, precedence)) = infix(&self.current().kind) {
            if precedence < minimum {
                break;
            }
            count += 1;
            // Bound left-deep chains before their recursive Box drop becomes a host-stack hazard.
            if count > MAX_NESTING {
                return Err(Diagnostic::new(
                    self.span(),
                    "expression chain exceeds the experimental parser limit",
                ));
            }
            let operator_span = self.advance().span;
            self.newlines();
            let right = self.nested(|parser| parser.binary(precedence + 1, allow_record))?;
            let span = Span {
                start: left.span.start,
                end: right.span.end,
            };
            left = Expr {
                kind: ExprKind::Binary {
                    op: operator,
                    operator: operator_span,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            };
        }
        Ok(left)
    }

    /// Parse contextual negation. An adjacent `not(...)` is still a named call.
    fn prefix(&mut self, allow_record: bool) -> Result<Expr, Diagnostic> {
        if !self.at_word("not") {
            return self.postfix(allow_record);
        }
        let operator = self.span();
        let next = &self.tokens[self.position + 1];
        let starts_operand = match &next.kind {
            // `not and ...` and `not or ...` can still start with a local named
            // `not`. To negate a local named `and` or `or`, group the operand.
            TokenKind::Ident(name) => !matches!(name.as_str(), "and" | "or" | "else"),
            TokenKind::Number(_) | TokenKind::Bytes(_) => true,
            TokenKind::Symbol('(') => operator.end < next.span.start,
            _ => false,
        };
        if !starts_operand {
            return self.postfix(allow_record);
        }
        self.advance();
        let value = self.nested(|parser| parser.binary(NOT_PRECEDENCE, allow_record))?;
        Ok(Expr {
            span: Span {
                start: operator.start,
                end: value.span.end,
            },
            kind: ExprKind::Not {
                operator,
                value: Box::new(value),
            },
        })
    }

    /// Parse a primary followed by calls, field projections, indexing, or record fields.
    /// Postfix chains retain their written order and one covering source span.
    fn postfix(&mut self, allow_record: bool) -> Result<Expr, Diagnostic> {
        let mut expression = self.primary()?;
        let mut count = 0;
        // Each recognized suffix consumes punctuation; newlines do not implicitly join statements.
        loop {
            let start = expression.span.start;
            let (kind, end) = if self.at_symbol('(') {
                let (args, end) = self.arguments()?;
                (
                    ExprKind::Call {
                        callee: Box::new(expression),
                        args,
                    },
                    end,
                )
            } else if self.take_symbol('.') {
                self.newlines();
                let (field, end) = self.identifier()?;
                (
                    ExprKind::Member {
                        base: Box::new(expression),
                        field,
                    },
                    end.end,
                )
            } else if self.take_symbol('[') {
                self.newlines();
                let index = self.expression(true)?;
                self.newlines();
                let end = self.expect_symbol(']')?.end;
                (
                    ExprKind::Index {
                        base: Box::new(expression),
                        index: Box::new(index),
                    },
                    end,
                )
            } else if allow_record && self.at_symbol('{') {
                // A record literal begins only with a simple logical type name, not an arbitrary value.
                if let ExprKind::Name(name) = &expression.kind {
                    let name = name.clone();
                    let (fields, end) = self.initializers()?;
                    (
                        ExprKind::Record {
                            name,
                            fields,
                            indented: false,
                        },
                        end,
                    )
                } else {
                    return Ok(expression);
                }
            } else {
                return Ok(expression);
            };
            // Moving each previous node avoids repeatedly cloning the growing suffix tree.
            expression = Expr {
                kind,
                span: Span { start, end },
            };
            count += 1;
            // Postfix projection/call chains are bounded independently of recursive grammar depth.
            if count > MAX_NESTING {
                return Err(Diagnostic::new(
                    expression.span,
                    "postfix chain exceeds the experimental parser limit",
                ));
            }
        }
    }

    /// Parse a literal, name, or parenthesized expression without consuming block braces.
    /// Parentheses permit an explicit record literal even in a condition context.
    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.advance();
        // These are syntax alternatives only; names and literal types are resolved later.
        match token.kind {
            TokenKind::Number(value) => Ok(Expr {
                kind: ExprKind::Number(value),
                span: token.span,
            }),
            TokenKind::Bytes(value) => Ok(Expr {
                kind: ExprKind::Bytes(value),
                span: token.span,
            }),
            TokenKind::Ident(name) if name == "true" || name == "false" => Ok(Expr {
                kind: ExprKind::Bool(name == "true"),
                span: token.span,
            }),
            TokenKind::Ident(name) => Ok(Expr {
                kind: ExprKind::Name(name),
                span: token.span,
            }),
            TokenKind::Symbol('(') => {
                self.newlines();
                let mut expression = self.expression(true)?;
                self.newlines();
                expression.span = Span {
                    start: token.span.start,
                    end: self.expect_symbol(')')?.end,
                };
                Ok(expression)
            }
            _ => Err(Diagnostic::new(token.span, "expected an expression")),
        }
    }

    /// Parse a comma-separated call list, allowing multiline arguments and one trailing comma.
    /// Returns ordered arguments and the closing parenthesis byte offset.
    fn arguments(&mut self) -> Result<(Vec<Expr>, usize), Diagnostic> {
        self.expect_symbol('(')?;
        let mut args = Vec::new();
        self.newlines();
        // A closing delimiter terminates the list; every nonempty iteration parses a full argument.
        while !self.at_symbol(')') {
            args.push(self.expression(true)?);
            self.newlines();
            if self.at_symbol(')') {
                break;
            }
            self.expect_symbol(',')?;
            self.newlines();
        }
        let end = self.expect_symbol(')')?.end;
        Ok((args, end))
    }

    /// Parse named record initializers in source order without reordering or resolving fields.
    /// Returns their spans and the record's final brace offset.
    fn initializers(&mut self) -> Result<(Vec<FieldInit>, usize), Diagnostic> {
        self.expect_symbol('{')?;
        self.separators();
        let mut fields = Vec::new();
        // EOF is not a valid empty/trailing field; the missing brace is diagnosed locally.
        while !self.at_symbol('}') {
            if matches!(self.current().kind, TokenKind::Eof) {
                return Err(Diagnostic::new(
                    self.span(),
                    "expected `}` to end the record initializer",
                ));
            }
            let (name, span) = self.identifier()?;
            self.newlines();
            self.expect_symbol(':')?;
            self.newlines();
            let value = self.expression(true)?;
            let end = value.span.end;
            fields.push(FieldInit {
                name,
                value,
                span: Span {
                    start: span.start,
                    end,
                },
            });
            // Commas and line separators both preserve the programmer's field evaluation order.
            if self.take_symbol(',') {
                self.separators();
            } else {
                self.end_statement()?;
            }
        }
        let end = self.expect_symbol('}')?.end;
        Ok((fields, end))
    }
}

/// Classify one infix token and precedence, leaving unrecognized tokens to enclosing grammar.
fn infix(kind: &TokenKind) -> Option<(BinaryOp, u8)> {
    // Comparisons bind below bitwise/arithmetic forms; numeric semantics are not selected here.
    Some(match kind {
        TokenKind::Ident(name) if name == "or" => (BinaryOp::LogicalOr, 1),
        TokenKind::Ident(name) if name == "and" => (BinaryOp::LogicalAnd, 2),
        TokenKind::EqualEqual => (BinaryOp::Eq, 4),
        TokenKind::NotEqual => (BinaryOp::Ne, 4),
        TokenKind::Symbol('<') => (BinaryOp::Lt, 4),
        TokenKind::LessEqual => (BinaryOp::Le, 4),
        TokenKind::Symbol('>') => (BinaryOp::Gt, 4),
        TokenKind::GreaterEqual => (BinaryOp::Ge, 4),
        TokenKind::Symbol('^') => (BinaryOp::Xor, 5),
        TokenKind::Symbol('&') => (BinaryOp::And, 6),
        TokenKind::Symbol('+') => (BinaryOp::Add, 7),
        TokenKind::Symbol('-') => (BinaryOp::Sub, 7),
        TokenKind::Symbol('*') => (BinaryOp::Mul, 8),
        TokenKind::Symbol('/') => (BinaryOp::Div, 8),
        _ => return None,
    })
}

/// Bound the completed AST's actual depth using an iterative compiler-side walk.
/// Grammar limits alone do not bound combined left/postfix/nested child shapes.
fn validate_shape(expression: &Expr) -> Result<(), Diagnostic> {
    let mut pending = vec![(expression, 1)];
    // Each node is visited once here; no recursive visitor can overflow on rejected input.
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_NESTING {
            return Err(Diagnostic::new(
                node.span,
                "expression depth exceeds the experimental parser limit of 128",
            ));
        }
        match &node.kind {
            ExprKind::Call { callee, args } => {
                pending.push((callee, depth + 1));
                pending.extend(args.iter().map(|argument| (argument, depth + 1)));
            }
            ExprKind::Member { base, .. } => pending.push((base, depth + 1)),
            ExprKind::Index { base, index } => {
                pending.push((base, depth + 1));
                pending.push((index, depth + 1));
            }
            ExprKind::Record { fields, .. } => {
                pending.extend(fields.iter().map(|field| (&field.value, depth + 1)))
            }
            ExprKind::Binary { left, right, .. } => {
                pending.push((left, depth + 1));
                pending.push((right, depth + 1));
            }
            ExprKind::Not { value, .. } | ExprKind::Ok(value) | ExprKind::Try(value) => {
                pending.push((value, depth + 1));
            }
            ExprKind::Error { case, payload } => {
                pending.push((case, depth + 1));
                // An error operand is also bounded before recursively owned AST drop.
                pending.extend(payload.as_deref().map(|value| (value, depth + 1)));
            }
            ExprKind::Handle { subject, arms } => {
                pending.push((subject, depth + 1));
                // Pattern names and arm expressions are also strict children of the dispatch.
                for arm in arms {
                    pending.push((&arm.value, depth + 1));
                    if let ResultPattern::Error { case, .. }
                    | ResultPattern::Rejected { case, .. } = &arm.pattern
                    {
                        pending.push((case, depth + 1));
                    }
                }
            }
            ExprKind::Number(_)
            | ExprKind::Bytes(_)
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::ErrorFamilyRoot { .. } => {}
        }
    }
    Ok(())
}
