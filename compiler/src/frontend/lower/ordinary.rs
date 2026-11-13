//! Lower synchronous borrowed functions and their typed actual arguments.
//!
//! A call retains its own argument evaluation order. Byte actuals carry source
//! identity and any checked windows. No payload or state record is
//! copied by this adapter. The IR verifier validates effects and live roots.

use super::{Block, Kind, Lowerer, arity, ast, terminates};
use crate::frontend::byte_ranges;
use crate::{
    ir,
    source::{Diagnostic, Span},
};
use std::collections::HashMap;

impl Lowerer {
    /// Lower a checked header's body under fresh formal bindings.
    /// Parameter names and the exact scalar or typed result belong to this body only.
    pub(super) fn ordinary_definition(
        &mut self,
        function: &ast::Function,
    ) -> Result<(ir::Function, Vec<Span>), Diagnostic> {
        // Counted loops can run zero times. The existing conservative structural
        // test only accepts a return when every fallthrough path has one.
        if !terminates(&function.body) {
            return Err(Diagnostic::new(
                function.span,
                "ordinary borrowed functions must return their declared result on every normal path",
            ));
        }
        self.scopes.push(HashMap::new());
        self.ordinary_body = true;
        self.returns_bool = self.definitions.bools.contains(&function.name);
        self.result = self.definitions.results.get(&function.name).cloned();
        let result = (|| {
            let kinds = self.definitions.ordinary[&function.name].clone();
            let mut params = Vec::with_capacity(kinds.len());
            // Capture only compiler metadata here. Native descriptors are supplied by calls.
            for (param, (argument, kind)) in function.args.iter().zip(kinds).enumerate() {
                let binding = match &kind {
                    ir::FunctionParameterKind::U64 => Kind::Scalar { mutable: false },
                    ir::FunctionParameterKind::Bool => Kind::Bool { mutable: false },
                    ir::FunctionParameterKind::Bytes => Kind::InputBytes {
                        param,
                        access: ir::Access::Readonly,
                    },
                    ir::FunctionParameterKind::WritableBytes => Kind::InputBytes {
                        param,
                        access: ir::Access::Readwrite,
                    },
                    ir::FunctionParameterKind::State(record) => Kind::Reference {
                        record: record.clone(),
                        access: ir::Access::Readwrite,
                        mutable: false,
                    },
                    ir::FunctionParameterKind::Batch { record, access } => Kind::BatchView {
                        record: record.clone(),
                        access: *access,
                    },
                };
                let name = self.bind(&argument.name, binding, argument.span)?;
                params.push(ir::FunctionParameter { name, kind });
            }
            let body = self.block(&function.body)?;
            Ok((
                ir::Function {
                    name: function.name.clone(),
                    params,
                    result: self.result.clone(),
                    returns_bool: self.returns_bool,
                    body: body.ops,
                },
                body.spans,
            ))
        })();
        // Even a rejected body must leave no formal binding in another function.
        self.ordinary_body = false;
        self.returns_bool = false;
        self.result = None;
        self.scopes.pop();
        result
    }

    /// Build one direct call with ordered actuals and no enclosing operation prelude.
    /// The native evaluator checks byte windows before continuing to later actuals.
    pub(super) fn ordinary_call(
        &mut self,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<ir::Expr, Diagnostic> {
        if self.definitions.bools.contains(name) {
            return Err(Diagnostic::new(
                span,
                "a bool return cannot be used as a u64",
            ));
        }
        // A tagged result must be handled before a scalar payload can be read.
        if let Some(result) = self.definitions.results.get(name) {
            if result.family == super::functions::OPTION_FAMILY {
                return Err(Diagnostic::new(
                    span,
                    "save the `Option<u64>` call with `let`, handle it with `on Some` and `on None`, or return it from an optional function",
                ));
            }
            return Err(Diagnostic::new(
                span,
                "save the call with `let`, handle it with `on`, or return its exact Result",
            ));
        }
        Ok(ir::Expr::Call {
            function: name.to_owned(),
            args: self.ordinary_arguments(name, args, span)?,
        })
    }

    /// Lower once-only actual evaluation for an ordinary call.
    /// The returned arguments contain their own evaluation and cannot escape as preludes.
    pub(super) fn ordinary_arguments(
        &mut self,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<Vec<ir::FunctionArgument>, Diagnostic> {
        let params = self.definitions.ordinary[name].clone();
        arity(args, params.len(), span)?;
        let previous = std::mem::replace(&mut self.deferred_actual, true);
        let result = (|| {
            let mut actuals = Vec::with_capacity(args.len());
            let mut output = Block::default();
            // Each child remains within the call node. In particular, a later
            // scalar helper must not execute before an earlier byte range check.
            for (actual, param) in args.iter().zip(params) {
                let argument = match param {
                    ir::FunctionParameterKind::U64 => {
                        ir::FunctionArgument::Scalar(self.scalar(actual, &mut output)?)
                    }
                    ir::FunctionParameterKind::Bool => {
                        ir::FunctionArgument::Bool(self.boolean(actual, &mut output)?)
                    }
                    ir::FunctionParameterKind::State(record) => {
                        let binding = self.named(actual)?;
                        if !matches!(binding.kind, Kind::Reference {
                            record: ref actual_record,
                            access: ir::Access::Readwrite,
                            ..
                        } if *actual_record == record)
                        {
                            return Err(Diagnostic::new(
                                actual.span,
                                format!("expected a live readwrite reference to `{record}` state"),
                            ));
                        }
                        ir::FunctionArgument::State(binding.ir_name)
                    }
                    ir::FunctionParameterKind::Batch { record, access } => {
                        let (name, actual_record) = self.batch(actual, access)?;
                        if actual_record != record {
                            return Err(Diagnostic::new(
                                actual.span,
                                format!("expected a batch of `{record}`, found `{actual_record}`"),
                            ));
                        }
                        ir::FunctionArgument::Batch(name)
                    }
                    ir::FunctionParameterKind::Bytes | ir::FunctionParameterKind::WritableBytes => {
                        self.byte_argument(
                            actual,
                            param == ir::FunctionParameterKind::WritableBytes,
                            &mut output,
                        )?
                    }
                };
                actuals.push(argument);
            }
            // This invariant guards against new scalar lowering that would move
            // an argument's work outside the ordered native call.
            assert!(
                output.ops.is_empty(),
                "ordinary actuals must remain expressions"
            );
            Ok(actuals)
        })();
        self.deferred_actual = previous;
        result
    }

    /// Lower call-scoped byte windows from the original source outward.
    /// Each window's operands stay in the call, where its bounds precede outer work.
    pub(super) fn byte_argument(
        &mut self,
        actual: &ast::Expr,
        writable: bool,
        output: &mut Block,
    ) -> Result<ir::FunctionArgument, Diagnostic> {
        let (root, windows) = byte_ranges::split(actual)?;
        let source = if let Some(name) = self.received_argument(root)? {
            if writable {
                return Err(Diagnostic::new(root.span, "received view is readonly"));
            }
            if windows.is_empty() {
                return Ok(ir::FunctionArgument::Received(name));
            }
            ir::BytesSource::Received(name)
        } else {
            self.bytes_source(root, writable)?
        };
        let mut ranges = Vec::with_capacity(windows.len());
        for window in windows {
            self.charge(window.span)?;
            let offset = window
                .offset
                .map(|value| self.scalar(value, output))
                .transpose()?;
            let length = self.scalar(window.length, output)?;
            ranges.push(ir::ByteRange { offset, length });
        }
        Ok(ir::FunctionArgument::Bytes { source, ranges })
    }

    /// Resolve the source and access of a byte argument without observing its payload.
    /// A forwarded formal retains its captured extent and caller root in the verifier.
    fn bytes_source(
        &self,
        expr: &ast::Expr,
        writable: bool,
    ) -> Result<ir::BytesSource, Diagnostic> {
        if let ast::ExprKind::Bytes(bytes) = &expr.kind {
            if writable {
                return Err(Diagnostic::new(
                    expr.span,
                    "byte string literals are readonly",
                ));
            }
            return Ok(ir::BytesSource::Literal(bytes.clone()));
        }
        // A byte formal or local window already names a captured descriptor.
        if let ast::ExprKind::Name(name) = &expr.kind {
            let binding = self.lookup(name, expr.span)?;
            match binding.kind {
                Kind::InputBytes { param, access } => {
                    if writable && access != ir::Access::Readwrite {
                        return Err(Diagnostic::new(
                            expr.span,
                            "expected a readwrite Bytes parameter",
                        ));
                    }
                    return Ok(ir::BytesSource::Param(param));
                }
                Kind::BytesView(access) => {
                    if writable && access != ir::Access::Readwrite {
                        return Err(Diagnostic::new(expr.span, "local byte view is readonly"));
                    }
                    return Ok(ir::BytesSource::Local(binding.ir_name));
                }
                _ => {}
            }
        }
        let (reference, path, kind) = self.projection(expr)?;
        // Only complete initialized fields may form new byte sources.
        if kind != ir::FieldKind::Bytes {
            return Err(Diagnostic::new(
                expr.span,
                "expected a live Bytes field or a byte parameter",
            ));
        }
        if writable {
            let mut root = expr;
            while let ast::ExprKind::Member { base, .. } = &root.kind {
                root = base;
            }
            if !matches!(
                self.named(root)?.kind,
                Kind::Reference {
                    access: ir::Access::Readwrite,
                    ..
                }
            ) {
                return Err(Diagnostic::new(
                    expr.span,
                    "expected a readwrite reference to the Bytes field",
                ));
            }
        }
        Ok(ir::BytesSource::Field { reference, path })
    }
}
