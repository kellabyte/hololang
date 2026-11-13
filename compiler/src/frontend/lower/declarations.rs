//! Lexical declarations and explicit destination/acquisition operations.
//!
//! Region/batch owners wrap the remaining lexical statements, rather than being
//! copied into ordinary scalar locals. Append declarations give their
//! constructors the final batch slot. Recovery exposes no unacquired owner.

use super::{Block, Kind, Lowerer, arity, ast, core_call};
use crate::{frontend::byte_ranges, ir, source::Diagnostic};

impl Lowerer {
    /// Lower a declaration and report whether an owner consumed the remaining block.
    /// The initializer precedes source binding insertion; recovery is a separate scope.
    pub(super) fn declaration(
        &mut self,
        statement: &ast::Statement,
        rest: &[ast::Statement],
    ) -> Result<(Block, bool), Diagnostic> {
        let ast::StatementKind::Let {
            name,
            mutable,
            writable_view,
            value,
            otherwise,
        } = &statement.kind
        else {
            unreachable!("declaration dispatch requires let/var");
        };
        let mut output = Block::default();
        if *writable_view && matches!(value.kind, ast::ExprKind::Bytes(_)) {
            return Err(Diagnostic::new(
                value.span,
                "byte string literals are readonly",
            ));
        }
        if *writable_view && (*mutable || !byte_ranges::is_window(value)) {
            return Err(Diagnostic::new(
                statement.span,
                "`let readwrite` requires a `core.prefix` or `core.subrange` initializer",
            ));
        }
        if (matches!(value.kind, ast::ExprKind::Bytes(_)) || byte_ranges::is_window(value))
            && otherwise.is_none()
        {
            if *mutable {
                return Err(Diagnostic::new(
                    statement.span,
                    "byte views cannot be rebound; bind the view with `let`",
                ));
            }
            // Keep source and range evaluation together and in written order.
            let saved = std::mem::replace(&mut self.deferred_actual, true);
            let argument = self.byte_argument(value, *writable_view, &mut output);
            self.deferred_actual = saved;
            let ir::FunctionArgument::Bytes { source, ranges } = argument? else {
                unreachable!("a byte window or literal has a captured source");
            };
            debug_assert!(output.ops.is_empty());
            let access = if *writable_view {
                ir::Access::Readwrite
            } else {
                ir::Access::Readonly
            };
            let name = self.bind(name, Kind::BytesView(access), statement.span)?;
            output.push(
                ir::Op::BindBytes {
                    name,
                    access,
                    source,
                    ranges,
                },
                statement.span,
                [],
            );
            return Ok((output, false));
        }
        if let Some((function, actuals, family)) = self.tagged_target(value) {
            if *mutable || otherwise.is_some() {
                return Err(Diagnostic::new(
                    statement.span,
                    "save a tagged result with an immutable `let`",
                ));
            }
            let family = family.to_owned();
            let args = self.ordinary_arguments(function, actuals, value.span)?;
            let name = self.bind(name, Kind::SavedResult(family), statement.span)?;
            output.push(
                ir::Op::BindResult {
                    name,
                    function: function.to_owned(),
                    args,
                },
                statement.span,
                [],
            );
            return Ok((output, false));
        }
        // Typed append publishes or rolls back before any selected arm runs.
        // Bind the decision after lowering all arms so none sees its own result.
        if let ast::ExprKind::Handle { subject, arms } = &value.kind
            && let Some(("append", args)) = core_call(subject)
        {
            if self.ordinary_body || *mutable || otherwise.is_some() {
                return Err(Diagnostic::new(
                    statement.span,
                    "typed append requires an entry `let` with `on` arms",
                ));
            }
            let boolean = arms
                .first()
                .is_some_and(|arm| matches!(arm.value.kind, ast::ExprKind::Bool(_)));
            let (call, arms) =
                self.append_decision(args, subject.span, arms, boolean, &mut output)?;
            // Both result types have an immutable checked binding.
            let kind = if boolean {
                Kind::HandledBool
            } else {
                Kind::Scalar { mutable: false }
            };
            let result = self.bind(name, kind, statement.span)?;
            output.push(call.handled(result, arms), statement.span, []);
            return Ok((output, false));
        }
        // A same-family failure leaves the ordinary frame before its success
        // binding is introduced. No owner acquisition or local recovery applies.
        if let ast::ExprKind::Try(subject) = &value.kind {
            if *mutable || otherwise.is_some() {
                return Err(Diagnostic::new(
                    statement.span,
                    "bind a propagated scalar with `let` and `try`",
                ));
            }
            let operation = self.try_binding(name, subject, value.span)?;
            output.push(operation, statement.span, []);
            return Ok((output, false));
        }
        if let Some((operation, args)) = core_call(value) {
            // Helpers work on existing argument storage. Owners and hosted
            // operations are entry-only. Element borrows use a checked batch view.
            if self.ordinary_body
                && matches!(
                    operation,
                    "region"
                        | "batch"
                        | "wait_input"
                        | "send"
                        | "wait_output"
                        | "next_session"
                        | "close"
                )
            {
                return Err(Diagnostic::new(
                    value.span,
                    format!("core.{operation} is unavailable in ordinary borrowed functions"),
                ));
            }
            // Owner declarations create no implicit copies or hidden fallback allocation.
            if operation == "region" {
                arity(args, 1, value.span)?;
                if *mutable {
                    return Err(Diagnostic::new(
                        statement.span,
                        "region owners cannot be rebound; use let",
                    ));
                }
                let recovery = otherwise.as_ref().ok_or_else(|| {
                    Diagnostic::new(
                        statement.span,
                        "core.region requires explicit `else { ... }` acquisition recovery",
                    )
                })?;
                let capacity = self.scalar(&args[0], &mut output)?;
                // Recovery has only enclosing bindings: failed acquisition must
                // not expose this owner or any of its success-only tail. A
                // handler may finish normally, retry in its own nested scope,
                // or terminate entry through the existing cleanup path.
                let failure = self.scoped(recovery)?;
                let ir_name = self.bind(name, Kind::Region, statement.span)?;
                let body = self.block(rest)?;
                output.push(
                    ir::Op::Region {
                        name: ir_name,
                        capacity,
                        fail_acquire: false,
                        body: body.ops,
                        on_failure: failure.ops,
                    },
                    statement.span,
                    [body.spans, failure.spans],
                );
                // Both paths finish this owner's lexical tail. Falling through
                // recovery skips the success tail and resumes after the enclosing
                // block; it never resumes statements that require a failed owner.
                return Ok((output, true));
            }
            if otherwise.is_some() {
                return Err(Diagnostic::new(
                    statement.span,
                    "else recovery is supported only on core.region acquisition",
                ));
            }
            // Host calls copy a status to an immutable binding. Their distinct
            // receiver and range rules live with the hosted lowering.
            if matches!(
                operation,
                "send" | "wait_input" | "wait_output" | "next_session" | "close"
            ) {
                if *mutable {
                    return Err(Diagnostic::new(
                        statement.span,
                        format!("bind core.{operation}'s status with let"),
                    ));
                }
                let output =
                    self.hosted_status_binding(operation, args, name, statement.span, value.span)?;
                return Ok((output, false));
            }
            if operation == "batch" {
                arity(args, 2, value.span)?;
                if *mutable {
                    return Err(Diagnostic::new(
                        statement.span,
                        "batch owners cannot be rebound; use let",
                    ));
                }
                let region = self.region(&args[0])?;
                let ast::ExprKind::Name(record) = &args[1].kind else {
                    return Err(Diagnostic::new(
                        args[1].span,
                        "core.batch requires a named record schema",
                    ));
                };
                if !self
                    .definitions
                    .records
                    .iter()
                    .any(|schema| schema.name == *record)
                {
                    return Err(Diagnostic::new(
                        args[1].span,
                        format!("unknown record schema `{record}`"),
                    ));
                }
                let ir_name = self.bind(
                    name,
                    Kind::Batch {
                        record: record.clone(),
                    },
                    statement.span,
                )?;
                let body = self.block(rest)?;
                output.push(
                    ir::Op::Batch {
                        name: ir_name,
                        region,
                        record: record.clone(),
                        body: body.ops,
                    },
                    statement.span,
                    [body.spans],
                );
                return Ok((output, true));
            }
            // Append reports a status instead of concealing a construction failure.
            if operation == "append" {
                let call = self.append_call(args, value.span, &mut output)?;
                // The numeric adapter cannot discard a typed rejection payload.
                if call.family.is_some() {
                    return Err(Diagnostic::new(
                        value.span,
                        "typed append requires immediate `on` arms",
                    ));
                }
                let status = self.bind(name, Kind::Scalar { mutable: *mutable }, statement.span)?;
                output.push(
                    ir::Op::Append {
                        batch: call.batch,
                        producer: call.producer,
                        args: call.args,
                        status,
                    },
                    statement.span,
                    [],
                );
                return Ok((output, false));
            }
            // Borrow/alias access is explicit; binding rebinding is a separate permission.
            if matches!(operation, "borrow_readonly" | "borrow_readwrite") {
                arity(args, 2, value.span)?;
                let access = access(operation);
                let (batch, record) = self.batch(&args[0], access)?;
                let index = self.scalar(&args[1], &mut output)?;
                let name = self.bind(
                    name,
                    Kind::Reference {
                        record,
                        access,
                        mutable: *mutable,
                    },
                    statement.span,
                )?;
                output.push(
                    ir::Op::Borrow {
                        name,
                        batch,
                        index,
                        access,
                    },
                    statement.span,
                    [],
                );
                return Ok((output, false));
            }
            if matches!(operation, "alias_readonly" | "alias_readwrite") {
                arity(args, 1, value.span)?;
                let source = self.named(&args[0])?;
                let Kind::Reference { record, .. } = source.kind else {
                    return Err(Diagnostic::new(
                        args[0].span,
                        "alias requires a live reference",
                    ));
                };
                let access = access(operation);
                let name = self.bind(
                    name,
                    Kind::Reference {
                        record,
                        access,
                        mutable: *mutable,
                    },
                    statement.span,
                )?;
                output.push(
                    ir::Op::Alias {
                        name,
                        source: source.ir_name,
                        access,
                    },
                    statement.span,
                    [],
                );
                return Ok((output, false));
            }
        }
        if otherwise.is_some() {
            return Err(Diagnostic::new(
                statement.span,
                "else recovery is supported only on core.region acquisition",
            ));
        }
        // A reference name makes an explicit same-access alias; no aggregate bytes move.
        if let ast::ExprKind::Name(source_name) = &value.kind {
            let source = self.lookup(source_name, value.span)?;
            if let Kind::Reference { record, access, .. } = source.kind {
                let name = self.bind(
                    name,
                    Kind::Reference {
                        record,
                        access,
                        mutable: *mutable,
                    },
                    statement.span,
                )?;
                output.push(
                    ir::Op::Alias {
                        name,
                        source: source.ir_name,
                        access,
                    },
                    statement.span,
                    [],
                );
                return Ok((output, false));
            }
        }
        // Literal boolean arms infer an immutable condition binding. Scalar arms
        // keep the existing u64 path, and mixed types are diagnosed arm-locally.
        if let ast::ExprKind::Handle { subject, arms } = &value.kind
            && arms
                .first()
                .is_some_and(|arm| matches!(arm.value.kind, ast::ExprKind::Bool(_)))
        {
            if *mutable {
                return Err(Diagnostic::new(
                    statement.span,
                    "bind a handled boolean with `let`",
                ));
            }
            let value = self.handle_result(subject, arms, true)?;
            let name = self.bind(name, Kind::HandledBool, statement.span)?;
            output.push(ir::Op::Let { name, value }, statement.span, []);
            return Ok((output, false));
        }
        if self.is_bool(value) {
            if *mutable {
                return Err(Diagnostic::new(statement.span, "bind a bool with `let`"));
            }
            let value = self.boolean(value, &mut output)?;
            let name = self.bind(name, Kind::Bool, statement.span)?;
            output.push(ir::Op::LetBool { name, value }, statement.span, []);
            return Ok((output, false));
        }
        let value = self.scalar(value, &mut output)?;
        let name = self.bind(name, Kind::Scalar { mutable: *mutable }, statement.span)?;
        output.push(ir::Op::Let { name, value }, statement.span, []);
        Ok((output, false))
    }
}

/// Map the explicit core capability spelling without asserting alias uniqueness.
fn access(operation: &str) -> ir::Access {
    // The caller restricts the operation set to exactly the four access-bearing calls.
    if operation.ends_with("readwrite") {
        ir::Access::Readwrite
    } else {
        ir::Access::Readonly
    }
}
