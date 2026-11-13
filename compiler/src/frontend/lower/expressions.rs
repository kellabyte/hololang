//! Scalar observations, projections, and once-only ordered source-call evaluation.
//!
//! Pure templates keep eager actuals, including within typed ordinary calls.
//! Readonly means path permission, not frozen memory or LLVM alias independence.
//! Numeric overflow defaults remain unavailable; explicit wrapping calls select
//! existing IR operations without silently choosing future language semantics.

use super::{Block, Kind, Lowerer, arity, ast, core_call, functions};
use crate::frontend::byte_ranges;
use crate::{
    ir,
    source::{Diagnostic, Span},
};

impl Lowerer {
    /// Lower one unsigned scalar, appending any ordered argument captures to output.
    /// Reject owners, aggregate copies, and unavailable numeric semantics explicitly.
    pub(super) fn scalar(
        &mut self,
        expr: &ast::Expr,
        output: &mut Block,
    ) -> Result<ir::Expr, Diagnostic> {
        self.charge(expr.span)?;
        match &expr.kind {
            // Literals copy exact u64 values; only initialized scalar names may be read.
            ast::ExprKind::Number(value) => Ok(ir::Expr::Const(*value)),
            ast::ExprKind::Bytes(_) => Err(Diagnostic::new(
                expr.span,
                "a byte string literal cannot be used as a u64",
            )),
            ast::ExprKind::Name(name) => {
                let binding = self.lookup(name, expr.span)?;
                // A byte descriptor cannot leave the call in a scalar result or local.
                if matches!(
                    binding.kind,
                    Kind::InputBytes { .. } | Kind::ReceivedBytes | Kind::BytesView(_)
                ) {
                    return Err(Diagnostic::new(
                        expr.span,
                        "a byte borrow cannot become a scalar or escape its scope",
                    ));
                }
                // A fallback binding retains its nominal case identity and has no scalar conversion.
                if matches!(binding.kind, Kind::Error) {
                    return Err(Diagnostic::new(
                        expr.span,
                        "a typed error cannot be used as a u64",
                    ));
                }
                if matches!(binding.kind, Kind::Bool | Kind::HandledBool) {
                    return Err(Diagnostic::new(
                        expr.span,
                        "a bool binding cannot be used as a u64",
                    ));
                }
                if let Kind::SavedResult(family) = &binding.kind {
                    let message = if family == super::functions::OPTION_FAMILY {
                        "handle the saved `Option<u64>` with `on Some` and `on None`"
                    } else {
                        "handle the saved `Result` with `on` arms before using its payload"
                    };
                    return Err(Diagnostic::new(expr.span, message));
                }
                if !matches!(binding.kind, Kind::Scalar { .. }) {
                    return Err(Diagnostic::new(
                        expr.span,
                        "expected a copied u64 scalar; owners and references are not implicit scalar values",
                    ));
                }
                Ok(ir::Expr::Local(binding.ir_name))
            }
            // Field paths are resolved statically; only u64 fields become scalar reads.
            ast::ExprKind::Member { .. } => {
                let (reference, path, kind) = self.projection(expr)?;
                if kind != ir::FieldKind::U64 {
                    return Err(Diagnostic::new(
                        expr.span,
                        "expected a u64 field; aggregate values require a destination context",
                    ));
                }
                Ok(ir::Expr::Read { reference, path })
            }
            // Byte reads retain the backend bounds check and evaluate the index once.
            ast::ExprKind::Index { base, index } => {
                if let ast::ExprKind::Bytes(bytes) = &base.kind {
                    return Ok(ir::Expr::LiteralByte {
                        bytes: bytes.clone(),
                        index: Box::new(self.scalar(index, output)?),
                    });
                }
                // A byte formal uses its captured logical extent instead of a field header.
                if let ast::ExprKind::Name(name) = &base.kind {
                    let binding = self.lookup(name, base.span)?;
                    match binding.kind {
                        Kind::InputBytes { param, .. } => {
                            return Ok(ir::Expr::InputByte {
                                param,
                                index: Box::new(self.scalar(index, output)?),
                            });
                        }
                        Kind::BytesView(_) => {
                            return Ok(ir::Expr::ViewByte {
                                name: binding.ir_name,
                                index: Box::new(self.scalar(index, output)?),
                            });
                        }
                        _ => {}
                    }
                }
                let (reference, path, kind) = self.projection(base)?;
                if kind != ir::FieldKind::Bytes {
                    return Err(Diagnostic::new(
                        base.span,
                        "only Bytes payloads support indexing in this subset",
                    ));
                }
                let index = self.scalar(index, output)?;
                Ok(ir::Expr::Byte {
                    reference,
                    path,
                    index: Box::new(index),
                })
            }
            // Calls resolve statically to core operations, ordinary signatures, or pure templates.
            ast::ExprKind::Call { callee, args } => {
                if let Some((name, args)) = core_call(expr) {
                    return self.core_scalar(name, args, expr.span, output);
                }
                let ast::ExprKind::Name(name) = &callee.kind else {
                    return Err(Diagnostic::new(
                        callee.span,
                        "only statically named functions are callable",
                    ));
                };
                if self.definitions.ordinary.contains_key(name) {
                    return self.ordinary_call(name, args, expr.span);
                }
                let Some(template) = self.definitions.scalars.get(name) else {
                    return Err(Diagnostic::new(
                        callee.span,
                        format!(
                            "unknown scalar helper `{name}`; record constructors require a checked destination such as core.append or core.construct"
                        ),
                    ));
                };
                arity(args, template.params, expr.span)?;
                // Capture arguments before borrowing the template, preventing repeated effects
                // if a helper uses one parameter several times or not at all.
                let arguments = self.arguments(args, output)?;
                let template = &self.definitions.scalars[name];
                // Inside a typed actual, an eager scalar frame evaluates the
                // actuals in place before using or discarding their values.
                if self.deferred_actual {
                    let identities: Vec<_> = (0..arguments.len()).map(ir::Expr::Param).collect();
                    let body = functions::instantiate(
                        &template.value,
                        &identities,
                        expr.span,
                        &mut self.budget,
                    )?;
                    return Ok(ir::Expr::Apply {
                        args: arguments,
                        body: Box::new(body),
                    });
                }
                functions::instantiate(&template.value, &arguments, expr.span, &mut self.budget)
            }
            // Parsing an infix operator does not silently settle overflow/truth conventions.
            ast::ExprKind::Binary { .. } if functions::boolean_syntax(expr) => Err(
                Diagnostic::new(expr.span, "a bool expression cannot be used as a u64"),
            ),
            ast::ExprKind::Binary { .. } => Err(Diagnostic::new(
                expr.span,
                "infix scalar arithmetic is unsupported while numeric defaults remain open; use explicit core.add_wrapping, core.sub_wrapping, core.mul_wrapping, core.and, or core.xor",
            )),
            ast::ExprKind::Not { .. } | ast::ExprKind::Bool(_) => Err(Diagnostic::new(
                expr.span,
                "boolean values cannot be used as a u64",
            )),
            // A bound family root is only a path component in an error case.
            ast::ExprKind::ErrorFamilyRoot { .. } => Err(Diagnostic::new(
                expr.span,
                "an error family cannot be used as a u64",
            )),
            ast::ExprKind::Record { .. } => Err(Diagnostic::new(
                expr.span,
                "record literals cannot initialize scalar locals; use a destination constructor with core.construct for a scoped record",
            )),
            ast::ExprKind::Handle { subject, arms } => self.handle_result(subject, arms, false),
            ast::ExprKind::Ok(_) | ast::ExprKind::Error { .. } => Err(Diagnostic::new(
                expr.span,
                "result constructors require a declared Result return type",
            )),
            ast::ExprKind::Try(_) => Err(Diagnostic::new(
                expr.span,
                "`try` requires an immutable binding in a typed-result function",
            )),
        }
    }

    /// Keep every scalar actual once in written order, including unused parameters.
    /// An enclosing typed actual uses expression evaluation rather than hoisted captures.
    pub(super) fn arguments(
        &mut self,
        args: &[ast::Expr],
        output: &mut Block,
    ) -> Result<Vec<ir::Expr>, Diagnostic> {
        let mut values = Vec::with_capacity(args.len());
        // A binary expression or eager Apply will evaluate its children in this
        // order. The existing entry path materializes each child immediately.
        for argument in args {
            let value = self.scalar(argument, output)?;
            let value = if self.deferred_actual {
                value
            } else {
                self.capture(value, argument.span, output)
            };
            values.push(value);
        }
        Ok(values)
    }

    /// Lower ordered constructor actuals without copying borrowed payloads.
    /// Window checks can trap, so a call with windows keeps all actuals together.
    /// Whole-field append retains eager scalar captures because its descriptor
    /// loads are immutable and cannot trap. Scoped construction already defers.
    pub(super) fn producer_arguments(
        &mut self,
        args: &[ast::Expr],
        parameters: &[ir::Parameter],
        output: &mut Block,
    ) -> Result<Vec<ir::Argument>, Diagnostic> {
        let saved = self.deferred_actual;
        self.deferred_actual |= args.iter().zip(parameters).any(|(actual, parameter)| {
            *parameter == ir::Parameter::Bytes && byte_ranges::is_window(actual)
        });
        let arguments = self.producer_actuals(args, parameters, output);
        self.deferred_actual = saved;
        arguments
    }

    /// Lower each actual according to its destination formal and capture mode.
    fn producer_actuals(
        &mut self,
        args: &[ast::Expr],
        parameters: &[ir::Parameter],
        output: &mut Block,
    ) -> Result<Vec<ir::Argument>, Diagnostic> {
        let mut values = Vec::with_capacity(args.len());
        // Arity is checked by declaration lowering; every source actual has one formal.
        for (argument, parameter) in args.iter().zip(parameters) {
            let value = match parameter {
                ir::Parameter::U64 => {
                    let value = self.scalar(argument, output)?;
                    // Append retains its existing eager locals. Construction
                    // must schedule the capture after resolving the region tail.
                    ir::Argument::Scalar(if self.deferred_actual {
                        value
                    } else {
                        self.capture(value, argument.span, output)
                    })
                }
                ir::Parameter::Bytes => {
                    self.charge(argument.span)?;
                    match self.byte_argument(argument, false, output)? {
                        ir::FunctionArgument::Received(name) => ir::Argument::Received(name),
                        ir::FunctionArgument::Bytes { source, ranges } if ranges.is_empty() => {
                            ir::Argument::Bytes(source)
                        }
                        ir::FunctionArgument::Bytes { source, ranges } => {
                            ir::Argument::ByteView { source, ranges }
                        }
                        _ => unreachable!("byte lowering returns a byte argument"),
                    }
                }
            };
            values.push(value);
        }
        Ok(values)
    }

    /// Identify a direct lexical refill argument without reading the payload.
    /// A bare name outside its lifetime reports an error. Other operand forms
    /// continue through the caller's ordinary byte-argument rules.
    pub(super) fn received_argument(&self, expr: &ast::Expr) -> Result<Option<String>, Diagnostic> {
        // A projection or call cannot fabricate a received capability.
        if let ast::ExprKind::Name(name) = &expr.kind
            && let binding = self.lookup(name, expr.span)?
            && matches!(binding.kind, Kind::ReceivedBytes)
        {
            return Ok(Some(binding.ir_name));
        }
        Ok(None)
    }

    /// Lower a supported core scalar observation or explicit arithmetic call.
    /// This never introduces a target allocator, indirect call, or aggregate copy.
    fn core_scalar(
        &mut self,
        name: &str,
        args: &[ast::Expr],
        span: Span,
        output: &mut Block,
    ) -> Result<ir::Expr, Diagnostic> {
        let binary = match name {
            // Explicit operation names carry the arithmetic policy, independent of future defaults.
            "add_wrapping" => Some(ir::Binary::AddWrapping),
            "sub_wrapping" => Some(ir::Binary::SubWrapping),
            "mul_wrapping" => Some(ir::Binary::MulWrapping),
            "and" => Some(ir::Binary::And),
            "xor" => Some(ir::Binary::Xor),
            _ => None,
        };
        if let Some(op) = binary {
            arity(args, 2, span)?;
            let mut values = self.arguments(args, output)?.into_iter();
            return Ok(ir::Expr::binary(
                op,
                values.next().expect("arity checked"),
                values.next().expect("arity checked"),
            ));
        }
        match name {
            // Lengths are metadata observations, never scans or implicit serialization.
            "len" => {
                arity(args, 1, span)?;
                if let ast::ExprKind::Bytes(bytes) = &args[0].kind {
                    return Ok(ir::Expr::Const(bytes.len() as u64));
                }
                if let ast::ExprKind::Name(name) = &args[0].kind {
                    let binding = self.lookup(name, args[0].span)?;
                    // These named values have a count or extent without a field projection.
                    match binding.kind {
                        Kind::Batch { .. } | Kind::BatchView { .. } => {
                            return Ok(ir::Expr::BatchLen(binding.ir_name));
                        }
                        Kind::InputBytes { param, .. } => return Ok(ir::Expr::InputLen(param)),
                        Kind::BytesView(_) | Kind::ReceivedBytes => {
                            return Ok(ir::Expr::ViewLen(binding.ir_name));
                        }
                        _ => {}
                    }
                }
                let (reference, path, kind) = self.projection(&args[0])?;
                if kind != ir::FieldKind::Bytes {
                    return Err(Diagnostic::new(
                        args[0].span,
                        "core.len requires a batch or Bytes field",
                    ));
                }
                Ok(ir::Expr::BytesLen { reference, path })
            }
            "used" => {
                arity(args, 1, span)?;
                Ok(ir::Expr::Used(self.region(&args[0])?))
            }
            // Harness observations cannot manufacture pointers or enter pure constructors.
            "address" => {
                arity(args, 1, span)?;
                let binding = self.named(&args[0])?;
                if !matches!(binding.kind, Kind::Reference { .. }) {
                    return Err(Diagnostic::new(
                        args[0].span,
                        "core.address requires a live reference",
                    ));
                }
                Ok(ir::Expr::Address(binding.ir_name))
            }
            "now" => {
                arity(args, 0, span)?;
                // Harness clock effects are outside the ordinary helper subset.
                if self.ordinary_body {
                    return Err(Diagnostic::new(
                        span,
                        "core.now is a harness-only entry operation",
                    ));
                }
                Ok(ir::Expr::Nanoseconds)
            }
            _ => Err(Diagnostic::new(
                span,
                format!("unsupported scalar core operation `{name}`"),
            )),
        }
    }

    /// Recognize the supported boolean-valued initializers and calls.
    pub(super) fn is_bool(&self, expr: &ast::Expr) -> bool {
        match &expr.kind {
            ast::ExprKind::Name(name) => self
                .lookup(name, expr.span)
                .is_ok_and(|binding| matches!(binding.kind, Kind::Bool | Kind::HandledBool)),
            ast::ExprKind::Call { .. } => functions::named_call(expr)
                .is_some_and(|(name, _)| self.definitions.bools.contains(name)),
            _ => functions::boolean_syntax(expr),
        }
    }

    /// Lower a typed boolean without admitting numeric values as truth tests.
    pub(super) fn boolean(
        &mut self,
        expr: &ast::Expr,
        output: &mut Block,
    ) -> Result<ir::BoolExpr, Diagnostic> {
        self.charge(expr.span)?;
        match &expr.kind {
            ast::ExprKind::Bool(value) => return Ok(ir::BoolExpr::Const(*value)),
            ast::ExprKind::Name(name) => {
                let binding = self.lookup(name, expr.span)?;
                match binding.kind {
                    Kind::Bool => return Ok(ir::BoolExpr::Local(binding.ir_name)),
                    Kind::HandledBool => {
                        return Ok(ir::BoolExpr::Compare(handled_truth(ir::Expr::Local(
                            binding.ir_name,
                        ))));
                    }
                    _ => {}
                }
            }
            ast::ExprKind::Not { value, .. } => {
                let value = self.deferred_boolean(value)?;
                return Ok(ir::BoolExpr::Not(Box::new(value)));
            }
            ast::ExprKind::Binary {
                op, left, right, ..
            } => {
                if matches!(op, ast::BinaryOp::LogicalAnd | ast::BinaryOp::LogicalOr) {
                    let left = Box::new(self.deferred_boolean(left)?);
                    let right = Box::new(self.deferred_boolean(right)?);
                    return Ok(if *op == ast::BinaryOp::LogicalAnd {
                        ir::BoolExpr::And(left, right)
                    } else {
                        ir::BoolExpr::Or(left, right)
                    });
                }
                if let Some(op) = functions::comparison(*op) {
                    return self
                        .scalar_comparison(op, left, right, output)
                        .map(ir::BoolExpr::Compare);
                }
            }
            ast::ExprKind::Call { .. } => {
                if let Some((name, args)) = functions::named_call(expr)
                    && self.definitions.bools.contains(name)
                {
                    return Ok(ir::BoolExpr::Call {
                        function: name.to_owned(),
                        args: self.ordinary_arguments(name, args, expr.span)?,
                    });
                }
            }
            ast::ExprKind::Handle { subject, arms } => {
                let value = self.handle_result(subject, arms, true)?;
                return Ok(ir::BoolExpr::Compare(handled_truth(value)));
            }
            _ => {}
        }
        Err(Diagnostic::new(expr.span, "expected a bool expression"))
    }

    /// Keep comparison operands and call actuals inside conditional evaluation.
    fn deferred_boolean(&mut self, expr: &ast::Expr) -> Result<ir::BoolExpr, Diagnostic> {
        let previous = std::mem::replace(&mut self.deferred_actual, true);
        let mut output = Block::default();
        let value = self.boolean(expr, &mut output);
        self.deferred_actual = previous;
        debug_assert!(output.ops.is_empty());
        value
    }

    /// Lower a boolean condition or an unsigned comparison.
    pub(super) fn condition(
        &mut self,
        expr: &ast::Expr,
        output: &mut Block,
    ) -> Result<ir::Condition, Diagnostic> {
        match &expr.kind {
            // Literal and handled conditions retain their existing lowering.
            ast::ExprKind::Bool(value) => Ok(ir::Condition {
                op: ir::Compare::Eq,
                left: ir::Expr::Const(u64::from(*value)),
                right: ir::Expr::Const(1),
            }),
            // Earlier result handlers leave a canonical zero-or-one decision.
            ast::ExprKind::Name(name) => {
                let binding = self.lookup(name, expr.span)?;
                if matches!(binding.kind, Kind::Bool) {
                    self.charge(expr.span)?;
                    return Ok(ir::Condition::truth(ir::BoolExpr::Local(binding.ir_name)));
                }
                if !matches!(binding.kind, Kind::HandledBool) {
                    return Err(Diagnostic::new(
                        expr.span,
                        "expected a comparison or boolean condition",
                    ));
                }
                Ok(handled_truth(ir::Expr::Local(binding.ir_name)))
            }
            ast::ExprKind::Binary {
                op, left, right, ..
            } => {
                if let Some(op) = functions::comparison(*op) {
                    self.scalar_comparison(op, left, right, output)
                } else if self.is_bool(expr) {
                    self.boolean(expr, output).map(ir::Condition::truth)
                } else {
                    Err(Diagnostic::new(
                        expr.span,
                        "expected a comparison or boolean condition",
                    ))
                }
            }
            _ if self.is_bool(expr) => self.boolean(expr, output).map(ir::Condition::truth),
            _ => Err(Diagnostic::new(
                expr.span,
                "expected a comparison or boolean condition",
            )),
        }
    }

    /// Capture the left scalar before the right operand's prelude can run.
    fn scalar_comparison(
        &mut self,
        op: ir::Compare,
        left: &ast::Expr,
        right: &ast::Expr,
        output: &mut Block,
    ) -> Result<ir::Condition, Diagnostic> {
        let left_value = self.scalar(left, output)?;
        let left = if self.deferred_actual {
            left_value
        } else {
            self.capture(left_value, left.span, output)
        };
        let right = self.scalar(right, output)?;
        Ok(ir::Condition { op, left, right })
    }

    /// Resolve a named reference and nested field path to its final logical kind.
    /// Walks iteratively; does not compute pointers or infer uniqueness/packing.
    pub(super) fn projection(
        &self,
        expr: &ast::Expr,
    ) -> Result<(String, Vec<String>, ir::FieldKind), Diagnostic> {
        let mut cursor = expr;
        let mut path = Vec::new();
        // Postfix members form a bounded parser tree; flatten without cloning its subtrees.
        while let ast::ExprKind::Member { base, field } = &cursor.kind {
            path.push(field.clone());
            cursor = base;
        }
        path.reverse();
        let binding = self.named(cursor)?;
        let Kind::Reference { record, .. } = binding.kind else {
            return Err(Diagnostic::new(
                cursor.span,
                "field projection requires a live record reference",
            ));
        };
        let mut kind = ir::FieldKind::Record(record);
        // Every field must exist in its enclosing schema; only nested records permit another hop.
        for field in &path {
            let ir::FieldKind::Record(record) = &kind else {
                return Err(Diagnostic::new(
                    expr.span,
                    "cannot project through a scalar or Bytes field",
                ));
            };
            let schema = self
                .definitions
                .records
                .iter()
                .find(|schema| schema.name == *record)
                .expect("definition lowering validates schema names");
            kind = schema
                .fields
                .iter()
                .find(|item| item.name == *field)
                .map(|item| item.kind.clone())
                .ok_or_else(|| {
                    Diagnostic::new(expr.span, format!("unknown field `{field}` in `{record}`"))
                })?;
        }
        Ok((binding.ir_name, path, kind))
    }
}

/// Earlier result decisions lower to canonical zero or one.
fn handled_truth(value: ir::Expr) -> ir::Condition {
    ir::Condition {
        op: ir::Compare::Eq,
        left: value,
        right: ir::Expr::Const(1),
    }
}
