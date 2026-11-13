//! LLVM emission for immediate handling and forwarding of scalar typed results.
//!
//! A private register-eligible aggregate carries an active unsigned success or
//! error payload plus a checked tag. It creates no diagnostic buffer or native stack
//! slot by itself. The handler uses disjoint control-flow edges and an SSA merge,
//! and extracts a payload only where the checker established its availability.

use crate::ir::{
    ErrorFamily, Expr, Function, FunctionArgument, FunctionParameterKind, Op, ResultArm,
    ResultPattern, ResultValue,
};

use super::{
    context::{Binding, Lowering},
    input::Input,
};

/// Private return representation of a `Result<u64, E>` in this lowering.
/// A zero tag selects success. Tag-only errors initialize the inactive scalar to zero.
pub(super) const RESULT_TYPE: &str = "{ i64, i8 }";

/// Incoming edge of a statically selected handler arm, with its active scalar.
struct DirectReturn {
    /// Actual predecessor after lowering the selected scalar expression.
    block: String,
    /// Copied success or error payload, absent for tag-only returns.
    payload: Option<String>,
}

/// Recognize a small checked return chain eligible for direct handler branches.
///
/// The callee may test copied formals in a bounded list of early return guards
/// and finish with a scalar result. Pure capture locals may precede a guard or
/// return. The per-site limit bounds emitted code size; calls, local mutation,
/// and other forms retain the ordinary ABI and switch.
fn direct_shape(function: &Function) -> Option<(&[Op], &ResultValue)> {
    let (tail, steps) = function.body.split_last()?;
    let Op::ReturnResult(tail) = tail else {
        return None;
    };
    // Bound scalar inputs and top-level work before inspecting nested operands.
    if function.params.len() > 8
        || steps.len() > 24
        || function
            .params
            .iter()
            .any(|param| param.kind != FunctionParameterKind::U64)
    {
        return None;
    }
    let mut budget = 64;
    direct_result_cost(tail, &mut budget)?;
    // Each guard either finishes this call or falls through to the next test.
    for step in steps {
        match step {
            Op::Let { value, .. } => direct_scalar_cost(value, &mut budget)?,
            Op::If {
                condition,
                then_body,
                else_body,
            } if else_body.is_empty() => {
                direct_scalar_cost(&condition.left, &mut budget)?;
                direct_scalar_cost(&condition.right, &mut budget)?;
                let (locals, outcome) = direct_branch(then_body)?;
                // Branch locals may only capture scalar work before the return.
                for local in locals {
                    let Op::Let { value, .. } = local else {
                        return None;
                    };
                    direct_scalar_cost(value, &mut budget)?;
                }
                direct_result_cost(outcome, &mut budget)?;
            }
            _ => return None,
        }
    }
    Some((steps, tail))
}

/// Separate the prefix of a return branch from its final checked alternative.
///
/// The shape recognizer checks the prefix before the emitter uses it as locals.
fn direct_branch(body: &[Op]) -> Option<(&[Op], &ResultValue)> {
    let (tail, prefix) = body.split_last()?;
    // Branches that can continue need the ordinary function's control flow.
    let Op::ReturnResult(outcome) = tail else {
        return None;
    };
    Some((prefix, outcome))
}

/// Charge only the active scalar of a return with a statically known alternative.
///
/// A forwarding call needs a runtime discriminant and remains on the normal ABI.
fn direct_result_cost(outcome: &ResultValue, budget: &mut usize) -> Option<()> {
    match outcome {
        ResultValue::Ok(value)
        | ResultValue::Error {
            payload: Some(value),
            ..
        } => direct_scalar_cost(value, budget),
        ResultValue::Error { payload: None, .. } => Some(()),
        ResultValue::Call { .. } => None,
    }
}

/// Bound inlined scalar work and admit only copied operands and wrapping arithmetic.
///
/// The countdown also bounds recursion in the shape recognizer. Excluding calls
/// avoids recursive specialization and keeps the callee free of frame effects.
fn direct_scalar_cost(value: &Expr, budget: &mut usize) -> Option<()> {
    *budget = budget.checked_sub(1)?;
    match value {
        Expr::Const(_) | Expr::Local(_) | Expr::Param(_) => Some(()),
        Expr::Binary { left, right, .. } => {
            direct_scalar_cost(left, budget)?;
            direct_scalar_cost(right, budget)
        }
        _ => None,
    }
}

/// Return the scalar binder shared by the ordinary and direct dispatch paths.
///
/// A fallback name denotes the original opaque error, with no scalar consumer.
fn payload_binding(pattern: &ResultPattern) -> Option<&str> {
    match pattern {
        ResultPattern::Ok(binding) | ResultPattern::Error { binding, .. } => binding.as_deref(),
        ResultPattern::Fallback(_) => None,
    }
}

impl Lowering<'_> {
    /// Emit a checked `value` and terminate its ordinary function block.
    /// The fresh dead continuation lets the structured emitter visit siblings.
    pub(super) fn return_result(&mut self, value: &ResultValue) {
        let result = match value {
            ResultValue::Ok(value) => {
                let value = self.expression(value);
                self.f.value(format!(
                    "insertvalue {RESULT_TYPE} zeroinitializer, i64 {value}, 0"
                ))
            }
            ResultValue::Error {
                family,
                case,
                payload,
            } => {
                let tagged = format!("{{ i64 0, i8 {} }}", self.error_tag(family, case));
                // Active error payloads share the success lane. Tag-only returns
                // retain their exact original representation and emit no extra work.
                if let Some(payload) = payload {
                    let payload = self.expression(payload);
                    self.f.value(format!(
                        "insertvalue {RESULT_TYPE} {tagged}, i64 {payload}, 0"
                    ))
                } else {
                    tagged
                }
            }
            ResultValue::Call { function, args } => self.ordinary_call(function, args),
        };
        self.f.line(format!("ret {RESULT_TYPE} {result}"));
        let continuation = self.f.fresh_label();
        self.f.label(&continuation);
    }

    /// Emit one call to `function` with ordered `args`, then choose one of `arms`.
    /// The returned i64 is a phi result; a named success payload never needs a slot.
    pub(super) fn handle_result(
        &mut self,
        function: &str,
        args: &[FunctionArgument],
        arms: &[ResultArm],
    ) -> String {
        // A small return chain can route known outcomes directly. Other callees
        // preserve the ordinary ABI, including stateful or forwarded results.
        let (index, function) = self.ordinary_definition(function);
        if let Some((steps, tail)) = direct_shape(function) {
            return self.handle_direct(function, args, arms, steps, tail);
        }
        self.handle_tagged(index, function, args, arms)
    }

    /// Call through the ordinary result ABI, then dispatch on the returned tag.
    ///
    /// The selected arm's i64 is an SSA merge, with no extra scalar slot.
    fn handle_tagged(
        &mut self,
        index: usize,
        function: &Function,
        args: &[FunctionArgument],
        arms: &[ResultArm],
    ) -> String {
        let result = self.ordinary_call_resolved(index, function, args);
        let tag = self
            .f
            .value(format!("extractvalue {RESULT_TYPE} {result}, 1"));
        let join = self.f.fresh_label();
        let labels: Vec<_> = arms.iter().map(|_| self.f.fresh_label()).collect();
        // A fallback covers residual errors; without one the checked tags are complete.
        let fallback = arms
            .last()
            .is_some_and(|arm| matches!(arm.pattern, ResultPattern::Fallback(_)));
        let default = if fallback {
            labels.last().expect("fallback has a block").clone()
        } else {
            self.f.fresh_label()
        };
        self.f.line(format!("switch i8 {tag}, label %{default} ["));
        // Only explicitly selected alternatives appear as switch cases.
        for (arm, label) in arms.iter().zip(&labels) {
            let case = match &arm.pattern {
                ResultPattern::Ok(_) => 0,
                ResultPattern::Error { family, case, .. } => self.error_tag(family, case),
                ResultPattern::Fallback(_) => continue,
            };
            self.f.line(format!("  i8 {case}, label %{label}"));
        }
        self.f.line("]");
        // An unrecognized tag cannot enter a checked exhaustive handler.
        if !fallback {
            self.f.label(&default);
            self.f.line("unreachable");
        }
        let mut incoming = Vec::with_capacity(arms.len());
        // Each arm has independent bindings and only its selected payload is available.
        for (arm, label) in arms.iter().zip(&labels) {
            self.f.label(label);
            self.push_scope();
            // A discarded payload needs no extraction, including on a fallback edge.
            if let Some(name) = payload_binding(&arm.pattern) {
                let value = self
                    .f
                    .value(format!("extractvalue {RESULT_TYPE} {result}, 0"));
                self.bind(name, Binding::ScalarValue(value));
            }
            let value = self.expression(&arm.value);
            self.pop_scope();
            // A nested handler can finish in its own join rather than this arm's first block.
            incoming.push(format!("[ {value}, %{} ]", self.f.current_label()));
            self.f.line(format!("br label %{join}"));
        }
        self.f.label(&join);
        self.f.value(format!("phi i64 {}", incoming.join(", ")))
    }

    /// Emit a recognized guarded callee into its immediate handler.
    ///
    /// All actuals evaluate in the caller before entering the callee's local
    /// scope. Its return edges select arms directly and carry only active
    /// scalars. Restore caller bindings before evaluating any arm expression.
    fn handle_direct(
        &mut self,
        function: &Function,
        args: &[FunctionArgument],
        arms: &[ResultArm],
        steps: &[Op],
        tail: &ResultValue,
    ) -> String {
        let mut captured = Vec::with_capacity(args.len());
        // Capturing eagerly preserves side effects and traps of unused actuals.
        for argument in args {
            let FunctionArgument::Scalar(value) = argument else {
                unreachable!("checked scalar actual for a copied formal");
            };
            captured.push(Input::Scalar(self.expression(value)));
        }
        let labels: Vec<_> = arms.iter().map(|_| self.f.fresh_label()).collect();
        let mut returns: Vec<Vec<DirectReturn>> = (0..arms.len()).map(|_| Vec::new()).collect();
        self.push_scope();
        // Both checked local and indexed forms denote the same copied parameters.
        for (param, input) in function.params.iter().zip(&captured) {
            let Input::Scalar(value) = input else {
                unreachable!("the copied arguments supply only scalar operands");
            };
            self.bind(&param.name, Binding::ScalarValue(value.clone()));
        }
        let previous = std::mem::replace(&mut self.params, captured);
        // Follow the original comparison order and evaluate only a taken return.
        for step in steps {
            match step {
                Op::Let { name, value } => self.direct_local(name, value),
                Op::If {
                    condition,
                    then_body,
                    ..
                } => {
                    let (locals, outcome) = direct_branch(then_body).expect("direct guard exits");
                    let condition = self.condition(condition);
                    let selected = self.f.fresh_label();
                    let next = self.f.fresh_label();
                    self.f.line(format!(
                        "br i1 {condition}, label %{selected}, label %{next}"
                    ));
                    self.f.label(&selected);
                    self.push_scope();
                    // A branch's scalar captures belong only to its own exit.
                    for local in locals {
                        let Op::Let { name, value } = local else {
                            unreachable!("direct branch prefix contains only scalar locals");
                        };
                        self.direct_local(name, value);
                    }
                    self.direct_return(outcome, arms, &labels, &mut returns);
                    self.pop_scope();
                    self.f.label(&next);
                }
                _ => unreachable!("direct shape contains only scalar locals and guards"),
            }
        }
        self.direct_return(tail, arms, &labels, &mut returns);
        self.params = previous;
        self.pop_scope();

        let join = self.f.fresh_label();
        let mut incoming = Vec::with_capacity(arms.len());
        // Arm bodies remain lazy and execute in the original caller frame.
        for ((arm, label), returns) in arms.iter().zip(labels).zip(returns) {
            // Exhaustive source handling can include alternatives this body never returns.
            if returns.is_empty() {
                continue;
            }
            self.f.label(&label);
            self.push_scope();
            // Several return sites may converge on the same named payload arm.
            if let Some(name) = payload_binding(&arm.pattern) {
                let value = self.direct_payload(&returns);
                self.bind(name, Binding::ScalarValue(value));
            }
            let value = self.expression(&arm.value);
            self.pop_scope();
            // Nested handlers can move the current block before this final branch.
            incoming.push(format!("[ {value}, %{} ]", self.f.current_label()));
            self.f.line(format!("br label %{join}"));
        }
        self.f.label(&join);
        self.f.value(format!("phi i64 {}", incoming.join(", ")))
    }

    /// Evaluate one copied initializer and bind it in the current inlined scope.
    ///
    /// The recognized shape cannot assign, so the value needs no local slot.
    fn direct_local(&mut self, name: &str, value: &Expr) {
        let value = self.expression(value);
        self.bind(name, Binding::ScalarValue(value));
    }

    /// Branch from a direct return to its statically selected nominal arm.
    ///
    /// The active scalar is evaluated once, and compiler-only edge records keep
    /// it available for a payload merge after restoring caller bindings.
    fn direct_return(
        &mut self,
        outcome: &ResultValue,
        arms: &[ResultArm],
        labels: &[String],
        returns: &mut [Vec<DirectReturn>],
    ) {
        // Use the same ordered partition as the switch, with fallback last.
        let target = arms
            .iter()
            .position(|arm| match (&arm.pattern, outcome) {
                (ResultPattern::Ok(_), ResultValue::Ok(_))
                | (ResultPattern::Fallback(_), ResultValue::Error { .. }) => true,
                (
                    ResultPattern::Error { family, case, .. },
                    ResultValue::Error {
                        family: actual_family,
                        case: actual_case,
                        ..
                    },
                ) => family == actual_family && case == actual_case,
                _ => false,
            })
            .expect("checked handler covers every known direct outcome");
        let payload = match outcome {
            ResultValue::Ok(value)
            | ResultValue::Error {
                payload: Some(value),
                ..
            } => Some(self.expression(value)),
            ResultValue::Error { payload: None, .. } => None,
            ResultValue::Call { .. } => unreachable!("forwarded results use the tagged path"),
        };
        returns[target].push(DirectReturn {
            block: self.f.current_label().to_owned(),
            payload,
        });
        self.f.line(format!("br label %{}", labels[target]));
    }

    /// Bind an active direct-return scalar, merging only when several sites reach it.
    ///
    /// The checker guarantees a payload for each incoming edge of a binding arm.
    fn direct_payload(&mut self, returns: &[DirectReturn]) -> String {
        // A unique predecessor already dominates the arm.
        if let [edge] = returns {
            return edge.payload.clone().expect("checked payload edge");
        }
        let incoming: Vec<_> = returns
            .iter()
            .map(|edge| {
                let payload = edge.payload.as_deref().expect("checked payload edge");
                format!("[ {payload}, %{} ]", edge.block)
            })
            .collect();
        self.f.value(format!("phi i64 {}", incoming.join(", ")))
    }

    /// Evaluate the actuals and call once. Bind success or return its typed failure.
    /// The checker establishes the family relation and rules out active cleanup.
    /// Success uses an immutable SSA value, and a child lift replaces only the
    /// failure tag. Neither operation needs a separate return slot.
    pub(super) fn try_binding(&mut self, name: &str, function: &str, args: &[FunctionArgument]) {
        let (index, callee) = self.ordinary_definition(function);
        let source = callee
            .result
            .as_deref()
            .expect("checked try calls a result function");
        let target = self
            .result_family
            .expect("checked try returns a typed result");
        let offset = self.try_error_offset(source, target);
        let result = self.ordinary_call_resolved(index, callee, args);
        let tag = self
            .f
            .value(format!("extractvalue {RESULT_TYPE} {result}, 1"));
        let ok = self.f.value(format!("icmp eq i8 {tag}, 0"));
        let success = self.f.fresh_label();
        let failure = self.f.fresh_label();
        self.f
            .line(format!("br i1 {ok}, label %{success}, label %{failure}"));

        self.f.label(&failure);
        // Child tags form a contiguous range in the target. The checker bounds
        // their sum to 255. An identity or a zero-offset child keeps the pair.
        let adjusted = (offset != 0).then(|| {
            let lifted = self.f.value(format!("add i8 {tag}, {offset}"));
            self.f.value(format!(
                "insertvalue {RESULT_TYPE} {result}, i8 {lifted}, 1"
            ))
        });
        let failed = adjusted.as_deref().unwrap_or(&result);
        self.f.line(format!("ret {RESULT_TYPE} {failed}"));
        self.f.label(&success);
        let value = self
            .f
            .value(format!("extractvalue {RESULT_TYPE} {result}, 0"));
        self.bind(name, Binding::ScalarValue(value));
    }

    /// Look up a checked nominal family by name for private tag assignment.
    /// No runtime lookup or allocation is emitted. The declaration must exist.
    fn error_family(&self, family: &str) -> &ErrorFamily {
        self.program
            .body
            .iter()
            .find_map(|operation| match operation {
                Op::ErrorFamily(declaration) if declaration.name == family => Some(declaration),
                _ => None,
            })
            .expect("checked error family exists")
    }

    /// Resolve a checked nominal case to its private nonzero declaration ordinal.
    pub(super) fn error_tag(&self, family: &str, case: &str) -> usize {
        self.error_family(family)
            .cases
            .iter()
            .position(|declared| declared.name == case)
            .expect("checked error case exists")
            + 1
    }

    /// Return the ordinal shift when a checked `try` promotes `source` into `target`.
    /// Exact families need no lookup. A mismatched child here violates verification.
    pub(super) fn try_error_offset(&self, source: &str, target: &str) -> u8 {
        // Identity keeps the original representation and emits no remapping.
        if source == target {
            return 0;
        }
        self.error_family(target)
            .try_error_offset(source)
            .expect("checked try has a declared error child")
    }

    /// Encode a declared case beyond the numeric append's reserved statuses.
    ///
    /// The ordinal is at most 255 under the checked family limit. A wide
    /// internal status leaves all 255 cases distinct from capacity and overflow.
    pub(super) fn append_error_status(&self, family: &str, case: &str) -> u64 {
        3 + self.error_tag(family, case) as u64
    }
}
