//! Pure unsigned helper and condition lowering for source constructors.
//!
//! Only explicit core arithmetic and closed scalar helpers are admitted. No
//! unbound published-memory read, allocation, mutation, or unknown call enters a
//! scalar template. Pure expressions can read static byte literals; producers
//! can also read explicit readonly byte formals. The contextual `core.index()`
//! is available in payload initializers. All byte reads retain bounds checks.

use super::{
    Lowerer, MAX_DEPTH, ScalarTemplate, ast, core_name, instantiate, parameter_kinds, parameters,
};
use crate::ir;
use crate::source::{Diagnostic, Span};
use std::collections::HashMap;

/// Pure-expression environment borrowing immutable explicit argument names.
pub(super) struct Context<'a> {
    /// Header argument names mapped to the producer/template's indexed inputs.
    pub params: &'a HashMap<&'a str, usize>,
    /// Source-order formal types; indexes in `params` name these complete slots.
    pub kinds: &'a [ir::Parameter],
    /// Whether `core.index()` names an active byte-initializer iteration.
    pub index: bool,
    /// Current statically resolved function expansion depth.
    pub calls: usize,
}

/// Lower a scalar helper's single pure return into a reusable parameter template.
/// Inputs remain immutable values; unsupported body effects are rejected at their statements.
pub(super) fn function(
    lowerer: &mut Lowerer<'_>,
    function: &ast::Function,
    depth: usize,
) -> Result<ScalarTemplate, Diagnostic> {
    let [statement] = function.body.as_slice() else {
        return Err(Diagnostic::new(
            function.span,
            "experimental scalar helpers require exactly one pure `return` statement",
        ));
    };
    let ast::StatementKind::Return(value) = &statement.kind else {
        return Err(Diagnostic::new(
            statement.span,
            "experimental scalar helpers support only a pure `return` statement",
        ));
    };
    let params = parameters(function);
    let kinds = parameter_kinds(function);
    let context = Context {
        params: &params,
        kinds: &kinds,
        index: false,
        calls: depth,
    };
    Ok(ScalarTemplate {
        params: function.args.len(),
        value: expression(lowerer, value, &context, 0)?,
    })
}

/// Lower one pure scalar expression and charge every newly created node first.
/// `depth` bounds expression recursion independently of named helper recursion.
pub(super) fn expression(
    lowerer: &mut Lowerer<'_>,
    value: &ast::Expr,
    context: &Context<'_>,
    depth: usize,
) -> Result<ir::Expr, Diagnostic> {
    if depth >= MAX_DEPTH {
        return Err(Diagnostic::new(
            value.span,
            "pure expression exceeds the experimental depth limit of 64",
        ));
    }
    // Pure scalar expressions can observe readonly input without copying it.
    let lowered = match &value.kind {
        ast::ExprKind::Number(number) => {
            lowerer.charge(1, value.span)?;
            Ok(ir::Expr::Const(*number))
        }
        ast::ExprKind::Bytes(_) => Err(Diagnostic::new(
            value.span,
            "a byte string literal cannot be used as a u64",
        )),
        ast::ExprKind::Name(name) => {
            let index = context.params.get(name.as_str()).copied().ok_or_else(|| {
                Diagnostic::new(value.span, format!("unknown pure scalar value `{name}`"))
            })?;
            // A borrowed descriptor cannot be reinterpreted as a numeric scalar or pointer.
            if context.kinds[index] != ir::Parameter::U64 {
                return Err(Diagnostic::new(
                    value.span,
                    "a readonly Bytes input requires core.len(input) or input[index], not a scalar value",
                ));
            }
            lowerer.charge(1, value.span)?;
            Ok(ir::Expr::Param(index))
        }
        ast::ExprKind::Index { base, index } => {
            lowerer.charge(1, value.span)?;
            if let ast::ExprKind::Bytes(bytes) = &base.kind {
                let index = expression(lowerer, index, context, depth + 1)?;
                Ok(ir::Expr::LiteralByte {
                    bytes: bytes.clone(),
                    index: Box::new(index),
                })
            } else {
                let param = byte_parameter(base, context)?;
                let index = expression(lowerer, index, context, depth + 1)?;
                Ok(ir::Expr::InputByte {
                    param,
                    index: Box::new(index),
                })
            }
        }
        ast::ExprKind::Call { callee, args } => {
            call(lowerer, callee, args, value.span, context, depth)
        }
        ast::ExprKind::Binary { .. } | ast::ExprKind::Not { .. }
            if super::boolean_syntax(value) =>
        {
            Err(Diagnostic::new(
                value.span,
                "a bool expression cannot be used as a u64",
            ))
        }
        ast::ExprKind::Binary { .. } => Err(Diagnostic::new(
            value.span,
            "infix scalar arithmetic is unsupported while numeric policy remains open; use explicit `core.add_wrapping`, `core.sub_wrapping`, `core.mul_wrapping`, `core.xor`, or `core.and`",
        )),
        _ => Err(Diagnostic::new(
            value.span,
            "expected a pure u64 expression; memory projections, booleans, and aggregate values are unsupported here",
        )),
    }?;
    // A cached helper's depth combines with surrounding source syntax after substitution.
    if depth == 0 {
        validate_depth(&lowered, value.span)?;
    }
    Ok(lowered)
}

/// Validate the final combined tree, including inlined helpers nested beneath core calls.
/// Iterative inspection allocates a bounded work list and never copies IR nodes.
fn validate_depth(expression: &ir::Expr, span: Span) -> Result<(), Diagnostic> {
    let mut pending = vec![(expression, 1usize)];
    // Each node is inspected once; unlike grammar depth, this includes instantiated templates.
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            return Err(Diagnostic::new(
                span,
                "scalar expression expansion exceeds the experimental depth limit of 64",
            ));
        }
        match node {
            ir::Expr::Binary { left, right, .. } => {
                pending.push((right, depth + 1));
                pending.push((left, depth + 1));
            }
            ir::Expr::Byte { index, .. }
            | ir::Expr::InputByte { index, .. }
            | ir::Expr::LiteralByte { index, .. } => {
                pending.push((index, depth + 1));
            }
            ir::Expr::Apply { args, body } => {
                // Frame boundaries do not hide depth or compiler-owned children.
                pending.push((body, depth + 1));
                for argument in args.iter().rev() {
                    pending.push((argument, depth + 1));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Resolve a pure core/scalar call, checking arity and the complete callee graph.
/// Scalar-only templates retain pure substitution. Byte-bearing producer contexts
/// capture actuals with Apply so inlining cannot erase or duplicate trapping reads.
fn call(
    lowerer: &mut Lowerer<'_>,
    callee: &ast::Expr,
    args: &[ast::Expr],
    span: Span,
    context: &Context<'_>,
    depth: usize,
) -> Result<ir::Expr, Diagnostic> {
    // Only exact core members receive trusted builtin behavior; every other target needs a definition.
    if let Some(name) = core_name(callee) {
        // Length observes a captured readonly extent, never a payload scan or numeric address.
        if name == "len" {
            arity(args.len(), 1, span)?;
            lowerer.charge(1, span)?;
            return Ok(if let ast::ExprKind::Bytes(bytes) = &args[0].kind {
                ir::Expr::Const(bytes.len() as u64)
            } else {
                ir::Expr::InputLen(byte_parameter(&args[0], context)?)
            });
        }
        if name == "index" {
            arity(args.len(), 0, span)?;
            if !context.index {
                return Err(Diagnostic::new(
                    span,
                    "`core.index()` is available only inside a byte initializer's element expression",
                ));
            }
            lowerer.charge(1, span)?;
            return Ok(ir::Expr::Index);
        }
        let op = match name {
            "add_wrapping" => ir::Binary::AddWrapping,
            "sub_wrapping" => ir::Binary::SubWrapping,
            "mul_wrapping" => ir::Binary::MulWrapping,
            "xor" => ir::Binary::Xor,
            "and" => ir::Binary::And,
            _ => {
                return Err(Diagnostic::new(
                    callee.span,
                    format!("unknown or unsupported pure core call `core.{name}`"),
                ));
            }
        };
        arity(args.len(), 2, span)?;
        let left = expression(lowerer, &args[0], context, depth + 1)?;
        let right = expression(lowerer, &args[1], context, depth + 1)?;
        lowerer.charge(1, span)?;
        return Ok(ir::Expr::binary(op, left, right));
    }
    let ast::ExprKind::Name(name) = &callee.kind else {
        return Err(Diagnostic::new(
            callee.span,
            "dynamic or projected helper calls are unsupported",
        ));
    };
    let function = lowerer
        .functions
        .get(name.as_str())
        .copied()
        .ok_or_else(|| Diagnostic::new(callee.span, format!("unknown function `{name}`")))?;
    if function.result == "bool" {
        return Err(Diagnostic::new(
            callee.span,
            "a bool return cannot be used as a u64",
        ));
    }
    if function.result != "u64" {
        return Err(Diagnostic::new(
            callee.span,
            format!("record constructor `{name}` is not a scalar expression"),
        ));
    }
    // Mutable ordinary calls belong to the statement verifier, outside the pure
    // scalar and private destination-construction graphs.
    if lowerer.ordinary.contains(name.as_str()) {
        return Err(Diagnostic::new(
            callee.span,
            format!(
                "ordinary borrowed function `{name}` cannot be called from a pure helper or destination constructor"
            ),
        ));
    }
    arity(args.len(), function.args.len(), span)?;
    lowerer.function(name, context.calls + 1)?;
    let mut arguments = Vec::with_capacity(args.len());
    // Argument evaluation order is retained even though all accepted computations are pure.
    for argument in args {
        arguments.push(expression(lowerer, argument, context, depth + 1)?);
    }
    // A scoped eager frame preserves trapping actuals, including an input or
    // literal byte read, even when the callee ignores or repeats its parameter.
    if context.kinds.contains(&ir::Parameter::Bytes) || arguments.iter().any(has_literal_read) {
        lowerer.charge(1, span)?;
        let identities: Vec<_> = (0..arguments.len()).map(ir::Expr::Param).collect();
        let template = &lowerer.scalars[name].value;
        let body = instantiate(template, &identities, span, &mut lowerer.budget)?;
        Ok(ir::Expr::Apply {
            args: arguments,
            body: Box::new(body),
        })
    } else {
        let template = &lowerer.scalars[name].value;
        instantiate(template, &arguments, span, &mut lowerer.budget)
    }
}

/// Pure scalar arguments may contain trapping literal reads after inlining.
fn has_literal_read(value: &ir::Expr) -> bool {
    match value {
        ir::Expr::LiteralByte { .. } => true,
        ir::Expr::Binary { left, right, .. } => has_literal_read(left) || has_literal_read(right),
        ir::Expr::Apply { args, body } => {
            args.iter().any(has_literal_read) || has_literal_read(body)
        }
        _ => false,
    }
}

/// Resolve a bare readonly byte parameter without admitting globals or stored views.
/// `value` and `context` are borrowed compiler syntax/facts; the returned full-slot
/// index retains its source formal kind. Missing/nonbyte names reject at the input span.
pub(super) fn byte_parameter(
    value: &ast::Expr,
    context: &Context<'_>,
) -> Result<usize, Diagnostic> {
    // Only an existing byte formal can be forwarded or observed in a producer.
    if let ast::ExprKind::Name(name) = &value.kind
        && let Some(index) = context.params.get(name.as_str()).copied()
        && context.kinds[index] == ir::Parameter::Bytes
    {
        return Ok(index);
    }
    Err(Diagnostic::new(
        value.span,
        "expected a readonly Bytes input parameter; published globals and scalar descriptors are unsupported",
    ))
}

/// Lower a producer guard using pure operands and short-circuit evaluation.
pub(super) fn condition(
    lowerer: &mut Lowerer<'_>,
    value: &ast::Expr,
    context: &Context<'_>,
) -> Result<ir::Condition, Diagnostic> {
    let result = boolean(lowerer, value, context)?;
    Ok(match result {
        // Keep the existing simple guard shapes for destination planning.
        ir::BoolExpr::Const(flag) => {
            lowerer.charge(2, value.span)?;
            ir::Condition {
                op: ir::Compare::Eq,
                left: ir::Expr::Const(u64::from(flag)),
                right: ir::Expr::Const(1),
            }
        }
        ir::BoolExpr::Compare(condition) => condition,
        value => ir::Condition::truth(value),
    })
}

/// Check each guard operand using the producer's pure expression context.
fn boolean(
    lowerer: &mut Lowerer<'_>,
    value: &ast::Expr,
    context: &Context<'_>,
) -> Result<ir::BoolExpr, Diagnostic> {
    lowerer.charge(1, value.span)?;
    match &value.kind {
        ast::ExprKind::Bool(flag) => Ok(ir::BoolExpr::Const(*flag)),
        ast::ExprKind::Not { value, .. } => Ok(ir::BoolExpr::Not(Box::new(boolean(
            lowerer, value, context,
        )?))),
        ast::ExprKind::Binary {
            op, left, right, ..
        } => {
            if matches!(op, ast::BinaryOp::LogicalAnd | ast::BinaryOp::LogicalOr) {
                let left = Box::new(boolean(lowerer, left, context)?);
                let right = Box::new(boolean(lowerer, right, context)?);
                return Ok(if *op == ast::BinaryOp::LogicalAnd {
                    ir::BoolExpr::And(left, right)
                } else {
                    ir::BoolExpr::Or(left, right)
                });
            }
            let relation = super::comparison(*op).ok_or_else(|| {
                Diagnostic::new(
                    value.span,
                    "a condition requires a comparison or boolean literal",
                )
            })?;
            let left = expression(lowerer, left, context, 0)?;
            let right = expression(lowerer, right, context, 0)?;
            Ok(ir::BoolExpr::Compare(ir::Condition {
                op: relation,
                left,
                right,
            }))
        }
        _ => Err(Diagnostic::new(
            value.span,
            "a condition requires a comparison or boolean literal",
        )),
    }
}

/// Enforce exact static argument count before indexing or lowering children.
pub(super) fn arity(actual: usize, expected: usize, span: Span) -> Result<(), Diagnostic> {
    // Mismatched arity cannot become an accidental partial call or unchecked argument access.
    if actual != expected {
        return Err(Diagnostic::new(
            span,
            format!("call expects {expected} arguments, found {actual}"),
        ));
    }
    Ok(())
}
