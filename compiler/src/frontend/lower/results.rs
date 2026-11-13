//! Source lowering for scalar call results and fixed-record append outcomes.
//!
//! Families remain nominal through IR checking. One direct call feeds one arm,
//! and arm-local evaluation cannot be hoisted before dispatch. Bindings and
//! coverage tables allocate only in the compiler.

use std::collections::HashSet;

use super::{Block, Kind, Lowerer, arity, ast};
use crate::frontend::functions;
use crate::frontend::functions::{OPTION_FAMILY, OPTION_NONE};
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

/// A direct tagged call or a previously evaluated scalar outcome.
enum HandlerSubject {
    Call {
        function: String,
        args: Vec<ir::FunctionArgument>,
        family: String,
    },
    Local {
        name: String,
        family: String,
    },
}

impl HandlerSubject {
    /// Return the checked outcome identity that determines the legal arms.
    fn family(&self) -> &str {
        match self {
            Self::Call { family, .. } | Self::Local { family, .. } => family,
        }
    }
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
        let (batch, record) = self.owned_batch(&args[0])?;
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
                    ast::ResultPattern::Fallback(binding) => ir::AppendPattern::Fallback(
                        self.fallback_binding(binding.as_deref(), arm.span)?,
                    ),
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

    /// Bind a successful call or saved result, or propagate its same-family or
    /// declared child failure. Resolve the operand before introducing the local.
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
        if let ast::ExprKind::Name(source) = &expression.kind {
            let binding = self.lookup(source, expression.span)?;
            let Kind::SavedResult(actual) = binding.kind else {
                return Err(Diagnostic::new(
                    expression.span,
                    "`try` requires a saved `Result`",
                ));
            };
            self.require_try_family(&actual, family, expression.span)?;
            let name = self.bind(name, Kind::Scalar { mutable: false }, span)?;
            return Ok(ir::Op::TrySaved {
                name,
                source: binding.ir_name,
            });
        }
        let (function, args) = self.tagged_call(expression)?;
        self.require_try_family(&self.definitions.results[function], family, expression.span)?;
        let args = self.ordinary_arguments(function, args, expression.span)?;
        let name = self.bind(name, Kind::Scalar { mutable: false }, span)?;
        Ok(ir::Op::TryLet {
            name,
            function: function.to_owned(),
            args,
        })
    }

    /// Require a Result family that is exact or declared as a direct child.
    fn require_try_family(&self, actual: &str, family: &str, span: Span) -> Result<(), Diagnostic> {
        if family == OPTION_FAMILY || actual == OPTION_FAMILY {
            return Err(Diagnostic::new(
                span,
                "`try` requires a Result operand inside a Result function. Handle `Option<u64>` with `on Some` and `on None`",
            ));
        }
        // An explicit child edge provides exactly one nominal route on failure.
        if self.definitions.errors[family]
            .try_error_offset(actual)
            .is_none()
        {
            return Err(Diagnostic::new(
                span,
                format!(
                    "cannot forward Result from error family `{actual}` as `{family}` with `try`; declare it as a direct child"
                ),
            ));
        }
        Ok(())
    }

    /// Resolve a typed return against `family`. Scalar evaluation preludes
    /// stay in `output`, and a saved result needs only its lexical name.
    pub(super) fn result_return(
        &mut self,
        expression: &ast::Expr,
        family: &str,
        output: &mut Block,
    ) -> Result<ir::ResultValue, Diagnostic> {
        self.charge(expression.span)?;
        if let ast::ExprKind::Name(name) = &expression.kind
            && (family != OPTION_FAMILY || name != OPTION_NONE)
        {
            let binding = self.lookup(name, expression.span)?;
            let Kind::SavedResult(actual) = &binding.kind else {
                return Err(Diagnostic::new(
                    expression.span,
                    "a named result return requires a saved `Result` or `Option<u64>`",
                ));
            };
            self.require_forwarded_type(actual, family, "saved result", expression.span)?;
            return Ok(ir::ResultValue::Saved(binding.ir_name));
        }
        if family == OPTION_FAMILY {
            return self.option_return(expression, output);
        }
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
                "return `ok value`, `error Family.Case`, or a call or saved result with the exact Result type",
            )),
        }
    }

    /// Lower an optional return to the tagged scalar representation.
    fn option_return(
        &mut self,
        expression: &ast::Expr,
        output: &mut Block,
    ) -> Result<ir::ResultValue, Diagnostic> {
        match &expression.kind {
            ast::ExprKind::Name(name) if name == OPTION_NONE => Ok(ir::ResultValue::Error {
                family: OPTION_FAMILY.to_owned(),
                case: OPTION_NONE.to_owned(),
                payload: None,
            }),
            ast::ExprKind::Call { callee, args } if matches!(&callee.kind, ast::ExprKind::Name(name) if name == "Some") =>
            {
                arity(args, 1, expression.span)?;
                Ok(ir::ResultValue::Ok(self.scalar(&args[0], output)?))
            }
            ast::ExprKind::Call { .. } => {
                let (name, args) = self.forwarded_call(expression, OPTION_FAMILY)?;
                Ok(ir::ResultValue::Call {
                    function: name.to_owned(),
                    args: self.ordinary_arguments(name, args, expression.span)?,
                })
            }
            _ => Err(Diagnostic::new(
                expression.span,
                "return `Some(value)`, `None`, or a call or saved value of type `Option<u64>`",
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
        let (name, args) = self.tagged_call(expression)?;
        let actual = &self.definitions.results[name];
        self.require_forwarded_type(actual, family, "forwarded call", expression.span)?;
        Ok((name, args))
    }

    /// Require exact nominal identity for a return that preserves the original tag.
    fn require_forwarded_type(
        &self,
        actual: &str,
        family: &str,
        subject: &str,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if actual != family {
            if actual == OPTION_FAMILY || family == OPTION_FAMILY {
                return Err(Diagnostic::new(
                    span,
                    format!(
                        "the {subject} must have the exact return type because `Option<u64>` and `Result` are distinct"
                    ),
                ));
            }
            return Err(Diagnostic::new(
                span,
                format!("cannot forward Result from error family `{actual}` as `{family}`"),
            ));
        }
        Ok(())
    }

    /// Resolve a result subject and its disjoint arms. All scalar
    /// work inside each arm remains an expression until that arm executes.
    /// `boolean` selects the initial bool-literal decision form for a `let`.
    pub(super) fn handle_result(
        &mut self,
        subject: &ast::Expr,
        arms: &[ast::ResultArm],
        boolean: bool,
    ) -> Result<ir::Expr, Diagnostic> {
        let selected = self.handler_subject(subject)?;
        let arms = self.result_arms(
            subject.span,
            selected.family(),
            arms.iter().map(|arm| (&arm.pattern, arm.span, arm)),
            |lowerer, pattern, arm| {
                let saved = std::mem::replace(&mut lowerer.deferred_actual, true);
                let value = lowerer.arm_value(arm, boolean);
                lowerer.deferred_actual = saved;
                Ok(ir::ResultArm {
                    pattern,
                    value: value?,
                })
            },
        )?;
        Ok(match selected {
            HandlerSubject::Call { function, args, .. } => ir::Expr::Handle {
                function,
                args,
                arms,
            },
            HandlerSubject::Local { name, .. } => ir::Expr::HandleSavedResult { name, arms },
        })
    }

    /// Lower one tagged subject and its exhaustive statement blocks.
    /// Each block sees only the active payload and uses normal lexical control flow.
    pub(super) fn handle_result_blocks(
        &mut self,
        subject: &ast::Expr,
        arms: &[ast::ResultBlockArm],
        span: Span,
        output: &mut Block,
    ) -> Result<(), Diagnostic> {
        let selected = self.handler_subject(subject)?;
        let mut children = Vec::with_capacity(arms.len());
        let arms = self.result_arms(
            subject.span,
            selected.family(),
            arms.iter().map(|arm| (&arm.pattern, arm.span, arm)),
            |lowerer, pattern, arm| {
                let body = lowerer.block(&arm.body)?;
                children.push(body.spans);
                Ok(ir::ResultBlockArm {
                    pattern,
                    body: body.ops,
                })
            },
        )?;
        let operation = match selected {
            HandlerSubject::Call { function, args, .. } => ir::Op::HandleResult {
                function,
                args,
                arms,
            },
            HandlerSubject::Local { name, .. } => ir::Op::HandleSavedResult { name, arms },
        };
        output.push(operation, span, children);
        Ok(())
    }

    /// Bind the handler to a saved outcome or capture a call's actuals.
    fn handler_subject(&mut self, subject: &ast::Expr) -> Result<HandlerSubject, Diagnostic> {
        if let ast::ExprKind::Name(name) = &subject.kind {
            let binding = self.lookup(name, subject.span)?;
            if let Kind::SavedResult(family) = binding.kind {
                return Ok(HandlerSubject::Local {
                    name: binding.ir_name,
                    family,
                });
            }
            return Err(Diagnostic::new(
                subject.span,
                "a named `on` subject must be a saved `Result` or `Option<u64>`",
            ));
        }
        let (name, actuals) = self.tagged_call(subject)?;
        Ok(HandlerSubject::Call {
            function: name.to_owned(),
            family: self.definitions.results[name].clone(),
            args: self.ordinary_arguments(name, actuals, subject.span)?,
        })
    }

    /// Lower and check an ordinary handler's pattern partition and arm scopes.
    /// The body adapter handles the expression or statement form within each arm.
    fn result_arms<'a, A, B>(
        &mut self,
        subject: Span,
        family: &str,
        arms: impl ExactSizeIterator<Item = (&'a ast::ResultPattern, Span, A)>,
        mut lower: impl FnMut(&mut Self, ir::ResultPattern, A) -> Result<B, Diagnostic>,
    ) -> Result<Vec<B>, Diagnostic> {
        if family == OPTION_FAMILY {
            return self.option_arms(subject, arms, lower);
        }
        let mut success = false;
        let mut fallback = false;
        let mut cases = HashSet::new();
        let mut lowered = Vec::with_capacity(arms.len());
        for (pattern, span, arm) in arms {
            self.charge(span)?;
            if fallback {
                return Err(Diagnostic::new(
                    span,
                    "the `on error` fallback must be the last arm",
                ));
            }
            self.scopes.push(Default::default());
            let item = (|| {
                let pattern = match pattern {
                    ast::ResultPattern::Ok(binding) => {
                        if std::mem::replace(&mut success, true) {
                            return Err(Diagnostic::new(span, "duplicate `on ok` arm"));
                        }
                        let binding = binding
                            .as_deref()
                            .map(|name| {
                                self.result_binding(name, Kind::Scalar { mutable: false }, span)
                            })
                            .transpose()?
                            .flatten();
                        ir::ResultPattern::Ok(binding)
                    }
                    ast::ResultPattern::Error { case, binding } => {
                        let (family, case, has_payload) = self.error_case(case, family, false)?;
                        if !cases.insert(case.clone()) {
                            return Err(Diagnostic::new(
                                span,
                                format!("duplicate error case `{case}`"),
                            ));
                        }
                        let binding = self.payload_binding(
                            &family,
                            &case,
                            has_payload,
                            binding.as_deref(),
                            span,
                        )?;
                        ir::ResultPattern::Error {
                            family,
                            case,
                            binding,
                        }
                    }
                    ast::ResultPattern::Fallback(binding) => {
                        fallback = true;
                        ir::ResultPattern::Fallback(
                            self.fallback_binding(binding.as_deref(), span)?,
                        )
                    }
                    ast::ResultPattern::Rejected { .. } => {
                        return Err(Diagnostic::new(
                            span,
                            "`on Rejected Case` requires a typed append",
                        ));
                    }
                };
                lower(self, pattern, arm)
            })();
            self.scopes.pop();
            lowered.push(item?);
        }
        if !success {
            return Err(Diagnostic::new(
                subject,
                "result handling requires an `on ok` arm",
            ));
        }
        if !fallback && cases.len() != self.definitions.errors[family].cases.len() {
            let missing = self.definitions.errors[family]
                .cases
                .iter()
                .map(|case| &case.name)
                .filter(|case| !cases.contains(*case))
                .min()
                .expect("coverage checked");
            return Err(Diagnostic::new(
                subject,
                format!("missing error case `{family}.{missing}`"),
            ));
        }
        Ok(lowered)
    }

    /// Check exhaustive optional patterns and bind the payload in its own arm.
    fn option_arms<'a, A, B>(
        &mut self,
        subject: Span,
        arms: impl ExactSizeIterator<Item = (&'a ast::ResultPattern, Span, A)>,
        mut lower: impl FnMut(&mut Self, ir::ResultPattern, A) -> Result<B, Diagnostic>,
    ) -> Result<Vec<B>, Diagnostic> {
        let mut some = false;
        let mut none = false;
        let mut lowered = Vec::with_capacity(arms.len());
        for (pattern, span, arm) in arms {
            self.charge(span)?;
            let ast::ResultPattern::Error { case, binding } = pattern else {
                return Err(Diagnostic::new(
                    span,
                    "use `on Some(value)` and `on None` for `Option<u64>`",
                ));
            };
            let ast::ExprKind::Name(name) = &case.kind else {
                return Err(Diagnostic::new(
                    case.span,
                    "use the bare `Some` or `None` case for `Option<u64>`",
                ));
            };
            self.scopes.push(Default::default());
            let item = (|| {
                let pattern = match name.as_str() {
                    "Some" => {
                        if std::mem::replace(&mut some, true) {
                            return Err(Diagnostic::new(span, "duplicate `on Some` arm"));
                        }
                        let binding = binding.as_deref().ok_or_else(|| {
                            Diagnostic::new(span, "`on Some` requires a payload binding or `(_)`")
                        })?;
                        ir::ResultPattern::Ok(self.result_binding(
                            binding,
                            Kind::Scalar { mutable: false },
                            span,
                        )?)
                    }
                    OPTION_NONE => {
                        if std::mem::replace(&mut none, true) {
                            return Err(Diagnostic::new(span, "duplicate `on None` arm"));
                        }
                        if binding.is_some() {
                            return Err(Diagnostic::new(span, "`None` has no payload to bind"));
                        }
                        ir::ResultPattern::Error {
                            family: OPTION_FAMILY.to_owned(),
                            case: OPTION_NONE.to_owned(),
                            binding: None,
                        }
                    }
                    _ => {
                        return Err(Diagnostic::new(
                            span,
                            "use `on Some(value)` and `on None` for `Option<u64>`",
                        ));
                    }
                };
                lower(self, pattern, arm)
            })();
            self.scopes.pop();
            lowered.push(item?);
        }
        if !some || !none {
            let missing = if some { "None" } else { "Some" };
            return Err(Diagnostic::new(
                subject,
                format!("missing `on {missing}` arm for `Option<u64>`"),
            ));
        }
        Ok(lowered)
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
        span: Span,
    ) -> Result<Option<String>, Diagnostic> {
        binding
            .map(|name| self.result_binding(name, Kind::Error, span))
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

    /// Recognize a direct tagged call whose namespace target was already
    /// bound. The borrowed actuals are lowered later in their written order.
    fn tagged_call<'a>(
        &self,
        expression: &'a ast::Expr,
    ) -> Result<(&'a str, &'a [ast::Expr]), Diagnostic> {
        self.tagged_target(expression)
            .map(|(name, args, _)| (name, args))
            .ok_or_else(|| {
                Diagnostic::new(
                    expression.span,
                    "expected a direct call returning a typed Result or `Option<u64>`",
                )
            })
    }

    /// Read a statically resolved tagged call signature without lowering its arguments.
    pub(super) fn tagged_target<'a>(
        &self,
        expression: &'a ast::Expr,
    ) -> Option<(&'a str, &'a [ast::Expr], &str)> {
        if let ast::ExprKind::Call { callee, args } = &expression.kind
            && let ast::ExprKind::Name(name) = &callee.kind
            && let Some(family) = self.definitions.results.get(name)
        {
            return Some((name, args, family));
        }
        None
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
