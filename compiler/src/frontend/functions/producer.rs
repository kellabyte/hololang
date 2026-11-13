//! Fresh record literals and final-destination calls from source functions.
//!
//! A constructor may reject inputs before returning one fresh literal. Fields
//! retain declaration/evaluation order. Nested literals receive synthetic producer
//! identities and typed outer inputs. An exact whole-result call forwards the
//! same destination without an aggregate temporary. Readonly byte inputs keep
//! their original borrowed extent through each call.

use super::{
    Lowerer, MAX_DEPTH, ast, case_path, core_name, parameter_kinds, parameters, result_parts,
    scalar,
};
use crate::frontend::byte_ranges;
use crate::ir;
use crate::source::{Diagnostic, Span};

/// Lower a fresh literal or an exact whole-result destination call.
/// Literal prefix rejections preserve suffix rollback through the checked IR.
pub(super) fn function(
    lowerer: &mut Lowerer<'_>,
    function: &ast::Function,
    depth: usize,
) -> Result<ir::Producer, Diagnostic> {
    let (record, family) = result_parts(&function.result)
        .map_or((function.result.as_str(), None), |(record, family)| {
            (record, Some(family))
        });
    let params = parameters(function);
    let kinds = parameter_kinds(function);
    let context = scalar::Context {
        params: &params,
        kinds: &kinds,
        index: false,
        calls: depth,
    };
    let Some((last, prefix)) = function.body.split_last() else {
        return Err(Diagnostic::new(
            function.span,
            "record constructors require a final `return` of a fresh record or exact constructor call",
        ));
    };
    let mut body = Vec::new();
    if let ast::StatementKind::Return(value) = &last.kind
        && let ast::ExprKind::Call { callee, args } = &value.kind
    {
        // Delegation begins before any reservation in this function. Prefix
        // guards still belong to the fresh-literal construction path.
        if !prefix.is_empty() {
            return Err(Diagnostic::new(
                prefix[0].span,
                "a whole-result constructor return must be the function's only statement",
            ));
        }
        lowerer.charge(1, value.span)?;
        body.push(forward(
            lowerer,
            &function.result,
            callee,
            args,
            &context,
            value.span,
        )?);
    } else {
        // The typed form carries a nominal error; the old status adapter retains
        // its explicit numeric failure operation and cannot manufacture one.
        for statement in prefix {
            if let Some(family) = family {
                body.push(rejection(lowerer, statement, &context, family)?);
                continue;
            }
            let ast::StatementKind::Expr(value) = &statement.kind else {
                return Err(Diagnostic::new(
                    statement.span,
                    "constructor prefixes support only `core.reject_if(condition)`",
                ));
            };
            let ast::ExprKind::Call { callee, args } = &value.kind else {
                return Err(Diagnostic::new(
                    value.span,
                    "constructor prefixes support only `core.reject_if(condition)`",
                ));
            };
            if core_name(callee) != Some("reject_if") {
                return Err(Diagnostic::new(
                    callee.span,
                    "constructor prefixes support only `core.reject_if(condition)`",
                ));
            }
            scalar::arity(args.len(), 1, value.span)?;
            let condition = scalar::condition(lowerer, &args[0], &context)?;
            lowerer.charge(1, statement.span)?;
            body.push(ir::Build::FailIf(condition));
        }
        let ast::StatementKind::Return(value) = &last.kind else {
            return Err(Diagnostic::new(
                last.span,
                "record constructors require a final `return` of a fresh record or exact constructor call",
            ));
        };
        let value = if family.is_some() {
            if let ast::ExprKind::Ok(value) = &value.kind
                && matches!(value.kind, ast::ExprKind::Record { indented: true, .. })
            {
                value
            } else {
                return Err(Diagnostic::new(
                    value.span,
                    "a typed constructor requires `return ok Record:` with a fresh destination",
                ));
            }
        } else {
            value
        };
        body.extend(literal(lowerer, record, value, &context, 0, family)?);
    }
    lowerer.charge(1, function.span)?;
    Ok(ir::Producer {
        name: function.name.clone(),
        record: record.to_owned(),
        error: family.map(str::to_owned),
        params: kinds,
        body,
    })
}

/// Lower a sole named return with the exact nominal constructor result type.
/// Both typed and numeric callers keep their existing result and storage ABI.
fn forward(
    lowerer: &mut Lowerer<'_>,
    expected: &str,
    callee: &ast::Expr,
    args: &[ast::Expr],
    context: &scalar::Context<'_>,
    span: Span,
) -> Result<ir::Build, Diagnostic> {
    let ast::ExprKind::Name(name) = &callee.kind else {
        return Err(Diagnostic::new(
            callee.span,
            "whole-result returns require a statically named destination constructor",
        ));
    };
    let function = lowerer
        .functions
        .get(name.as_str())
        .copied()
        .ok_or_else(|| Diagnostic::new(callee.span, format!("unknown constructor `{name}`")))?;
    // Checked names are already canonical in standalone and packaged source.
    // Exact forwarding does not lift a child family or change record identity.
    if function.result != expected {
        return Err(Diagnostic::new(
            callee.span,
            format!(
                "constructor `{name}` returns `{}`, expected exact result `{expected}`",
                function.result
            ),
        ));
    }
    let args = constructor_arguments(lowerer, function, args, context, span)?;
    Ok(ir::Build::Forward {
        producer: name.clone(),
        args,
    })
}

/// Check one typed guard before construction and preserve its nominal tag and
/// optional scalar payload. Runtime lowering evaluates the payload on failure.
fn rejection(
    lowerer: &mut Lowerer<'_>,
    statement: &ast::Statement,
    context: &scalar::Context<'_>,
    family: &str,
) -> Result<ir::Build, Diagnostic> {
    // This finite prefix has no continuation on the selected failure path.
    let (condition, case, payload, span) = if let ast::StatementKind::If {
        condition,
        then_body,
        else_body,
    } = &statement.kind
        && else_body.is_empty()
        && let [
            ast::Statement {
                kind: ast::StatementKind::Return(value),
                ..
            },
        ] = then_body.as_slice()
        && let ast::ExprKind::Error { case, payload } = &value.kind
    {
        (condition, case, payload, value.span)
    } else {
        return Err(Diagnostic::new(
            statement.span,
            "typed constructor prefixes require `if condition { return error Family.Case }`",
        ));
    };
    let leaf = case_path(lowerer.errors, family, case, true)?;
    let field = leaf.name.clone();
    let has_payload = leaf.payload;
    // The active payload shape comes from the error declaration, never a conversion.
    if has_payload != payload.is_some() {
        return Err(Diagnostic::new(
            span,
            format!(
                "error case `{family}.{field}` must carry {} u64 payloads",
                usize::from(has_payload)
            ),
        ));
    }
    lowerer.charge(1, statement.span)?;
    let condition = scalar::condition(lowerer, condition, context)?;
    let payload = payload
        .as_deref()
        .map(|value| scalar::expression(lowerer, value, context, 0))
        .transpose()?;
    Ok(ir::Build::Reject {
        condition,
        case: field,
        payload,
    })
}

/// Initialize exactly one matching fresh literal, retaining written field order.
/// Nested literals recurse under a fixed implementation depth bound; scalar
/// computations remain pure and constructors write only unpublished destinations.
fn literal(
    lowerer: &mut Lowerer<'_>,
    expected: &str,
    value: &ast::Expr,
    context: &scalar::Context<'_>,
    depth: usize,
    family: Option<&str>,
) -> Result<Vec<ir::Build>, Diagnostic> {
    if depth >= MAX_DEPTH {
        return Err(Diagnostic::new(
            value.span,
            "nested record literals exceed the experimental depth limit of 64",
        ));
    }
    let ast::ExprKind::Record { name, fields, .. } = &value.kind else {
        return Err(Diagnostic::new(
            value.span,
            "record returns require a fresh matching record literal or a single exact constructor call; returning an existing aggregate is unsupported",
        ));
    };
    if name != expected {
        return Err(Diagnostic::new(
            value.span,
            format!("expected fresh `{expected}` record, found `{name}`"),
        ));
    }
    let record = lowerer
        .records
        .get(expected)
        .copied()
        .ok_or_else(|| Diagnostic::new(value.span, format!("unknown record `{expected}`")))?;
    if fields.len() != record.fields.len() {
        return Err(Diagnostic::new(
            value.span,
            format!(
                "`{expected}` requires exactly {} fields in declaration order, found {}",
                record.fields.len(),
                fields.len()
            ),
        ));
    }
    let mut body = Vec::with_capacity(fields.len());
    // Zip is complete because field counts were checked, and each next field has one initializer.
    for (field, declaration) in fields.iter().zip(&record.fields) {
        if field.name != declaration.name {
            return Err(Diagnostic::new(
                field.span,
                format!(
                    "expected field `{}` in declaration order, found `{}`",
                    declaration.name, field.name
                ),
            ));
        }
        let operation = match declaration.ty.as_str() {
            "u64" => ir::Build::U64 {
                field: field.name.clone(),
                value: scalar::expression(lowerer, &field.value, context, 0)?,
            },
            // Typed and numeric producers share ordered byte reservations and
            // checked reads from their explicitly captured readonly inputs.
            "Bytes" => bytes(lowerer, field, context)?,
            nested => nested_record(lowerer, field, nested, context, depth, family)?,
        };
        lowerer.charge(1, field.span)?;
        body.push(operation);
    }
    Ok(body)
}

/// Lower the explicit payload initializer and its contextual byte-index expression.
/// Length is evaluated once by checked IR; each element is narrowed to its low eight bits.
fn bytes(
    lowerer: &mut Lowerer<'_>,
    field: &ast::FieldInit,
    context: &scalar::Context<'_>,
) -> Result<ir::Build, Diagnostic> {
    let ast::ExprKind::Call { callee, args } = &field.value.kind else {
        return Err(Diagnostic::new(
            field.value.span,
            "a Bytes field requires `core.bytes(length, element_expression)`",
        ));
    };
    if core_name(callee) != Some("bytes") {
        return Err(Diagnostic::new(
            callee.span,
            "a Bytes field requires `core.bytes(length, element_expression)`",
        ));
    }
    scalar::arity(args.len(), 2, field.value.span)?;
    let len = scalar::expression(lowerer, &args[0], context, 0)?;
    let indexed = scalar::Context {
        params: context.params,
        kinds: context.kinds,
        index: true,
        calls: context.calls,
    };
    let value = scalar::expression(lowerer, &args[1], &indexed, 0)?;
    Ok(ir::Build::Bytes {
        field: field.name.clone(),
        len,
        value,
    })
}

/// Lower a named nested producer call or introduce a synthetic fresh-literal producer.
/// Outer scalar/byte parameters retain their kinds; no nested aggregate is copied.
fn nested_record(
    lowerer: &mut Lowerer<'_>,
    field: &ast::FieldInit,
    expected: &str,
    context: &scalar::Context<'_>,
    depth: usize,
    family: Option<&str>,
) -> Result<ir::Build, Diagnostic> {
    // The typed form routes the child's declared failure through the parent's
    // existing destination. Legacy producers keep their original literal/call path.
    let value = if family.is_some() {
        let ast::ExprKind::Try(value) = &field.value.kind else {
            return Err(Diagnostic::new(
                field.value.span,
                "typed nested record fields require `try constructor(...)`",
            ));
        };
        value
    } else {
        &field.value
    };
    match &value.kind {
        ast::ExprKind::Record { .. } if family.is_none() => {
            let name = format!("$literal.{}", lowerer.literal);
            lowerer.literal += 1;
            let body = literal(lowerer, expected, value, context, depth + 1, None)?;
            lowerer.charge(1, field.span)?;
            lowerer.producers.push(ir::Producer {
                name: name.clone(),
                record: expected.to_owned(),
                error: None,
                params: context.kinds.to_vec(),
                body,
            });
            let mut args = Vec::with_capacity(context.params.len());
            // Forward typed slots in original order; a borrowed payload is never a scalar address.
            for (index, kind) in context.kinds.iter().enumerate() {
                lowerer.charge(1, field.span)?;
                args.push(match kind {
                    ir::Parameter::U64 => ir::Argument::Scalar(ir::Expr::Param(index)),
                    ir::Parameter::Bytes => ir::Argument::Bytes(ir::BytesSource::Param(index)),
                });
            }
            Ok(ir::Build::Record {
                field: field.name.clone(),
                producer: name,
                args,
            })
        }
        ast::ExprKind::Call { callee, args } => {
            let ast::ExprKind::Name(name) = &callee.kind else {
                return Err(Diagnostic::new(
                    callee.span,
                    "nested record fields require a statically named constructor or a fresh literal",
                ));
            };
            let function = lowerer
                .functions
                .get(name.as_str())
                .copied()
                .ok_or_else(|| {
                    Diagnostic::new(callee.span, format!("unknown constructor `{name}`"))
                })?;
            let (actual, child) = result_parts(&function.result)
                .map_or((function.result.as_str(), None), |(record, family)| {
                    (record, Some(family))
                });
            // Record shape stays exact. Typed failure can flow through a
            // declared child edge while numeric constructors keep their own ABI.
            let compatible = match (family, child) {
                (Some(parent), Some(child)) => {
                    lowerer.errors[parent].try_error_offset(child).is_some()
                }
                (None, None) => true,
                _ => false,
            };
            if actual != expected || !compatible {
                let expected = family.map_or_else(
                    || expected.to_owned(),
                    |family| format!("Result<{expected},{family}>"),
                );
                return Err(Diagnostic::new(
                    callee.span,
                    format!(
                        "constructor `{name}` returns `{}`, expected `{expected}`",
                        function.result
                    ),
                ));
            }
            let arguments =
                constructor_arguments(lowerer, function, args, context, field.value.span)?;
            Ok(ir::Build::Record {
                field: field.name.clone(),
                producer: name.clone(),
                args: arguments,
            })
        }
        _ => Err(Diagnostic::new(
            field.value.span,
            "nested record fields require a fresh literal or named constructor call; aggregate copying is unsupported",
        )),
    }
}

/// Check a destination call through the bounded graph, then resolve its actuals.
/// The returned arguments preserve written order and forward byte formals without
/// reading their payload or copying the original borrowed storage.
fn constructor_arguments(
    lowerer: &mut Lowerer<'_>,
    function: &ast::Function,
    args: &[ast::Expr],
    context: &scalar::Context<'_>,
    span: Span,
) -> Result<Vec<ir::Argument>, Diagnostic> {
    scalar::arity(args.len(), function.args.len(), span)?;
    lowerer.function(&function.name, context.calls + 1)?;
    let mut arguments = Vec::with_capacity(args.len());
    // Each actual uses the matching declared kind in original evaluation order.
    for (argument, parameter) in args.iter().zip(parameter_kinds(function)) {
        arguments.push(match parameter {
            ir::Parameter::U64 => {
                ir::Argument::Scalar(scalar::expression(lowerer, argument, context, 0)?)
            }
            ir::Parameter::Bytes => {
                let (root, windows) = byte_ranges::split(argument)?;
                lowerer.charge(1, argument.span)?;
                let source = if let ast::ExprKind::Bytes(bytes) = &root.kind {
                    ir::BytesSource::Literal(bytes.clone())
                } else {
                    ir::BytesSource::Param(scalar::byte_parameter(root, context)?)
                };
                if windows.is_empty() {
                    ir::Argument::Bytes(source)
                } else {
                    let mut ranges = Vec::with_capacity(windows.len());
                    for window in windows {
                        lowerer.charge(1, window.span)?;
                        let offset = window
                            .offset
                            .map(|value| scalar::expression(lowerer, value, context, 0))
                            .transpose()?;
                        let length = scalar::expression(lowerer, window.length, context, 0)?;
                        ranges.push(ir::ByteRange { offset, length });
                    }
                    ir::Argument::ByteView { source, ranges }
                }
            }
        });
    }
    Ok(arguments)
}
