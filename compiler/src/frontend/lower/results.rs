//! Source lowering for scalar call results and fixed-record append outcomes.
//!
//! Families remain nominal through IR checking. One direct call feeds one arm,
//! and arm-local evaluation cannot be hoisted before dispatch. Bindings and
//! coverage tables allocate only in the compiler.

use std::collections::HashSet;

use super::{Block, Kind, Lowerer, arity, ast};
use crate::frontend::functions;
use crate::{
    ir,
    source::{Diagnostic, Span},
};

/// Captured input bindings for one statically named destination append.
pub(super) struct AppendCall {
    /// Live tail owner of the matched destination schema.
    pub batch: String,
    /// Name of the destination constructor that receives the slot.
    pub producer: String,
    /// Actuals in written order, carrying their original borrowed roots.
    pub args: Vec<ir::Argument>,
    /// Error family for a typed constructor, absent for the numeric adapter.
    pub family: Option<String>,
}

impl AppendCall {
    /// Complete the typed append operation after its arms are lowered and its
    /// destination result name is known. Moves captured actuals without cloning.
    pub(super) fn handled(self, result: String, arms: Vec<ir::AppendArm>) -> ir::Op {
        ir::Op::AppendHandled {
            batch: self.batch,
            producer: self.producer,
            args: self.args,
            result,
            arms,
        }
    }
}

impl Lowerer {
    /// Resolve the batch and constructor and schedule its actuals before dispatch.
    /// This common path preserves operand evaluation for both append interfaces.
    pub(super) fn append_call(
        &mut self,
        args: &[ast::Expr],
        span: Span,
        output: &mut Block,
    ) -> Result<AppendCall, Diagnostic> {
        arity(args, 2, span)?;
        let (batch, record) = self.batch(&args[0])?;
        // The constructor expression denotes a destination call, so it cannot
        // be evaluated independently into a temporary aggregate.
        let ast::ExprKind::Call {
            callee,
            args: inputs,
        } = &args[1].kind
        else {
            return Err(Diagnostic::new(
                args[1].span,
                "append requires a named destination-constructor call",
            ));
        };
        let ast::ExprKind::Name(producer) = &callee.kind else {
            return Err(Diagnostic::new(
                callee.span,
                "append requires a statically named destination constructor",
            ));
        };
        let definition = self
            .definitions
            .producers
            .iter()
            .find(|definition| definition.name == *producer)
            .ok_or_else(|| {
                Diagnostic::new(
                    callee.span,
                    format!("unknown destination constructor `{producer}`"),
                )
            })?;
        // A different schema would write fields at unrelated offsets.
        if definition.record != record {
            return Err(Diagnostic::new(
                args[1].span,
                format!(
                    "constructor result does not match the batch schema (expected `{record}`, found `{}`)",
                    definition.record
                ),
            ));
        }
        arity(inputs, definition.params.len(), args[1].span)?;
        let parameters = definition.params.clone();
        let family = definition.error.clone();
        let args = self.producer_arguments(inputs, &parameters, output)?;
        Ok(AppendCall {
            batch,
            producer: producer.clone(),
            args,
            family,
        })
    }

    /// Lower an immediate typed append decision. Arm expressions are scoped
    /// to the completed append and evaluated only on their selected outcome.
    pub(super) fn append_decision(
        &mut self,
        args: &[ast::Expr],
        subject: Span,
        arms: &[ast::ResultArm],
        boolean: bool,
        output: &mut Block,
    ) -> Result<(AppendCall, Vec<ir::AppendArm>), Diagnostic> {
        let call = self.append_call(args, subject, output)?;
        let family = call.family.as_deref().ok_or_else(|| {
            Diagnostic::new(
                subject,
                "append `on` arms require a typed destination constructor",
            )
        })?;
        let mut lowered = Vec::with_capacity(arms.len());
        // Each arm's payload binding is inaccessible to its siblings and caller.
        for arm in arms {
            self.charge(arm.span)?;
            self.scopes.push(Default::default());
            let saved = std::mem::replace(&mut self.deferred_actual, true);
            let item = (|| {
                let pattern = match &arm.pattern {
                    ast::ResultPattern::Ok(None) => ir::AppendPattern::Ok,
                    ast::ResultPattern::Ok(Some(_)) => {
                        return Err(Diagnostic::new(
                            arm.span,
                            "append success has no payload; use `on ok`",
                        ));
                    }
                    ast::ResultPattern::Error {
                        case,
                        binding: None,
                    } if matches!(&case.kind, ast::ExprKind::Name(name) if name == "Full") => {
                        ir::AppendPattern::Full
                    }
                    ast::ResultPattern::Error {
                        case,
                        binding: None,
                    } if matches!(&case.kind, ast::ExprKind::Name(name) if name == "SizeOverflow") => {
                        ir::AppendPattern::SizeOverflow
                    }
                    ast::ResultPattern::Error { .. } => {
                        return Err(Diagnostic::new(
                            arm.span,
                            "use `on Rejected Case` for a constructor error, or a bare `on Full` or `on SizeOverflow` for append storage errors",
                        ));
                    }
                    ast::ResultPattern::Rejected { case, binding } => {
                        let (family, case, has_payload) = self.error_case(case, family, false)?;
                        let binding = self.payload_binding(
                            &family,
                            &case,
                            has_payload,
                            binding.as_deref(),
                            arm.span,
                        )?;
                        ir::AppendPattern::Rejected {
                            family,
                            case,
                            binding,
                        }
                    }
                    ast::ResultPattern::Fallback(binding) => {
                        ir::AppendPattern::Fallback(self.fallback_binding(binding.as_deref(), arm)?)
                    }
                };
                Ok(ir::AppendArm {
                    pattern,
                    value: self.arm_value(arm, boolean)?,
                })
            })();
            self.deferred_actual = saved;
            self.scopes.pop();
            lowered.push(item?);
        }
        Ok((call, lowered))
    }

    /// Bind a successful direct call in the current scope, or propagate its
    /// same-family or declared child failure. Capture actuals before binding.
    /// Lowering allocates compiler IR only and rejects unsupported contexts.
    pub(super) fn try_binding(
        &mut self,
        name: &str,
        expression: &ast::Expr,
        span: Span,
    ) -> Result<ir::Op, Diagnostic> {
        self.charge(span)?;
        let family = self.result_family.as_deref().ok_or_else(|| {
            Diagnostic::new(span, "`try` requires a scalar typed-result function")
        })?;
        let (function, args) = self.fallible_call(expression)?;
        let actual = &self.definitions.results[function];
        // An explicit child edge provides exactly one nominal route on failure.
        if self.definitions.errors[family]
            .try_error_offset(actual)
            .is_none()
        {
            return Err(Diagnostic::new(
                expression.span,
                format!(
                    "cannot forward Result from error family `{actual}` as `{family}` with `try`; declare it as a direct child"
                ),
            ));
        }
        let args = self.ordinary_arguments(function, args, expression.span)?;
        let name = self.bind(name, Kind::Scalar { mutable: false }, span)?;
        Ok(ir::Op::TryLet {
            name,
            function: function.to_owned(),
            args,
        })
    }

    /// Resolve a typed return against `family`, evaluating only its active
    /// scalar or forwarding call. Any scalar evaluation prelude stays in `output`.
    pub(super) fn result_return(
        &mut self,
        expression: &ast::Expr,
        family: &str,
        output: &mut Block,
    ) -> Result<ir::ResultValue, Diagnostic> {
        self.charge(expression.span)?;
        match &expression.kind {
            // The first success value is a full-width u64, including zero.
            ast::ExprKind::Ok(value) => Ok(ir::ResultValue::Ok(self.scalar(value, output)?)),
            ast::ExprKind::Error { case, payload } => {
                let (family, case, has_payload) = self.error_case(case, family, true)?;
                // The payload is an ordinary u64 expression on this return edge.
                let payload = match (has_payload, payload.as_deref()) {
                    (true, Some(value)) => Some(self.scalar(value, output)?),
                    (false, None) => None,
                    (true, None) => {
                        return Err(Diagnostic::new(
                            expression.span,
                            format!("error case `{family}.{case}` requires a `u64` payload"),
                        ));
                    }
                    (false, Some(value)) => {
                        return Err(Diagnostic::new(
                            value.span,
                            format!("error case `{family}.{case}` has no payload"),
                        ));
                    }
                };
                Ok(ir::ResultValue::Error {
                    family,
                    case,
                    payload,
                })
            }
            // Forward the pair as a result, rather than silently translating its error.
            ast::ExprKind::Call { .. } => {
                let (name, args) = self.forwarded_call(expression, family)?;
                Ok(ir::ResultValue::Call {
                    function: name.to_owned(),
                    args: self.ordinary_arguments(name, args, expression.span)?,
                })
            }
            _ => Err(Diagnostic::new(
                expression.span,
                "return `ok value`, `error Family.Case`, or an exact Result call",
            )),
        }
    }

    /// Borrow the name and actual expressions of a direct call only when its
    /// error family is exactly the enclosing scalar result's family.
    fn forwarded_call<'a>(
        &self,
        expression: &'a ast::Expr,
        family: &str,
    ) -> Result<(&'a str, &'a [ast::Expr]), Diagnostic> {
        let (name, args) = self.fallible_call(expression)?;
        let actual = &self.definitions.results[name];
        // A whole-result return preserves the exact type and its original tag.
        if actual != family {
            return Err(Diagnostic::new(
                expression.span,
                format!("cannot forward Result from error family `{actual}` as `{family}`"),
            ));
        }
        Ok((name, args))
    }

    /// Resolve one immediate result subject and its disjoint arms. All scalar
    /// work inside each arm remains an expression until that arm executes.
    /// `boolean` selects the initial bool-literal decision form for a `let`.
    pub(super) fn handle_result(
        &mut self,
        subject: &ast::Expr,
        arms: &[ast::ResultArm],
        boolean: bool,
    ) -> Result<ir::Expr, Diagnostic> {
        let (name, actuals) = self.fallible_call(subject)?;
        let family = self.definitions.results[name].clone();
        let args = self.ordinary_arguments(name, actuals, subject.span)?;
        let mut success = false;
        let mut fallback = false;
        let mut cases = HashSet::new();
        let mut lowered = Vec::with_capacity(arms.len());
        // Separate lexical scopes keep inactive payload bindings inaccessible.
        for arm in arms {
            self.charge(arm.span)?;
            if fallback {
                return Err(Diagnostic::new(
                    arm.span,
                    "the `on error` fallback must be the last arm",
                ));
            }
            self.scopes.push(Default::default());
            let saved = std::mem::replace(&mut self.deferred_actual, true);
            let item = (|| {
                let pattern = match &arm.pattern {
                    ast::ResultPattern::Ok(binding) => {
                        if std::mem::replace(&mut success, true) {
                            return Err(Diagnostic::new(arm.span, "duplicate `on ok` arm"));
                        }
                        // Bare `ok` selects success without introducing a scalar,
                        // just like the explicit wildcard `ok(_)`.
                        let binding = binding
                            .as_deref()
                            .map(|name| {
                                self.result_binding(name, Kind::Scalar { mutable: false }, arm.span)
                            })
                            .transpose()?
                            .flatten();
                        ir::ResultPattern::Ok(binding)
                    }
                    ast::ResultPattern::Error { case, binding } => {
                        let (family, case, has_payload) = self.error_case(case, &family, false)?;
                        if !cases.insert(case.clone()) {
                            return Err(Diagnostic::new(
                                arm.span,
                                format!("duplicate error case `{case}`"),
                            ));
                        }
                        // Binding or `_` makes handling an active payload explicit.
                        let binding = self.payload_binding(
                            &family,
                            &case,
                            has_payload,
                            binding.as_deref(),
                            arm.span,
                        )?;
                        ir::ResultPattern::Error {
                            family,
                            case,
                            binding,
                        }
                    }
                    ast::ResultPattern::Fallback(binding) => {
                        fallback = true;
                        let binding = self.fallback_binding(binding.as_deref(), arm)?;
                        ir::ResultPattern::Fallback(binding)
                    }
                    ast::ResultPattern::Rejected { .. } => {
                        return Err(Diagnostic::new(
                            arm.span,
                            "`on Rejected Case` requires a typed append",
                        ));
                    }
                };
                Ok(ir::ResultArm {
                    pattern,
                    value: self.arm_value(arm, boolean)?,
                })
            })();
            self.deferred_actual = saved;
            self.scopes.pop();
            lowered.push(item?);
        }
        if !success {
            return Err(Diagnostic::new(
                subject.span,
                "result handling requires an `on ok` arm",
            ));
        }
        if !fallback && cases.len() != self.definitions.errors[&family].cases.len() {
            let missing = self.definitions.errors[&family]
                .cases
                .iter()
                .map(|case| &case.name)
                .filter(|case| !cases.contains(*case))
                .min()
                .expect("coverage checked");
            return Err(Diagnostic::new(
                subject.span,
                format!("missing error case `{family}.{missing}`"),
            ));
        }
        Ok(ir::Expr::Handle {
            function: name.to_owned(),
            args,
            arms: lowered,
        })
    }

    /// Bind a scalar payload when the selected case declares one. Explicit
    /// discard uses the same shape check without creating an IR binding.
    pub(super) fn payload_binding(
        &mut self,
        family: &str,
        case: &str,
        has_payload: bool,
        binding: Option<&str>,
        span: Span,
    ) -> Result<Option<String>, Diagnostic> {
        // Tag-only cases do not carry storage; payload cases must acknowledge it.
        match (has_payload, binding) {
            (true, Some(name)) => self.result_binding(name, Kind::Scalar { mutable: false }, span),
            (false, None) => Ok(None),
            (true, None) => Err(Diagnostic::new(
                span,
                format!("error case `{family}.{case}` requires a payload binding or `(_)`"),
            )),
            (false, Some(_)) => Err(Diagnostic::new(
                span,
                format!("error case `{family}.{case}` has no payload to bind"),
            )),
        }
    }

    /// Optionally bind an opaque fallback error in the current arm scope.
    /// No scalar interpretation of the original error is exposed.
    fn fallback_binding(
        &mut self,
        binding: Option<&str>,
        arm: &ast::ResultArm,
    ) -> Result<Option<String>, Diagnostic> {
        binding
            .map(|name| self.result_binding(name, Kind::Error, arm.span))
            .transpose()
            .map(Option::flatten)
    }

    /// Keep one result expression behind its selected arm. Boolean decisions
    /// accept literals only; scalar operands use deferred actual evaluation.
    fn arm_value(&mut self, arm: &ast::ResultArm, boolean: bool) -> Result<ir::Expr, Diagnostic> {
        // The initial bool representation is an immutable zero/one binding.
        if boolean {
            let ast::ExprKind::Bool(value) = arm.value.kind else {
                return Err(Diagnostic::new(
                    arm.value.span,
                    "all boolean result arms must return a boolean literal",
                ));
            };
            return Ok(ir::Expr::Const(u64::from(value)));
        }
        let mut output = Block::default();
        let value = self.scalar(&arm.value, &mut output)?;
        // Deferred actual mode guarantees no arm work escaped dispatch.
        debug_assert!(output.ops.is_empty());
        Ok(value)
    }

    /// Bind an active arm value or leave `_` anonymous. The generated name is
    /// unique across the entire checked body even when source names are reused.
    pub(super) fn result_binding(
        &mut self,
        name: &str,
        kind: Kind,
        span: Span,
    ) -> Result<Option<String>, Diagnostic> {
        if name == "_" {
            return Ok(None);
        }
        self.bind(name, kind, span).map(Some)
    }

    /// Recognize a direct fallible call whose namespace target was already
    /// bound. The borrowed actuals are lowered later in their written order.
    fn fallible_call<'a>(
        &self,
        expression: &'a ast::Expr,
    ) -> Result<(&'a str, &'a [ast::Expr]), Diagnostic> {
        if let ast::ExprKind::Call { callee, args } = &expression.kind
            && let ast::ExprKind::Name(name) = &callee.kind
            && self.definitions.results.contains_key(name)
        {
            return Ok((name, args));
        }
        Err(Diagnostic::new(
            expression.span,
            "expected a direct call returning a typed Result",
        ))
    }

    /// Resolve a case and its payload shape in its expected family. Body returns
    /// require qualification; an `on` arm may derive it from the result type.
    pub(super) fn error_case(
        &self,
        expression: &ast::Expr,
        expected: &str,
        qualified: bool,
    ) -> Result<(String, String, bool), Diagnostic> {
        let leaf = functions::case_path(&self.definitions.errors, expected, expression, qualified)?;
        Ok((expected.to_owned(), leaf.name.clone(), leaf.payload))
    }
}
