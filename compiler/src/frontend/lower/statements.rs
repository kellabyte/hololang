//! Structured execution, mutation, and observable core operations.
//!
//! Structured branches/loops retain lexical roots for the existing checker.
//! Assignment captures a byte destination index and checks its bounds before
//! evaluating the replacement, preserving specified left-to-right failure order.
//! Entry returns use the backend's lexical cleanup path, including early exits.

use super::{Block, Kind, Lowerer, arity, ast, core_call};
use crate::{
    ir,
    source::{Diagnostic, Span},
};

impl Lowerer {
    /// Lower statements in written order; owners consume the remaining block.
    /// All statements are checked, including statically unreachable source text.
    pub(super) fn statements(&mut self, body: &[ast::Statement]) -> Result<Block, Diagnostic> {
        let mut output = Block::default();
        // Each ordinary statement advances the cursor; an owner lowers its tail once and returns.
        for (index, statement) in body.iter().enumerate() {
            if matches!(statement.kind, ast::StatementKind::Let { .. }) {
                let (declaration, consumed_tail) =
                    self.declaration(statement, &body[index + 1..])?;
                output.extend(declaration);
                if consumed_tail {
                    return Ok(output);
                }
            } else {
                output.extend(self.statement(statement)?);
            }
        }
        Ok(output)
    }

    /// Lower one nondeclaration statement, preserving structured source maps.
    /// Returns explicit core operations or a subset diagnostic without target execution.
    fn statement(&mut self, statement: &ast::Statement) -> Result<Block, Diagnostic> {
        let mut output = Block::default();
        match &statement.kind {
            // Declaration routing belongs to statements because owner tails consume siblings.
            ast::StatementKind::Let { .. } => {
                unreachable!("declarations are routed by block lowering")
            }
            ast::StatementKind::Assign { target, value } => {
                self.assign(target, value, statement.span, &mut output)?
            }
            ast::StatementKind::Expr(expr) => self.operation(expr, &mut output)?,
            ast::StatementKind::PrintText(text) => {
                // Like numeric print, literal output belongs to entry execution.
                if self.ordinary_body {
                    return Err(Diagnostic::new(
                        statement.span,
                        "core.print is unavailable in ordinary borrowed functions",
                    ));
                }
                output.push(ir::Op::PrintText(text.clone()), statement.span, []);
            }
            ast::StatementKind::Construct { subject, arms } => {
                self.construct(subject, arms, statement.span, &mut output)?;
            }
            ast::StatementKind::HandleResult { subject, arms } => {
                self.handle_result_blocks(subject, arms, statement.span, &mut output)?;
            }
            ast::StatementKind::WithRegion {
                capacity,
                body,
                on_failure,
            } => {
                self.with_region(capacity, body, on_failure, statement.span, &mut output)?;
            }
            ast::StatementKind::TcpScope { stream, body } => {
                // Ordinary functions do not have the hosted entry admission.
                // The checker separately validates the entry's scope placement.
                if self.ordinary_body {
                    return Err(Diagnostic::new(
                        statement.span,
                        "with core.tcp_stream is available only in main",
                    ));
                }
                let (stream, body) =
                    self.scoped_input(stream, Kind::TcpStream, body, statement.span)?;
                output.push(
                    ir::Op::TcpScope {
                        stream,
                        body: body.ops,
                    },
                    statement.span,
                    [body.spans],
                );
            }
            // Ordinary calls return their declared type. Entry completion
            // keeps its separate process cleanup and zero status.
            ast::StatementKind::Return(value) => {
                // A typed return owns exactly one alternative and retains its nominal identity.
                if let Some(result) = self.result.clone() {
                    let result = self.result_return(value, &result, &mut output)?;
                    output.push(ir::Op::ReturnResult(result), statement.span, []);
                    return Ok(output);
                }
                if self.returns_bool {
                    let value = self.boolean(value, &mut output)?;
                    output.push(ir::Op::ReturnBool(value), statement.span, []);
                    return Ok(output);
                }
                if self.ordinary_body {
                    let value = self.scalar(value, &mut output)?;
                    output.push(ir::Op::Return(value), statement.span, []);
                    return Ok(output);
                }
                // Entry can directly handle a typed append when each alternative
                // has the same permitted literal-zero exit result. The append
                // still runs before the cleanup-aware process exit.
                if let ast::ExprKind::Handle { subject, arms } = &value.kind
                    && let Some(("append", args)) = core_call(subject)
                {
                    if let Some(arm) = arms
                        .iter()
                        .find(|arm| !matches!(arm.value.kind, ast::ExprKind::Number(0)))
                    {
                        return Err(Diagnostic::new(
                            arm.value.span,
                            "an append returned from experimental main requires a literal zero in every arm",
                        ));
                    }
                    let (call, arms) =
                        self.append_decision(args, subject.span, arms, false, &mut output)?;
                    let result = self.fresh("append");
                    output.push(call.handled(result, arms), statement.span, []);
                } else if !matches!(value.kind, ast::ExprKind::Number(0)) {
                    return Err(Diagnostic::new(
                        value.span,
                        "only `return 0` is supported in experimental main; general exit codes remain unsupported",
                    ));
                }
                output.push(ir::Op::ExitZero, statement.span, []);
            }
            ast::StatementKind::Scope(body) => {
                let body = self.scoped(body)?;
                output.push(ir::Op::Scope(body.ops), statement.span, [body.spans]);
            }
            // The hosted entry may repeat an input session without an artificial count.
            ast::StatementKind::Loop(body) => {
                if self.ordinary_body {
                    return Err(Diagnostic::new(
                        statement.span,
                        "experimental loop is available only in main",
                    ));
                }
                let body = self.scoped(body)?;
                output.push(ir::Op::Loop(body.ops), statement.span, [body.spans]);
            }
            ast::StatementKind::Break => output.push(ir::Op::Break, statement.span, []),
            ast::StatementKind::Continue => output.push(ir::Op::Continue, statement.span, []),
            ast::StatementKind::Refill {
                slot,
                limit,
                stream,
                data_name,
                data_body,
                status_name,
                other_body,
            } => {
                // Only the entry owns the hosted input capability. The verifier
                // still checks that the destination has write access.
                if self.ordinary_body {
                    return Err(Diagnostic::new(
                        statement.span,
                        "hosted refill is available only in main",
                    ));
                }
                // Resolve the named capability before examining the byte range.
                // The checker tracks whether it is live at this exact operation.
                let stream = stream
                    .as_ref()
                    .map(|(name, span)| self.tcp_name(name, *span))
                    .transpose()?;
                let destination = self.refill_destination(slot)?;
                // Preserve destination capture before evaluating the request,
                // including scalar call expressions with checks and state writes.
                let previous = std::mem::replace(&mut self.deferred_actual, true);
                let limit = self.scalar(limit, &mut output);
                self.deferred_actual = previous;
                let limit = limit?;
                assert!(
                    output.ops.is_empty(),
                    "refill limit must remain an expression"
                );
                let (data_name, data) =
                    self.scoped_input(data_name, Kind::ReceivedBytes, data_body, statement.span)?;
                let (result_name, other) = self.scoped_input(
                    status_name,
                    Kind::Scalar { mutable: false },
                    other_body,
                    statement.span,
                )?;
                output.push(
                    Self::tcp_io(
                        stream,
                        ir::Op::Refill(ir::Refill {
                            destination,
                            limit,
                            data_name,
                            data_body: data.ops,
                            result_name,
                            other_body: other.ops,
                        }),
                    ),
                    statement.span,
                    [data.spans, other.spans],
                );
            }
            // Both branches are lowered and then checked, independent of a constant condition.
            ast::StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition = self.condition(condition, &mut output)?;
                let then_body = self.scoped(then_body)?;
                let else_body = self.scoped(else_body)?;
                output.push(
                    ir::Op::If {
                        condition,
                        then_body: then_body.ops,
                        else_body: else_body.ops,
                    },
                    statement.span,
                    [then_body.spans, else_body.spans],
                );
            }
            ast::StatementKind::For {
                name,
                access,
                iterable,
                body,
            } => self.iteration(
                name,
                access.as_deref(),
                iterable,
                body,
                statement.span,
                &mut output,
            )?,
        }
        Ok(output)
    }

    /// Lower a statement-position operation; status-bearing append cannot be discarded.
    /// Scalar and boolean calls may discard their value while retaining evaluation.
    fn operation(&mut self, expr: &ast::Expr, output: &mut Block) -> Result<(), Diagnostic> {
        if let ast::ExprKind::Try(operand) = &expr.kind {
            let operation = self.try_binding(None, operand, expr.span)?;
            output.push(operation, expr.span, []);
            return Ok(());
        }
        if let Some((name, args)) = core_call(expr) {
            // The closed helper subset admits assertion traps but no external
            // observation or owner invalidation through statement builtins.
            if self.ordinary_body && matches!(name, "print" | "reset" | "opaque") {
                return Err(Diagnostic::new(
                    expr.span,
                    format!("core.{name} is unavailable in ordinary borrowed functions"),
                ));
            }
            match name {
                // Assertions terminate on false; print observes an explicitly evaluated scalar.
                "assert" => {
                    arity(args, 1, expr.span)?;
                    let condition = self.condition(&args[0], output)?;
                    output.push(ir::Op::Assert(condition), expr.span, []);
                    return Ok(());
                }
                "print" => {
                    arity(args, 1, expr.span)?;
                    let value = self.scalar(&args[0], output)?;
                    output.push(ir::Op::Print(value), expr.span, []);
                    return Ok(());
                }
                "reset" => {
                    arity(args, 1, expr.span)?;
                    let (batch, _) = self.owned_batch(&args[0])?;
                    output.push(ir::Op::Reset { batch }, expr.span, []);
                    return Ok(());
                }
                "opaque" => {
                    arity(args, 1, expr.span)?;
                    let region = self.region(&args[0])?;
                    output.push(ir::Op::OpaqueRegion(region), expr.span, []);
                    return Ok(());
                }
                "move_bytes" => {
                    arity(args, 2, expr.span)?;
                    let destination = self.byte_move_parameter(&args[0], true)?;
                    let source = self.byte_move_parameter(&args[1], false)?;
                    output.push(
                        ir::Op::MoveInputBytes {
                            destination,
                            source,
                        },
                        expr.span,
                        [],
                    );
                    return Ok(());
                }
                "append" => {
                    return Err(Diagnostic::new(
                        expr.span,
                        "bind core.append's construction status with let/var; discarded fallible appends are unsupported",
                    ));
                }
                _ => {}
            }
        }
        if !matches!(expr.kind, ast::ExprKind::Call { .. }) {
            return Err(Diagnostic::new(
                expr.span,
                "expected a supported operation or scalar helper call",
            ));
        }
        if self.is_bool(expr) {
            let value = self.boolean(expr, output)?;
            let name = self.fresh("bool");
            output.push(ir::Op::LetBool { name, value }, expr.span, []);
            return Ok(());
        }
        let value = self.scalar(expr, output)?;
        self.capture(value, expr.span, output);
        Ok(())
    }

    /// Resolve a captured byte operand and require write authority for the destination.
    fn byte_move_parameter(
        &self,
        operand: &ast::Expr,
        writable: bool,
    ) -> Result<usize, Diagnostic> {
        let binding = self.named(operand)?;
        if let Kind::InputBytes { param, access } = binding.kind
            && (!writable || access == ir::Access::Readwrite)
        {
            return Ok(param);
        }
        let expected = if writable {
            "a writable Bytes parameter"
        } else {
            "a readable Bytes parameter"
        };
        Err(Diagnostic::new(
            operand.span,
            format!("core.move_bytes requires {expected}"),
        ))
    }

    /// Lower explicit local rebinding or fixed-shape field/byte mutation.
    /// Binding mutability never grants new write authority or changes root rules.
    fn assign(
        &mut self,
        target: &ast::Expr,
        value: &ast::Expr,
        span: Span,
        output: &mut Block,
    ) -> Result<(), Diagnostic> {
        match &target.kind {
            ast::ExprKind::Name(name) => {
                let binding = self.lookup(name, target.span)?;
                match binding.kind {
                    // Source let is readonly even though generic IR locals support Set.
                    Kind::Scalar { mutable: true } => {
                        let value = self.scalar(value, output)?;
                        output.push(
                            ir::Op::Set {
                                name: binding.ir_name,
                                value,
                            },
                            span,
                            [],
                        );
                    }
                    Kind::Bool { mutable: true } => {
                        let value = self.boolean(value, output)?;
                        output.push(
                            ir::Op::SetBool {
                                name: binding.ir_name,
                                value,
                            },
                            span,
                            [],
                        );
                    }
                    Kind::Reference { mutable: true, .. } => {
                        let source = self.named(value)?;
                        if !matches!(source.kind, Kind::Reference { .. }) {
                            return Err(Diagnostic::new(
                                value.span,
                                "reference rebinding requires an existing named reference",
                            ));
                        }
                        output.push(
                            ir::Op::AssignRef {
                                name: binding.ir_name,
                                source: source.ir_name,
                            },
                            span,
                            [],
                        );
                    }
                    Kind::Enum {
                        kind,
                        mutable: true,
                    } => {
                        let value = self.expect_enum(value, &kind)?;
                        output.push(
                            ir::Op::SetEnum {
                                name: binding.ir_name,
                                value,
                            },
                            span,
                            [],
                        );
                    }
                    _ => {
                        return Err(Diagnostic::new(
                            target.span,
                            "binding cannot be reassigned; use var for scalar or reference locals (owners remain noncopyable)",
                        ));
                    }
                }
            }
            // A field store changes only existing scalar data; shape metadata is inaccessible.
            ast::ExprKind::Member { .. } => {
                let (reference, path, kind) = self.projection(target)?;
                let operation = match kind {
                    ir::FieldKind::U64 => ir::Op::Write {
                        reference,
                        path,
                        value: self.scalar(value, output)?,
                    },
                    ir::FieldKind::Bool => ir::Op::WriteBool {
                        reference,
                        path,
                        value: self.boolean(value, output)?,
                    },
                    ir::FieldKind::Enum(kind) => ir::Op::WriteEnum {
                        reference,
                        path,
                        value: self.expect_enum(value, &kind)?,
                    },
                    _ => {
                        return Err(Diagnostic::new(
                            target.span,
                            "only existing scalar fields can be assigned; record/Bytes shape replacement is unsupported",
                        ));
                    }
                };
                output.push(operation, span, []);
            }
            ast::ExprKind::Index { base, index } => {
                if matches!(base.kind, ast::ExprKind::Bytes(_)) {
                    return Err(Diagnostic::new(
                        base.span,
                        "cannot write through a readonly byte string literal",
                    ));
                }
                if let ast::ExprKind::Name(name) = &base.kind
                    && let binding = self.lookup(name, base.span)?
                    && let Kind::BytesView(access) = binding.kind
                {
                    if access != ir::Access::Readwrite {
                        return Err(Diagnostic::new(
                            base.span,
                            "cannot write through a readonly local byte view",
                        ));
                    }
                    // Keep operand evaluation inside the store so the destination
                    // check precedes any evaluation of the replacement.
                    let previous = std::mem::replace(&mut self.deferred_actual, true);
                    let mut operands = Block::default();
                    let lowered = (|| {
                        Ok(ir::Op::WriteViewByte {
                            name: binding.ir_name,
                            index: self.scalar(index, &mut operands)?,
                            value: self.scalar(value, &mut operands)?,
                        })
                    })();
                    self.deferred_actual = previous;
                    debug_assert!(operands.ops.is_empty());
                    output.push(lowered?, span, []);
                    return Ok(());
                }
                if let ast::ExprKind::Name(name) = &base.kind
                    && let Kind::InputBytes { param, access } = self.lookup(name, base.span)?.kind
                {
                    if access != ir::Access::Readwrite {
                        return Err(Diagnostic::new(
                            base.span,
                            "cannot write through a readonly byte parameter",
                        ));
                    }
                    let index_value = self.scalar(index, output)?;
                    let index = self.capture(index_value, index.span, output);
                    output.push(
                        ir::Op::Assert(ir::Condition {
                            op: ir::Compare::Lt,
                            left: index.clone(),
                            right: ir::Expr::InputLen(param),
                        }),
                        target.span,
                        [],
                    );
                    let value = self.scalar(value, output)?;
                    output.push(
                        ir::Op::WriteInputByte {
                            param,
                            index,
                            value,
                        },
                        span,
                        [],
                    );
                    return Ok(());
                }
                let (reference, path, kind) = self.projection(base)?;
                if kind != ir::FieldKind::Bytes {
                    return Err(Diagnostic::new(
                        base.span,
                        "indexed assignment requires an existing Bytes payload",
                    ));
                }
                let index_value = self.scalar(index, output)?;
                let index = self.capture(index_value, index.span, output);
                // Check destination bounds before RHS argument prefixes. The final byte
                // store retains its own check; normal optimization may prove it redundant.
                output.push(
                    ir::Op::Assert(ir::Condition {
                        op: ir::Compare::Lt,
                        left: index.clone(),
                        right: ir::Expr::BytesLen {
                            reference: reference.clone(),
                            path: path.clone(),
                        },
                    }),
                    target.span,
                    [],
                );
                let value = self.scalar(value, output)?;
                output.push(
                    ir::Op::WriteByte {
                        reference,
                        path,
                        index,
                        value,
                    },
                    span,
                    [],
                );
            }
            _ => {
                return Err(Diagnostic::new(
                    target.span,
                    "unsupported assignment destination",
                ));
            }
        }
        Ok(())
    }

    /// Lower a counted scalar range or captured-prefix batch cursor.
    /// Counts are evaluated once; cursor access must be explicit and grants no uniqueness.
    fn iteration(
        &mut self,
        name: &str,
        access: Option<&str>,
        iterable: &ast::Expr,
        body: &[ast::Statement],
        span: Span,
        output: &mut Block,
    ) -> Result<(), Diagnostic> {
        if let Some(("range", args)) = core_call(iterable) {
            arity(args, 1, iterable.span)?;
            if access.is_some() {
                return Err(Diagnostic::new(
                    iterable.span,
                    "core.range indices are readonly copied scalars; omit reference access",
                ));
            }
            let count = self.scalar(&args[0], output)?;
            self.scopes.push(Default::default());
            let result = (|| {
                let index = self.bind(name, Kind::Scalar { mutable: false }, span)?;
                let body = self.block(body)?;
                Ok((index, body))
            })();
            self.scopes.pop();
            let (index, body) = result?;
            output.push(
                ir::Op::Repeat {
                    count,
                    index,
                    body: body.ops,
                },
                span,
                [body.spans],
            );
            return Ok(());
        }
        let access = match access {
            // No default reference mode is selected by this experimental adapter.
            Some("readonly") => ir::Access::Readonly,
            Some("readwrite") => ir::Access::Readwrite,
            _ => {
                return Err(Diagnostic::new(
                    iterable.span,
                    "batch iteration requires explicit `for readonly name in batch` or `for readwrite ...`",
                ));
            }
        };
        let (batch, record) = self.batch(iterable, access)?;
        self.scopes.push(Default::default());
        let result = (|| {
            let reference = self.bind(
                name,
                Kind::Reference {
                    record,
                    access,
                    mutable: false,
                },
                span,
            )?;
            let body = self.block(body)?;
            Ok((reference, body))
        })();
        self.scopes.pop();
        let (reference, body) = result?;
        output.push(
            ir::Op::ForEach {
                batch,
                reference,
                access,
                body: body.ops,
            },
            span,
            [body.spans],
        );
        Ok(())
    }
}
