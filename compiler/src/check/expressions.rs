//! Scalar-expression and field-projection validation for the structured IR.
//!
//! Constructor expressions may use scalar/readonly-byte parameters and byte indices
//! but cannot name arbitrary published memory. Entry and ordinary-call expressions
//! require live roots, and typed actual captures preserve caller permissions.
//! Iterative expression walking avoids recursive validator stack growth; it does
//! not evaluate values or replace the backend's native bounds/overflow checks.

use super::CheckError;
use super::definitions::Definitions;
use super::scopes::{Binding, Environment};
use crate::ir::{
    Access, Argument, BoolExpr, BytesSource, Compare, Condition, Expr, FieldKind, Function,
    FunctionArgument, FunctionParameterKind, OPTION_U64_FAMILY, Parameter, ResultArm,
    ResultPattern,
};
use std::collections::HashSet;

/// Available scalar inputs while checking one expression.
#[derive(Clone, Copy)]
pub(super) enum Context<'a, 'p> {
    /// A scalar helper body's private frame, with no enclosing byte/index authority.
    Scalar {
        /// Number of eager scalar actuals captured before entering this body.
        params: usize,
    },
    /// Construction expressions have typed readonly inputs and an optional index.
    Producer {
        /// Ordered zero-based constructor parameter kinds.
        params: &'a [Parameter],
        /// Whether the current byte's index is in scope.
        index: bool,
    },
    /// Entry and ordinary helpers see initialized lexical scalar and storage bindings.
    Execution(&'a Environment<'p>),
}

/// One work item in the ordered expression walk. Ordinary argument checks have
/// their own item so a nested byte window or call is validated before later arguments.
enum Pending<'a, 'p> {
    /// Scalar expression with its available parameter and root context.
    Expression(&'p Expr, Context<'a, 'p>),
    /// Boolean expression with its available parameter and root context.
    Boolean(&'p BoolExpr, Context<'a, 'p>),
    /// Typed ordinary actual, its required formal kind, and the caller frame.
    Argument(
        &'p FunctionArgument,
        &'p FunctionParameterKind,
        &'a Environment<'p>,
    ),
    /// Check an arm with its own immutable bindings after checking the subject's actuals.
    Arm(&'p ResultArm, &'p str, &'a Environment<'p>),
    /// Restore the enclosing bindings before the next arm or expression.
    LeaveArm(&'a Environment<'p>),
}

/// Validate typed actuals against `params` without evaluating/copying source data.
/// Scalar expressions follow `context`; byte inputs must be forwarded typed formals
/// or initialized fields and received prefixes through live execution roots.
/// Arity and kind errors reject before lowering. Synchronous borrows cannot escape
/// this instruction set, so the call-site root suffices without runtime tracking.
pub(super) fn arguments<'p>(
    args: &'p [Argument],
    params: &[Parameter],
    definitions: &Definitions<'p>,
    context: Context<'_, 'p>,
) -> Result<(), CheckError> {
    // A length mismatch cannot be hidden by zip discarding unmatched actuals/formals.
    if args.len() != params.len() {
        return Err(CheckError::new("producer has wrong argument count"));
    }
    // Written order determines the first diagnostic and later native capture order.
    for (argument, expected) in args.iter().zip(params) {
        match (argument, expected) {
            (Argument::Scalar(value), Parameter::U64) => {
                expression(value, definitions, context)?;
            }
            (Argument::Bytes(source), Parameter::Bytes) => {
                producer_byte_source(source, definitions, context)?;
            }
            (Argument::ByteView { source, ranges }, Parameter::Bytes) => {
                producer_byte_source(source, definitions, context)?;
                for range in ranges {
                    if let Some(offset) = &range.offset {
                        expression(offset, definitions, context)?;
                    }
                    expression(&range.length, definitions, context)?;
                }
            }
            (Argument::Received(name), Parameter::Bytes) => {
                // Only an entry append in the live data arm can name its exact
                // prefix. Nested producers must forward their own byte formal.
                let Context::Execution(environment) = context else {
                    return Err(CheckError::new(
                        "received byte argument requires a live refill data branch. Nested producers must forward a Bytes parameter",
                    ));
                };
                environment.received(name)?;
            }
            _ => return Err(CheckError::new("producer argument has incompatible type")),
        }
    }
    Ok(())
}

/// Authorize the original root before inspecting a producer view's bound expressions.
fn producer_byte_source<'p>(
    source: &'p BytesSource,
    definitions: &Definitions<'p>,
    context: Context<'_, 'p>,
) -> Result<(), CheckError> {
    match (source, context) {
        (BytesSource::Literal(_), Context::Producer { .. } | Context::Execution(_)) => Ok(()),
        (BytesSource::Param(index), Context::Producer { params, .. })
            if params.get(*index) == Some(&Parameter::Bytes) =>
        {
            Ok(())
        }
        (BytesSource::Field { reference, path }, Context::Execution(environment)) => {
            let reference = environment.reference(reference)?;
            require_field(definitions, reference.record, path, FieldKind::Bytes)
        }
        (BytesSource::Received(name), Context::Execution(environment)) => {
            environment.received(name)
        }
        (BytesSource::Local(name), Context::Execution(environment)) => {
            environment.byte_view(name, Access::Readonly).map(|_| ())
        }
        _ => Err(CheckError::new(
            "byte argument requires a live published field or forwarded Bytes parameter",
        )),
    }
}

/// Resolve `path` from `record` to its final logical field kind.
///
/// Empty/missing paths or traversal through a scalar/payload are rejected. Only
/// schema fields are accessible, so header lengths/offsets have no writable path.
pub(super) fn field<'p>(
    definitions: &Definitions<'p>,
    record: &str,
    path: &[String],
) -> Result<&'p FieldKind, CheckError> {
    let mut schema = definitions.record(record)?;
    // Each component consumes one finite path segment; only nested records continue.
    for (index, component) in path.iter().enumerate() {
        let current = schema
            .fields
            .iter()
            .find(|field| field.name == *component)
            .ok_or_else(|| {
                CheckError::new(format!("record {} has no field {component}", schema.name))
            })?;
        // The final component determines the operation's required scalar/payload kind.
        if index + 1 == path.len() {
            return Ok(&current.kind);
        }
        // Intermediate paths cannot reinterpret bytes or scalar values as records.
        match &current.kind {
            FieldKind::Record(nested) => schema = definitions.record(nested)?,
            _ => {
                return Err(CheckError::new(format!(
                    "field {component} is not a nested record"
                )));
            }
        }
    }
    Err(CheckError::new("a field projection must not be empty"))
}

/// Require a particular terminal kind for a reference projection.
///
/// Returns no value on success; errors identify an incompatible read/write path.
pub(super) fn require_field(
    definitions: &Definitions<'_>,
    record: &str,
    path: &[String],
    expected: FieldKind,
) -> Result<(), CheckError> {
    // Logical type agreement prevents stores from changing layout metadata or shape.
    if field(definitions, record, path)? != &expected {
        return Err(CheckError::new(format!(
            "field path {} has incompatible type",
            path.join(".")
        )));
    }
    Ok(())
}

/// Validate the boolean or both ordered unsigned operands of a condition.
pub(super) fn condition<'p>(
    value: &'p Condition,
    definitions: &Definitions<'p>,
    context: Context<'_, 'p>,
) -> Result<(), CheckError> {
    let mut pending = Vec::new();
    if matches!(value.op, Compare::Truth) {
        let (Expr::Bool(value), Expr::Const(0)) = (&value.left, &value.right) else {
            return Err(CheckError::new(
                "truth condition requires a boolean operand",
            ));
        };
        pending.push(Pending::Boolean(value, context));
    } else {
        push_comparison(&mut pending, value, context)?;
    }
    walk_context(pending, definitions, context)
}

/// Schedule a genuine scalar comparison without recursively testing truth values.
fn push_comparison<'a, 'p>(
    pending: &mut Vec<Pending<'a, 'p>>,
    value: &'p Condition,
    context: Context<'a, 'p>,
) -> Result<(), CheckError> {
    if matches!(value.op, Compare::Truth) {
        return Err(CheckError::new("expected an unsigned scalar comparison"));
    }
    pending.push(Pending::Expression(&value.right, context));
    pending.push(Pending::Expression(&value.left, context));
    Ok(())
}

/// Check an immutable boolean independently of unsigned scalar expressions.
pub(super) fn boolean<'p>(
    value: &'p BoolExpr,
    definitions: &Definitions<'p>,
    environment: &Environment<'p>,
) -> Result<(), CheckError> {
    let pending = vec![Pending::Boolean(value, Context::Execution(environment))];
    walk_in_environment(pending, definitions, environment)
}

/// Schedule boolean children, preserving ordinary call edges and operand order.
fn push_boolean<'a, 'p>(
    pending: &mut Vec<Pending<'a, 'p>>,
    value: &'p BoolExpr,
    definitions: &Definitions<'p>,
    context: Context<'a, 'p>,
) -> Result<(), CheckError> {
    match value {
        BoolExpr::Const(_) => {}
        BoolExpr::Local(name) => {
            let Context::Execution(environment) = context else {
                return Err(CheckError::new(
                    "boolean locals require an execution context",
                ));
            };
            if !matches!(environment.lookup(name)?, Binding::Boolean) {
                return Err(CheckError::new(format!("{name} is not a boolean local")));
            }
        }
        BoolExpr::Call { function, args } => {
            let Context::Execution(environment) = context else {
                return Err(CheckError::new(
                    "ordinary calls require an execution context",
                ));
            };
            let callee = definitions.function(function)?;
            if !callee.returns_bool {
                return Err(CheckError::new(format!("{function} does not return bool")));
            }
            push_actuals(pending, callee, args, environment)?;
        }
        BoolExpr::Compare(value) => push_comparison(pending, value, context)?,
        BoolExpr::And(left, right) | BoolExpr::Or(left, right) => {
            // Both operands are checked and contribute call edges.
            pending.push(Pending::Boolean(right, context));
            pending.push(Pending::Boolean(left, context));
        }
        BoolExpr::Not(value) => pending.push(Pending::Boolean(value, context)),
    }
    Ok(())
}

/// Validate an expression without evaluating it or mutating any program state.
///
/// Constructor observations are restricted to typed scalar/readonly-byte inputs; runtime
/// observations need a live binding of the correct kind. A temporary host work
/// vector is proportional to expression nesting, with no generated runtime cost.
pub(super) fn expression<'p>(
    value: &'p Expr,
    definitions: &Definitions<'p>,
    context: Context<'_, 'p>,
) -> Result<(), CheckError> {
    let pending = vec![Pending::Expression(value, context)];
    walk_context(pending, definitions, context)
}

/// Restore the active expression-local arm scope after any failed walk.
fn walk_context<'a, 'p>(
    pending: Vec<Pending<'a, 'p>>,
    definitions: &Definitions<'p>,
    context: Context<'a, 'p>,
) -> Result<(), CheckError> {
    match context {
        Context::Execution(environment) => walk_in_environment(pending, definitions, environment),
        _ => walk(pending, definitions),
    }
}

/// Nominal compatibility required by an ordinary result-forwarding operation.
pub(super) enum ResultForwarding {
    /// A bare return forwards a result of the exact declared type.
    Exact,
    /// Explicit `try` can also lift a declared direct child.
    Try,
}

/// Validate a result-forwarding call and its ordered actuals and record its
/// call edge. Nominal checks run in the compiler before any runtime lowering.
pub(super) fn forward_result<'p>(
    name: &str,
    args: &'p [FunctionArgument],
    family: &str,
    forwarding: ResultForwarding,
    definitions: &Definitions<'p>,
    environment: &Environment<'p>,
) -> Result<(), CheckError> {
    let callee = definitions.function(name)?;
    let source = callee.result.as_deref();
    match forwarding {
        ResultForwarding::Exact if source != Some(family) => {
            return Err(CheckError::new(format!(
                "forwarded result from {name} must have error family {family}"
            )));
        }
        ResultForwarding::Try => check_try_family(name, source, family, definitions)?,
        ResultForwarding::Exact => {}
    }
    check_actuals(callee, args, definitions, environment)
}

/// Require an exact family or one direct child for either propagation operand.
pub(super) fn check_try_family(
    name: &str,
    source: Option<&str>,
    family: &str,
    definitions: &Definitions<'_>,
) -> Result<(), CheckError> {
    if family == OPTION_U64_FAMILY || source == Some(OPTION_U64_FAMILY) {
        return Err(CheckError::new(
            "try requires a Result operand inside a Result function",
        ));
    }
    if source != Some(family) {
        let target = definitions.error_family(family)?;
        if source.is_none_or(|source| target.try_error_offset(source).is_none()) {
            return Err(CheckError::new(format!(
                "try result from {name} must have error family {family} or a declared child"
            )));
        }
    }
    Ok(())
}

/// Resolve the callee's result family and check its ordered actuals before binding.
pub(super) fn result_initializer<'p>(
    name: &str,
    args: &'p [FunctionArgument],
    definitions: &Definitions<'p>,
    environment: &Environment<'p>,
) -> Result<&'p str, CheckError> {
    let callee = definitions.function(name)?;
    let family = callee.result.as_deref().ok_or_else(|| {
        CheckError::new(format!(
            "result initializer {name} must return a typed result"
        ))
    })?;
    check_actuals(callee, args, definitions, environment)?;
    Ok(family)
}

/// Resolve a saved result's nominal family and check its complete partition.
pub(super) fn saved_result_subject<'p>(
    name: &str,
    patterns: impl ExactSizeIterator<Item = &'p ResultPattern>,
    definitions: &Definitions<'p>,
    environment: &Environment<'p>,
) -> Result<&'p str, CheckError> {
    let Binding::SavedResult(family) = environment.lookup(name)? else {
        return Err(CheckError::new(format!(
            "{name} is not a saved result local"
        )));
    };
    check_arms(patterns, family, definitions)?;
    Ok(family)
}

/// Validate the call and its partition before checking any statement arm body.
pub(super) fn result_block_subject<'p>(
    name: &str,
    args: &'p [FunctionArgument],
    patterns: impl ExactSizeIterator<Item = &'p ResultPattern>,
    definitions: &Definitions<'p>,
    environment: &Environment<'p>,
) -> Result<&'p str, CheckError> {
    let callee = definitions.function(name)?;
    let family = callee.result.as_deref().ok_or_else(|| {
        CheckError::new(format!("result handler subject {name} has no error family"))
    })?;
    check_arms(patterns, family, definitions)?;
    check_actuals(callee, args, definitions, environment)?;
    Ok(family)
}

/// Check ordinary actuals using the ordered walker and register the call edge.
fn check_actuals<'p>(
    callee: &'p Function,
    args: &'p [FunctionArgument],
    definitions: &Definitions<'p>,
    environment: &Environment<'p>,
) -> Result<(), CheckError> {
    let mut pending = Vec::new();
    push_actuals(&mut pending, callee, args, environment)?;
    walk_in_environment(pending, definitions, environment)
}

/// Schedule checked argument work and retain the ordinary acyclic graph edge.
fn push_actuals<'a, 'p>(
    pending: &mut Vec<Pending<'a, 'p>>,
    callee: &'p Function,
    args: &'p [FunctionArgument],
    environment: &'a Environment<'p>,
) -> Result<(), CheckError> {
    // Never let zip silently drop a missing or excess actual.
    if args.len() != callee.params.len() {
        return Err(CheckError::new(format!(
            "ordinary function {} has wrong argument count",
            callee.name
        )));
    }
    environment.note_call(&callee.name);
    // The LIFO work list diagnoses actuals in their written order.
    for (argument, parameter) in args.iter().zip(&callee.params).rev() {
        pending.push(Pending::Argument(argument, &parameter.kind, environment));
    }
    Ok(())
}

/// Check the disjoint tag partition of one immediate result handler.
fn check_arms<'a>(
    patterns: impl ExactSizeIterator<Item = &'a ResultPattern>,
    family: &str,
    definitions: &Definitions<'_>,
) -> Result<(), CheckError> {
    let declared = definitions.error_family(family)?;
    let mut success = false;
    let mut cases = HashSet::new();
    let mut fallback = false;
    // A final fallback can cover failures only; success must be explicit.
    let count = patterns.len();
    for (index, pattern) in patterns.enumerate() {
        match pattern {
            ResultPattern::Ok(_) => {
                if std::mem::replace(&mut success, true) {
                    return Err(CheckError::new("duplicate success arm"));
                }
            }
            ResultPattern::Error {
                family: actual,
                case,
                binding,
            } => {
                let declared = definitions.error_case(family, actual, case)?;
                // The inactive lane of a tag-only error never becomes a scalar binding.
                if binding.is_some() && !declared.payload {
                    return Err(CheckError::new(format!(
                        "error arm {family}.{case} has no payload to bind"
                    )));
                }
                if !cases.insert(case.as_str()) {
                    return Err(CheckError::new(format!(
                        "duplicate error arm {family}.{case}"
                    )));
                }
            }
            ResultPattern::Fallback(_) => {
                if family == OPTION_U64_FAMILY {
                    return Err(CheckError::new("Option<u64> requires Some and None arms"));
                }
                if index + 1 != count {
                    return Err(CheckError::new("error fallback must be the last arm"));
                }
                fallback = true;
            }
        }
    }
    if !success {
        return Err(CheckError::new("missing success arm"));
    }
    if !fallback
        && let Some(case) = declared
            .cases
            .iter()
            .find(|case| !cases.contains(case.name.as_str()))
    {
        return Err(CheckError::new(format!(
            "missing error arm {family}.{}",
            case.name
        )));
    }
    Ok(())
}

/// Walk an expression frame, discarding temporary arm bindings even on rejection.
fn walk_in_environment<'a, 'p>(
    pending: Vec<Pending<'a, 'p>>,
    definitions: &Definitions<'p>,
    environment: &'a Environment<'p>,
) -> Result<(), CheckError> {
    let depth = environment.arm_depth();
    let result = walk(pending, definitions);
    environment.restore_arms(depth);
    result
}

/// Walk ordered expressions and actuals without using the host call stack per node.
fn walk<'a, 'p>(
    mut pending: Vec<Pending<'a, 'p>>,
    definitions: &Definitions<'p>,
) -> Result<(), CheckError> {
    // Every child is visited once; reverse pushing preserves left-to-right diagnosis.
    while let Some(item) = pending.pop() {
        let (value, context) = match item {
            Pending::Expression(value, context) => (value, context),
            Pending::Boolean(value, context) => {
                push_boolean(&mut pending, value, definitions, context)?;
                continue;
            }
            Pending::Arm(arm, family, environment) => {
                let binding = match &arm.pattern {
                    ResultPattern::Ok(Some(name))
                    | ResultPattern::Error {
                        binding: Some(name),
                        ..
                    } => Some((name, Binding::Scalar { writable: false })),
                    ResultPattern::Fallback(Some(name)) => Some((name, Binding::Error { family })),
                    _ => None,
                };
                // The binder shadows enclosing names only for its own arm.
                if let Some((name, binding)) = binding {
                    if name.is_empty() {
                        return Err(CheckError::new("empty result arm binding"));
                    }
                    environment.push_arm(name, binding)?;
                    pending.push(Pending::LeaveArm(environment));
                }
                (&arm.value, Context::Execution(environment))
            }
            Pending::LeaveArm(environment) => {
                environment.pop_arm();
                continue;
            }
            Pending::Argument(argument, expected, environment) => {
                // A byte source's authority is checked before its range operands.
                // Scalar and window expressions continue through this same walker.
                match (argument, expected) {
                    (FunctionArgument::Scalar(value), FunctionParameterKind::U64) => {
                        pending.push(Pending::Expression(value, Context::Execution(environment)));
                    }
                    (FunctionArgument::Bool(value), FunctionParameterKind::Bool) => {
                        pending.push(Pending::Boolean(value, Context::Execution(environment)));
                    }
                    (FunctionArgument::State(name), FunctionParameterKind::State(record)) => {
                        let reference = environment.writable_reference(name)?;
                        if reference.record != record {
                            return Err(CheckError::new(format!(
                                "ordinary state argument {name} has incompatible schema"
                            )));
                        }
                    }
                    (
                        FunctionArgument::Batch(name),
                        FunctionParameterKind::Batch { record, access },
                    ) => {
                        let view = environment.batch_view(name, *access)?;
                        if view.record != record {
                            return Err(CheckError::new(format!(
                                "ordinary batch argument {name} has incompatible schema"
                            )));
                        }
                    }
                    (
                        FunctionArgument::Bytes { source, ranges },
                        FunctionParameterKind::Bytes | FunctionParameterKind::WritableBytes,
                    ) => {
                        let access = if *expected == FunctionParameterKind::WritableBytes {
                            Access::Readwrite
                        } else {
                            Access::Readonly
                        };
                        environment.byte_view_source(source, access, definitions)?;
                        let context = Context::Execution(environment);
                        for range in ranges.iter().rev() {
                            pending.push(Pending::Expression(&range.length, context));
                            if let Some(offset) = &range.offset {
                                pending.push(Pending::Expression(offset, context));
                            }
                        }
                    }
                    // Only a currently usable Data binding can become this readonly actual.
                    (FunctionArgument::Received(name), FunctionParameterKind::Bytes) => {
                        environment.received(name)?;
                    }
                    _ => {
                        return Err(CheckError::new(
                            "ordinary call argument has incompatible type",
                        ));
                    }
                }
                continue;
            }
        };
        match value {
            Expr::Const(_) => {}
            Expr::Bool(_) => {
                return Err(CheckError::new(
                    "a boolean cannot be used as an unsigned scalar",
                ));
            }
            Expr::Apply { args, body } => {
                // Body formals belong to the fresh scalar frame, while actuals
                // retain their enclosing roots/index scope and diagnose first.
                pending.push(Pending::Expression(
                    body,
                    Context::Scalar { params: args.len() },
                ));
                for argument in args.iter().rev() {
                    pending.push(Pending::Expression(argument, context));
                }
            }
            Expr::Call { function, args } => {
                // Destination constructors and pure scalar frames cannot acquire
                // published mutation effects through an ordinary call.
                let Context::Execution(environment) = context else {
                    return Err(CheckError::new(
                        "ordinary calls require an execution context",
                    ));
                };
                let callee = definitions.function(function)?;
                if callee.returns_bool {
                    return Err(CheckError::new(format!(
                        "boolean result of {function} cannot be used as an unsigned scalar"
                    )));
                }
                if callee.result.is_some() {
                    return Err(CheckError::new(format!(
                        "result of {function} must be handled or forwarded"
                    )));
                }
                push_actuals(&mut pending, callee, args, environment)?;
            }
            Expr::Handle {
                function,
                args,
                arms,
            } => {
                let Context::Execution(environment) = context else {
                    return Err(CheckError::new(
                        "result handlers require an execution context",
                    ));
                };
                let callee = definitions.function(function)?;
                let family = callee.result.as_deref().ok_or_else(|| {
                    CheckError::new(format!(
                        "result handler subject {function} has no error family"
                    ))
                })?;
                check_arms(arms.iter().map(|arm| &arm.pattern), family, definitions)?;
                // Actuals are scheduled last so they validate before any arm locals exist.
                for arm in arms.iter().rev() {
                    pending.push(Pending::Arm(arm, family, environment));
                }
                push_actuals(&mut pending, callee, args, environment)?;
            }
            Expr::HandleSavedResult { name, arms } => {
                let Context::Execution(environment) = context else {
                    return Err(CheckError::new(
                        "result handlers require an execution context",
                    ));
                };
                let family = saved_result_subject(
                    name,
                    arms.iter().map(|arm| &arm.pattern),
                    definitions,
                    environment,
                )?;
                for arm in arms.iter().rev() {
                    pending.push(Pending::Arm(arm, family, environment));
                }
            }
            Expr::Binary { left, right, .. } => {
                pending.push(Pending::Expression(right, context));
                pending.push(Pending::Expression(left, context));
            }
            Expr::LiteralByte { index, .. } => {
                pending.push(Pending::Expression(index, context));
            }
            Expr::Param(index) => match context {
                Context::Producer { params, .. } if params.get(*index) == Some(&Parameter::U64) => {
                }
                Context::Scalar { params } if *index < params => {}
                _ => {
                    return Err(CheckError::new(
                        "constructor parameter is unavailable or out of range",
                    ));
                }
            },
            Expr::InputLen(param) | Expr::InputByte { param, .. } => {
                // A byte input requires its own typed formal, never a numeric address.
                let available = match context {
                    Context::Producer { params, .. } => {
                        params.get(*param) == Some(&Parameter::Bytes)
                    }
                    Context::Execution(environment) => environment.byte_param(*param),
                    Context::Scalar { .. } => false,
                };
                if !available {
                    return Err(CheckError::new(
                        "readonly byte input is unavailable or has incompatible parameter type",
                    ));
                }
                // Index children remain scalar expressions and inherit the byte-index scope.
                if let Expr::InputByte { index, .. } = value {
                    pending.push(Pending::Expression(index, context));
                }
            }
            Expr::Index => match context {
                Context::Producer { index: true, .. } => {}
                _ => {
                    return Err(CheckError::new(
                        "byte initializer index is unavailable here",
                    ));
                }
            },
            // Locals and memory observations belong only to entry execution, not producers.
            _ => {
                let Context::Execution(environment) = context else {
                    return Err(CheckError::new(
                        "constructor expressions cannot observe locals, direct published references, or globals; use typed readonly inputs",
                    ));
                };
                match value {
                    // Timing is a harness observation, permitted only outside construction.
                    Expr::Nanoseconds => {
                        if environment.in_function() {
                            return Err(CheckError::new(
                                "ordinary functions cannot observe the harness clock",
                            ));
                        }
                    }
                    Expr::Local(name) => environment.require_scalar(name)?,
                    Expr::ViewLen(name) => {
                        if !matches!(environment.lookup(name)?, Binding::Received(_)) {
                            environment.byte_view(name, Access::Readonly)?;
                        }
                    }
                    Expr::ViewByte { name, index } => {
                        environment.byte_view(name, Access::Readonly)?;
                        pending.push(Pending::Expression(index, context));
                    }
                    Expr::Read { reference, path } => {
                        let reference = environment.reference(reference)?;
                        require_field(definitions, reference.record, path, FieldKind::U64)?;
                    }
                    Expr::Byte {
                        reference,
                        path,
                        index,
                    } => {
                        let reference = environment.reference(reference)?;
                        require_field(definitions, reference.record, path, FieldKind::Bytes)?;
                        pending.push(Pending::Expression(index, context));
                    }
                    Expr::BytesLen { reference, path } => {
                        let reference = environment.reference(reference)?;
                        require_field(definitions, reference.record, path, FieldKind::Bytes)?;
                    }
                    Expr::BatchLen(name) => {
                        environment.batch_view(name, crate::ir::Access::Readonly)?;
                    }
                    Expr::Used(name) => {
                        // A marker observation cannot reinterpret a batch/ref as backing storage.
                        if !matches!(environment.lookup(name)?, Binding::Region { .. }) {
                            return Err(CheckError::new(format!("{name} is not a region")));
                        }
                    }
                    Expr::Address(name) => {
                        if environment.in_function() {
                            return Err(CheckError::new(
                                "ordinary functions cannot export a reference address",
                            ));
                        }
                        environment.reference(name)?;
                    }
                    _ => {
                        unreachable!("pure expressions were handled before execution observations")
                    }
                }
            }
        }
    }
    Ok(())
}
