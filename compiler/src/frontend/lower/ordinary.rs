//! Lower synchronous borrowed functions and their typed actual arguments.
//!
//! A call retains its own argument evaluation order. Byte actuals carry source
//! identity and an optional checked extent. No payload or state record is
//! copied by this adapter. The IR verifier validates effects and live roots.

use super::{Block, Kind, Lowerer, arity, ast, core_call, terminates};
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
        self.result_family = self.definitions.results.get(&function.name).cloned();
        let result = (|| {
            let kinds = self.definitions.ordinary[&function.name].clone();
            let mut params = Vec::with_capacity(kinds.len());
            // Capture only compiler metadata here. Native descriptors are supplied by calls.
            for (param, (argument, kind)) in function.args.iter().zip(kinds).enumerate() {
                let binding = match &kind {
                    ir::FunctionParameterKind::U64 => Kind::Scalar { mutable: false },
                    ir::FunctionParameterKind::Bytes => Kind::InputBytes { param },
                    ir::FunctionParameterKind::State(record) => Kind::Reference {
                        record: record.clone(),
                        access: ir::Access::Readwrite,
                        mutable: false,
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
                    result: self.result_family.clone(),
                    body: body.ops,
                },
                body.spans,
            ))
        })();
        // Even a rejected body must leave no formal binding in another function.
        self.ordinary_body = false;
        self.result_family = None;
        self.scopes.pop();
        result
    }

    /// Build one direct call with ordered actuals and no enclosing operation prelude.
    /// The native evaluator checks byte prefixes before continuing to later actuals.
    pub(super) fn ordinary_call(
        &mut self,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<ir::Expr, Diagnostic> {
        // A fallible result must be handled or forwarded before a scalar can be read.
        if self.definitions.results.contains_key(name) {
            return Err(Diagnostic::new(
                span,
                "handle the fallible call with `on` or return its exact Result",
            ));
        }
        Ok(ir::Expr::Call {
            function: name.to_owned(),
            args: self.ordinary_arguments(name, args, span)?,
        })
    }

    /// Lower once-only actual evaluation for either ordinary result category.
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
            // scalar helper must not execute before an earlier prefix check.
            for (actual, param) in args.iter().zip(params) {
                let argument = match param {
                    ir::FunctionParameterKind::U64 => {
                        ir::FunctionArgument::Scalar(self.scalar(actual, &mut output)?)
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
                    ir::FunctionParameterKind::Bytes => {
                        // A received extent can only be lent as an immediate
                        // readonly argument. No copied or shortened value escapes.
                        if let Some(name) = self.received_argument(actual)? {
                            actuals.push(ir::FunctionArgument::Received(name));
                            continue;
                        }
                        let (source, prefix) = if let Some(("prefix", operands)) = core_call(actual)
                        {
                            arity(operands, 2, actual.span)?;
                            // The IR observes this source before evaluating the requested extent.
                            let source = self.bytes_source(&operands[0])?;
                            let length = self.scalar(&operands[1], &mut output)?;
                            (source, Some(length))
                        } else {
                            (self.bytes_source(actual)?, None)
                        };
                        ir::FunctionArgument::Bytes { source, prefix }
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

    /// Resolve the source of a readonly byte argument without observing payload bytes.
    /// A forwarded formal retains its captured extent and caller root in the verifier.
    fn bytes_source(&self, expr: &ast::Expr) -> Result<ir::BytesSource, Diagnostic> {
        // A bare byte formal names a descriptor rather than a record projection.
        if let ast::ExprKind::Name(name) = &expr.kind
            && let Kind::InputBytes { param } = self.lookup(name, expr.span)?.kind
        {
            return Ok(ir::BytesSource::Param(param));
        }
        let (reference, path, kind) = self.projection(expr)?;
        // Only complete initialized fields may form new byte sources.
        if kind != ir::FieldKind::Bytes {
            return Err(Diagnostic::new(
                expr.span,
                "expected a live Bytes field or a readonly byte parameter",
            ));
        }
        Ok(ir::BytesSource::Field { reference, path })
    }
}
