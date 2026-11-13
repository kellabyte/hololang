//! Recognize nested borrowed byte windows without choosing a storage context.

use super::ast::{Expr, ExprKind};
use crate::source::{Diagnostic, Span};

/// Written operands of one prefix or subrange, before scalar lowering.
pub(super) struct Window<'a> {
    /// Whole view span, including the source and bound expressions.
    pub(super) span: Span,
    /// Offset relative to the captured source. Prefixes start at zero.
    pub(super) offset: Option<&'a Expr>,
    /// Requested initialized extent.
    pub(super) length: &'a Expr,
}

/// Test whether a typed byte actual starts with a checked view operation.
pub(super) fn is_window(expr: &Expr) -> bool {
    operation(expr).is_some()
}

/// Split nested views into their original source and innermost-first bounds.
pub(super) fn split(mut expr: &Expr) -> Result<(&Expr, Vec<Window<'_>>), Diagnostic> {
    let mut windows = Vec::new();
    while let Some((prefix, args)) = operation(expr) {
        let expected = if prefix { 2 } else { 3 };
        if args.len() != expected {
            return Err(Diagnostic::new(
                expr.span,
                format!("expected {expected} arguments, received {}", args.len()),
            ));
        }
        windows.push(Window {
            span: expr.span,
            offset: (!prefix).then_some(&args[1]),
            length: &args[expected - 1],
        });
        expr = &args[0];
    }
    windows.reverse();
    Ok((expr, windows))
}

/// Recognize only explicit `core` view calls. The flag marks a prefix.
fn operation(expr: &Expr) -> Option<(bool, &[Expr])> {
    let ExprKind::Call { callee, args } = &expr.kind else {
        return None;
    };
    let ExprKind::Member { base, field } = &callee.kind else {
        return None;
    };
    if !matches!(&base.kind, ExprKind::Name(name) if name == "core") {
        return None;
    }
    match field.as_str() {
        "prefix" => Some((true, args)),
        "subrange" => Some((false, args)),
        _ => None,
    }
}
