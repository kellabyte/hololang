//! Definition checking and bounded lowering for experimental source functions.
//!
//! Scalar helpers become pure expression templates, and constructors retain
//! final-destination calls. Ordinary borrowed functions keep named signatures
//! for statement lowering. All headers are validated, including unused helpers.
//! Compiler-only expansion has explicit node and depth limits.

#[path = "functions/errors.rs"]
mod errors;
pub(super) use errors::case_path;
#[path = "functions/producer.rs"]
mod producer;
#[path = "functions/scalar.rs"]
mod scalar;
#[path = "functions/schema.rs"]
mod schema;
#[cfg(test)]
#[path = "functions/tests.rs"]
mod tests;

use super::ast;
use crate::ir;
use crate::source::{Diagnostic, Span};
use std::collections::{HashMap, HashSet};

/// Maximum source-helper expansion depth, an implementation limit rather than a language rule.
const MAX_DEPTH: usize = 64;
/// Shared budget for generated definition nodes, including intermediate scalar templates.
const MAX_NODES: usize = 100_000;

/// The optional return type supported by the scalar calling convention.
pub(super) const SCALAR_OPTION: &str = "Option<u64>";
pub(super) use ir::OPTION_U64_FAMILY as OPTION_FAMILY;
/// The sole tag for a missing optional value.
pub(super) const OPTION_NONE: &str = "None";

/// Checked source definitions awaiting the existing IR verifier and storage planner.
pub(super) struct Definitions {
    /// Ordered, resolved inline record schemas.
    pub records: Vec<ir::Record>,
    /// Closed, nonrecursive destination constructors, including synthetic literals.
    pub producers: Vec<ir::Producer>,
    /// Pure scalar helpers retained as parameterized expression templates.
    pub scalars: HashMap<String, ScalarTemplate>,
    /// Ordinary borrowed signatures available before checking any of their bodies.
    pub ordinary: HashMap<String, Vec<ir::FunctionParameterKind>>,
    /// Ordinary functions with a boolean return type.
    pub bools: HashSet<String>,
    /// Nominal result families and the private identity of optional returns.
    pub results: HashMap<String, String>,
    /// Resolved error graphs and their ordered, scalar-payload leaves.
    pub errors: HashMap<String, ir::ErrorFamily>,
}

/// One closed unsigned expression whose only inputs are indexed scalar parameters.
#[derive(Clone, Debug)]
pub(super) struct ScalarTemplate {
    /// Required explicit argument count.
    pub params: usize,
    /// Pure expression; `Param` indexes this helper's arguments.
    pub value: ir::Expr,
}

/// Mutable compiler state shared across schema, pure-expression, and constructor lowering.
struct Lowerer<'m> {
    /// Borrowed declarations for precise field/type diagnostics.
    records: HashMap<&'m str, &'m ast::RecordDecl>,
    /// Statically resolved source functions, including the separately handled entry point.
    functions: HashMap<&'m str, &'m ast::Function>,
    /// Functions lowered as checked bodies instead of scalar templates or producers.
    ordinary: HashSet<&'m str>,
    /// Declared error families for validating result headers.
    errors: &'m HashMap<String, ir::ErrorFamily>,
    /// Fully lowered scalar helpers; entries are inserted only after success.
    scalars: HashMap<String, ScalarTemplate>,
    /// Completed constructors, with synthetic literals in dependency order.
    producers: Vec<ir::Producer>,
    /// Named constructors whose bodies have already been validated.
    finished: HashSet<String>,
    /// Definitions currently being expanded, for explicit cycle rejection.
    active: HashSet<String>,
    /// Remaining compiler IR-node allocation budget.
    budget: usize,
    /// Next unspellable synthetic constructor identity.
    literal: usize,
}

/// Resolve every non-entry header and lower pure helpers and constructors.
/// Ordinary bodies and entry statements share the later lexical adapter.
pub(super) fn lower(module: &ast::Module) -> Result<Definitions, Diagnostic> {
    let mut budget = MAX_NODES;
    // Reserve source-schema output nodes before copying any owned record/field identities.
    for record in &module.records {
        budget = record
            .fields
            .len()
            .checked_add(1)
            .and_then(|nodes| budget.checked_sub(nodes))
            .ok_or_else(|| {
                Diagnostic::new(
                    record.span,
                    "schemas exceed the experimental 100000-node compilation budget",
                )
            })?;
    }
    let records = schema::lower(module)?;
    let mut errors = errors::lower(module, &mut budget)?;
    if module
        .functions
        .iter()
        .any(|function| function.result == SCALAR_OPTION)
    {
        errors.insert(
            OPTION_FAMILY.to_owned(),
            ir::ErrorFamily {
                name: OPTION_FAMILY.to_owned(),
                cases: vec![ir::ErrorCase {
                    name: OPTION_NONE.to_owned(),
                    payload: false,
                }],
                children: Vec::new(),
            },
        );
    }
    let mut lowerer = Lowerer {
        records: module
            .records
            .iter()
            .map(|record| (record.name.as_str(), record))
            .collect(),
        functions: HashMap::new(),
        ordinary: HashSet::new(),
        errors: &errors,
        scalars: HashMap::new(),
        producers: Vec::new(),
        finished: HashSet::new(),
        active: HashSet::new(),
        budget,
        literal: 0,
    };
    // Index all functions first so forward calls and duplicate unused definitions are checked.
    for function in &module.functions {
        if lowerer.functions.insert(&function.name, function).is_some() {
            return Err(Diagnostic::new(
                function.span,
                format!("duplicate function `{}`", function.name),
            ));
        }
        // Main's supported effects/header belong to entry lowering, rather than pure templates.
        if function.name != "main" {
            lowerer.header(function)?;
        }
    }
    lowerer.ordinary = ordinary_functions(module, &lowerer.functions);
    let mut ordinary = HashMap::new();
    let mut bools = HashSet::new();
    let mut results = HashMap::new();
    // Source order gives deterministic diagnostics. Ordinary calls keep their
    // signatures without expanding their bodies into the pure definition graph.
    for function in &module.functions {
        if function.name == "main" {
            continue;
        }
        if lowerer.ordinary.contains(function.name.as_str()) {
            if function.result == "bool" {
                bools.insert(function.name.clone());
            }
            // A fallible call keeps its exact nominal result through checking.
            if let Some(family) = ordinary_result(&function.result) {
                results.insert(function.name.clone(), family.to_owned());
            }
            let params = function.args.iter().map(|argument| {
                // The header establishes that every nonbuiltin type names a known schema.
                match argument.ty.as_str() {
                    "u64" => ir::FunctionParameterKind::U64,
                    "bool" => ir::FunctionParameterKind::Bool,
                    "Bytes" if argument.access == "readwrite" => {
                        ir::FunctionParameterKind::WritableBytes
                    }
                    "Bytes" => ir::FunctionParameterKind::Bytes,
                    ty => match batch_record(ty) {
                        Some(record) => ir::FunctionParameterKind::Batch {
                            record: record.to_owned(),
                            access: if argument.access == "readwrite" {
                                ir::Access::Readwrite
                            } else {
                                ir::Access::Readonly
                            },
                        },
                        None => ir::FunctionParameterKind::State(ty.to_owned()),
                    },
                }
            });
            ordinary.insert(function.name.clone(), params.collect());
        } else {
            lowerer.function(&function.name, 0)?;
        }
    }
    Ok(Definitions {
        records,
        producers: lowerer.producers,
        scalars: lowerer.scalars,
        ordinary,
        bools,
        results,
        errors,
    })
}

/// Borrow the nominal family from the parser's canonical scalar Result spelling.
/// Other return types retain their existing classification without allocating.
pub(super) fn result_family(result: &str) -> Option<&str> {
    result.strip_prefix("Result<u64,")?.strip_suffix('>')
}

/// Resolve the source return to its checked two-lane scalar representation.
fn ordinary_result(result: &str) -> Option<&str> {
    if result == SCALAR_OPTION {
        Some(OPTION_FAMILY)
    } else {
        result_family(result)
    }
}

/// Borrow the record component of a batch parameter's canonical type spelling.
pub(super) fn batch_record(ty: &str) -> Option<&str> {
    ty.strip_prefix("Batch<")?.strip_suffix('>')
}

/// Borrow the success and nominal family names from a canonical Result header.
/// The parser has already excluded malformed or nested type arguments.
pub(super) fn result_parts(result: &str) -> Option<(&str, &str)> {
    result
        .strip_prefix("Result<")?
        .strip_suffix('>')?
        .split_once(',')
}

/// Borrow the callee name and actuals from a statically named call.
pub(super) fn named_call(value: &ast::Expr) -> Option<(&str, &[ast::Expr])> {
    let ast::ExprKind::Call { callee, args } = &value.kind else {
        return None;
    };
    let ast::ExprKind::Name(name) = &callee.kind else {
        return None;
    };
    Some((name, args))
}

/// Map only the six comparisons, retaining unsigned scalar operands.
pub(super) fn comparison(op: ast::BinaryOp) -> Option<ir::Compare> {
    Some(match op {
        ast::BinaryOp::Eq => ir::Compare::Eq,
        ast::BinaryOp::Ne => ir::Compare::Ne,
        ast::BinaryOp::Lt => ir::Compare::Lt,
        ast::BinaryOp::Le => ir::Compare::Le,
        ast::BinaryOp::Gt => ir::Compare::Gt,
        ast::BinaryOp::Ge => ir::Compare::Ge,
        _ => return None,
    })
}

/// Identify boolean literals and operators before names and call results resolve.
pub(super) fn boolean_syntax(expr: &ast::Expr) -> bool {
    use ast::ExprKind;
    match expr.kind {
        ExprKind::Bool(_) => true,
        ExprKind::Binary { op, .. } if comparison(op).is_some() => true,
        _ => logical_operator(expr),
    }
}

fn logical_operator(expr: &ast::Expr) -> bool {
    use ast::{BinaryOp, ExprKind};
    matches!(
        expr.kind,
        ExprKind::Not { .. }
            | ExprKind::Binary {
                op: BinaryOp::LogicalAnd | BinaryOp::LogicalOr,
                ..
            }
    )
}

/// Ordinary bodies need no template. Boolean-dependent scalar bodies also
/// require ordinary lowering for their scalar-template callers.
enum OrdinaryFunction {
    Standard,
    ScalarBody,
}

/// Classify ordinary bodies once, then propagate newly supported scalar bodies to
/// scalar-template callers without repeatedly scanning declarations.
fn ordinary_functions<'m>(
    module: &'m ast::Module,
    functions: &HashMap<&str, &ast::Function>,
) -> HashSet<&'m str> {
    let mut ordinary = HashSet::new();
    let mut pending = Vec::new();
    for function in &module.functions {
        if function.name != "main"
            && let Some(kind) = ordinary_function(function, functions)
        {
            ordinary.insert(function.name.as_str());
            if matches!(kind, OrdinaryFunction::ScalarBody) {
                pending.push(function.name.as_str());
            }
        }
    }
    if pending.is_empty() {
        return ordinary;
    }

    // Index only single-return scalar candidates. Constructor calls and the
    // preexisting boundary around ordinary functions stay as before.
    let mut callers = HashMap::<&str, Vec<&str>>::new();
    for function in &module.functions {
        if function.name == "main"
            || function.result != "u64"
            || ordinary.contains(function.name.as_str())
        {
            continue;
        }
        let [statement] = function.body.as_slice() else {
            continue;
        };
        let ast::StatementKind::Return(value) = &statement.kind else {
            continue;
        };
        // Templates admit named/core calls and literal byte reads. Infix
        // operations and member projections are rejected.
        let mut expressions = vec![value];
        while let Some(expression) = expressions.pop() {
            match &expression.kind {
                ast::ExprKind::Call { callee, args } => {
                    if let ast::ExprKind::Name(name) = &callee.kind {
                        callers.entry(name).or_default().push(&function.name);
                    }
                    expressions.extend(args);
                }
                ast::ExprKind::Index { base, index }
                    if matches!(base.kind, ast::ExprKind::Bytes(_)) =>
                {
                    expressions.push(index);
                }
                _ => {}
            }
        }
    }
    // Visit the reverse graph once, including forward source references.
    while let Some(name) = pending.pop() {
        if let Some(callers) = callers.remove(name) {
            for caller in callers {
                if ordinary.insert(caller) {
                    pending.push(caller);
                }
            }
        }
    }
    ordinary
}

/// Select ordinary bodies for borrows, typed returns, and boolean or result dispatch.
fn ordinary_function(
    function: &ast::Function,
    functions: &HashMap<&str, &ast::Function>,
) -> Option<OrdinaryFunction> {
    if function.result == "bool" || ordinary_result(&function.result).is_some() {
        return Some(OrdinaryFunction::Standard);
    }
    if function.result != "u64" {
        return None;
    }
    if has_bool_parameter(function) {
        return Some(OrdinaryFunction::ScalarBody);
    }
    if function
        .args
        .iter()
        .any(|argument| argument.passing == "reference")
    {
        return Some(OrdinaryFunction::Standard);
    }
    let target = |value: &ast::Expr| {
        let (name, _) = named_call(value)?;
        functions.get(name).copied()
    };
    let boolean_condition = |value: &ast::Expr| {
        logical_operator(value) || target(value).is_some_and(|callee| callee.result == "bool")
    };
    let mut ordinary_scalar_body = false;
    let mut pending = vec![function.body.as_slice()];
    while let Some(body) = pending.pop() {
        for statement in body {
            use ast::StatementKind as Kind;
            let operand = match &statement.kind {
                Kind::Return(value) | Kind::Let { value, .. }
                    if matches!(
                        value.kind,
                        ast::ExprKind::Handle { .. } | ast::ExprKind::Try(_)
                    ) =>
                {
                    return Some(OrdinaryFunction::Standard);
                }
                Kind::Let { value, .. } => {
                    let callee = target(value);
                    if callee.is_some_and(|callee| ordinary_result(&callee.result).is_some()) {
                        return Some(OrdinaryFunction::Standard);
                    }
                    ordinary_scalar_body |= callee.is_some_and(|callee| callee.result == "bool")
                        || boolean_syntax(value)
                        || super::byte_ranges::split(value)
                            .is_ok_and(|(root, _)| matches!(root.kind, ast::ExprKind::Bytes(_)));
                    Some(value)
                }
                Kind::Expr(value) => {
                    ordinary_scalar_body |= target(value)
                        .is_some_and(|callee| callee.result == "bool")
                        || matches!(&value.kind, ast::ExprKind::Call { callee, args }
                            if core_name(callee) == Some("assert")
                                && matches!(args.as_slice(), [condition] if boolean_condition(condition)));
                    Some(value)
                }
                Kind::Return(value) | Kind::Assign { value, .. } => Some(value),
                Kind::Scope(body) => {
                    pending.push(body);
                    None
                }
                Kind::For { iterable, body, .. } => {
                    pending.push(body);
                    Some(iterable)
                }
                // Both forms use structured lowering, which diagnoses entry-only
                // construction when it occurs in an ordinary helper.
                Kind::Construct { .. } | Kind::HandleResult { .. } => {
                    return Some(OrdinaryFunction::Standard);
                }
                Kind::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    ordinary_scalar_body |= boolean_condition(condition);
                    pending.push(then_body);
                    pending.push(else_body);
                    Some(condition)
                }
                _ => None,
            };
            ordinary_scalar_body |= !ordinary_scalar_body
                && operand.is_some_and(|value| calls_bool_parameter(value, &target));
        }
    }
    ordinary_scalar_body.then_some(OrdinaryFunction::ScalarBody)
}

fn has_bool_parameter(function: &ast::Function) -> bool {
    function.args.iter().any(|argument| argument.ty == "bool")
}

/// Find calls that need ordinary lowering even inside a comparison or scalar actual.
fn calls_bool_parameter<'m>(
    operand: &ast::Expr,
    target: &impl Fn(&ast::Expr) -> Option<&'m ast::Function>,
) -> bool {
    use ast::ExprKind as Kind;
    let mut pending = vec![operand];
    while let Some(value) = pending.pop() {
        match &value.kind {
            Kind::Call { args, .. } => {
                if target(value).is_some_and(has_bool_parameter) {
                    return true;
                }
                pending.extend(args);
            }
            Kind::Binary { left, right, .. }
            | Kind::Index {
                base: left,
                index: right,
            } => {
                pending.extend([left.as_ref(), right.as_ref()]);
            }
            Kind::Not { value, .. } => pending.push(value),
            _ => {}
        }
    }
    false
}

impl Lowerer<'_> {
    /// Validate one non-entry header's explicit modes, result type, and contracts.
    /// Ordinary borrowed helpers may also receive byte views, record state, and batches.
    fn header(&self, function: &ast::Function) -> Result<(), Diagnostic> {
        let result = result_parts(&function.result);
        let success = if function.result == SCALAR_OPTION {
            "u64"
        } else {
            result.map_or(function.result.as_str(), |(success, _)| success)
        };
        let ordinary_return = success == "u64" || function.result == "bool";
        // Every typed signature must name a declaration, even if no caller uses it.
        if let Some((_, family)) = result
            && !self.errors.contains_key(family)
        {
            return Err(Diagnostic::new(
                function.span,
                format!("unknown error family `{family}`"),
            ));
        }
        let mut names = HashSet::new();
        // Duplicate names would make parameter indexes ambiguous, even in unused functions.
        for argument in &function.args {
            // The opt-in adapter resolves core.member as its explicit namespace,
            // so arguments must respect the same reservation as entry locals.
            if argument.name == "core" {
                return Err(Diagnostic::new(
                    argument.span,
                    "argument `core` shadows the reserved experimental core namespace",
                ));
            }
            if !names.insert(argument.name.as_str()) {
                return Err(Diagnostic::new(
                    argument.span,
                    format!("duplicate argument `{}`", argument.name),
                ));
            }
            if argument.ty == "bool" {
                if argument.access != "readonly" || argument.passing != "copy" {
                    return Err(Diagnostic::new(
                        argument.span,
                        "bool parameters require `as readonly copy`",
                    ));
                }
                if !ordinary_return {
                    return Err(Diagnostic::new(
                        argument.type_span,
                        "bool parameters are supported only by ordinary functions",
                    ));
                }
                continue;
            }
            let scalar =
                argument.ty == "u64" && argument.access == "readonly" && argument.passing == "copy";
            let bytes = argument.ty == "Bytes"
                && argument.passing == "reference"
                && (argument.access == "readonly"
                    || (argument.access == "readwrite" && ordinary_return));
            let state = ordinary_return
                && argument.access == "readwrite"
                && argument.passing == "reference"
                && self.records.contains_key(argument.ty.as_str());
            if let Some(record) = batch_record(&argument.ty) {
                if !ordinary_return || argument.passing != "reference" {
                    return Err(Diagnostic::new(
                        argument.span,
                        "`Batch<Record>` arguments require reference passing and an ordinary `u64`, `bool`, `Option<u64>`, or `Result<u64, E>` return",
                    ));
                }
                if !self.records.contains_key(record) {
                    return Err(Diagnostic::new(
                        argument.type_span,
                        format!("unknown batch record schema `{record}`"),
                    ));
                }
                continue;
            }
            // The IR verifier independently checks the state schema's fixed shape.
            if !scalar && !bytes && !state {
                return Err(Diagnostic::new(
                    argument.span,
                    "experimental helper arguments support `u64 as readonly copy` and `Bytes as readonly reference`; ordinary functions also accept `bool as readonly copy`, `Bytes as readwrite reference`, and writable fixed record state",
                ));
            }
        }
        let mut contracts = HashSet::new();
        // Each admitted call graph must prove these contracts. Unknown labels are rejected.
        for (name, span) in &function.guarantees {
            if !matches!(name.as_str(), "noalloc" | "noblock") {
                return Err(Diagnostic::new(
                    *span,
                    format!("unknown function contract `{name}`"),
                ));
            }
            if !contracts.insert(name.as_str()) {
                return Err(Diagnostic::new(
                    *span,
                    format!("duplicate function contract `{name}`"),
                ));
            }
            // Existing constructor lowering advances dynamic region storage, even for fixed records.
            if name == "noalloc" && !ordinary_return {
                return Err(Diagnostic::new(
                    *span,
                    "record constructors obtain fresh dynamic destination storage and cannot satisfy strict `noalloc` in this experimental adapter",
                ));
            }
        }
        // The constructor verifier independently checks fixed layouts and nested calls.
        if !ordinary_return && !self.records.contains_key(success) {
            return Err(Diagnostic::new(
                function.span,
                format!("unsupported or unknown return type `{}`", function.result),
            ));
        }
        Ok(())
    }

    /// Lower one statically named helper, resolving dependencies before caching it.
    /// Active definitions reject recursion; the depth cap bounds compiler recursion.
    fn function(&mut self, name: &str, depth: usize) -> Result<(), Diagnostic> {
        let function = self.functions.get(name).copied().ok_or_else(|| {
            Diagnostic::new(
                Span { start: 0, end: 0 },
                format!("unknown function `{name}`"),
            )
        })?;
        // Completed nodes may be shared by many callers without repeated expansion work here.
        if self.finished.contains(name) || self.scalars.contains_key(name) {
            return Ok(());
        }
        if name == "main" {
            return Err(Diagnostic::new(
                function.span,
                "calling the entry function from a helper is unsupported",
            ));
        }
        if depth >= MAX_DEPTH {
            return Err(Diagnostic::new(
                function.span,
                "helper expansion exceeds the experimental depth limit of 64",
            ));
        }
        if !self.active.insert(name.to_owned()) {
            return Err(Diagnostic::new(
                function.span,
                format!("recursive function graph at `{name}` is unsupported"),
            ));
        }
        // Result types select two existing checked representations, never a hidden aggregate copy.
        if function.result == "u64" {
            let template = scalar::function(self, function, depth)?;
            self.scalars.insert(name.to_owned(), template);
        } else {
            let producer = producer::function(self, function, depth)?;
            self.producers.push(producer);
            self.finished.insert(name.to_owned());
        }
        self.active.remove(name);
        Ok(())
    }

    /// Reserve `count` generated nodes before allocating them, or reject at `span`.
    /// This compiler bound is cumulative, including discarded intermediate argument trees.
    fn charge(&mut self, count: usize, span: Span) -> Result<(), Diagnostic> {
        self.budget = self.budget.checked_sub(count).ok_or_else(|| {
            Diagnostic::new(
                span,
                "helper expansion exceeds the experimental 100000-node compilation budget",
            )
        })?;
        Ok(())
    }
}

/// Substitute template parameters with already-evaluated/pure `args` expressions.
/// A complete iterative preflight reserves the output's nodes and bounds depth
/// before any cloning. Arguments are copied as trees without substituting their
/// own `Param` nodes; callers preserve evaluation-once behavior with local captures.
pub(super) fn instantiate(
    template: &ir::Expr,
    args: &[ir::Expr],
    span: Span,
    budget: &mut usize,
) -> Result<ir::Expr, Diagnostic> {
    let mut pending = vec![(template, 1usize, true)];
    let mut nodes = 0usize;
    // Visit actual expanded output, stopping before a repeated parameter can exhaust host memory.
    while let Some((expression, depth, substitute)) = pending.pop() {
        if depth > MAX_DEPTH {
            return Err(Diagnostic::new(
                span,
                "scalar expression expansion exceeds the experimental depth limit of 64",
            ));
        }
        if let ir::Expr::Param(index) = expression
            && substitute
        {
            let argument = args.get(*index).ok_or_else(|| {
                Diagnostic::new(span, "scalar template has an invalid parameter index")
            })?;
            pending.push((argument, depth, false));
            continue;
        }
        nodes += 1;
        if nodes > *budget {
            return Err(Diagnostic::new(
                span,
                "scalar expansion exceeds the experimental compilation node budget",
            ));
        }
        // These are the only IR expression forms with recursive expression children.
        match expression {
            ir::Expr::Binary { left, right, .. } => {
                pending.push((right, depth + 1, substitute));
                pending.push((left, depth + 1, substitute));
            }
            ir::Expr::Byte { index, .. }
            | ir::Expr::InputByte { index, .. }
            | ir::Expr::LiteralByte { index, .. } => {
                pending.push((index, depth + 1, substitute));
            }
            ir::Expr::Apply { args, body } => {
                // The body has a fresh scalar parameter frame; outer substitution
                // applies only to its actuals. Still account for every owned body node.
                pending.push((body, depth + 1, false));
                for argument in args.iter().rev() {
                    pending.push((argument, depth + 1, substitute));
                }
            }
            _ => {}
        }
    }
    *budget -= nodes;
    Ok(copy_template(template, args))
}

/// Clone a preflight-bounded template, replacing only its own parameter nodes.
/// Depth/node validation in `instantiate` bounds recursion and allocation here.
fn copy_template(template: &ir::Expr, args: &[ir::Expr]) -> ir::Expr {
    // Unchanged leaf variants use derived cloning; recursive variants substitute in source order.
    match template {
        ir::Expr::Param(index) => args[*index].clone(),
        ir::Expr::Binary { op, left, right } => {
            ir::Expr::binary(*op, copy_template(left, args), copy_template(right, args))
        }
        ir::Expr::Byte {
            reference,
            path,
            index,
        } => ir::Expr::Byte {
            reference: reference.clone(),
            path: path.clone(),
            index: Box::new(copy_template(index, args)),
        },
        ir::Expr::InputByte { param, index } => ir::Expr::InputByte {
            param: *param,
            index: Box::new(copy_template(index, args)),
        },
        ir::Expr::LiteralByte { bytes, index } => ir::Expr::LiteralByte {
            bytes: bytes.clone(),
            index: Box::new(copy_template(index, args)),
        },
        ir::Expr::Apply { args: inputs, body } => ir::Expr::Apply {
            args: inputs
                .iter()
                .map(|input| copy_template(input, args))
                .collect(),
            // Body Params name this Apply's captured actuals, not the outer template's.
            body: body.clone(),
        },
        _ => template.clone(),
    }
}

/// Recognize only the exact `core.member` namespace shape, without dynamic lookup.
fn core_name(expression: &ast::Expr) -> Option<&str> {
    // Nested projections and user values cannot masquerade as trusted core operations.
    match &expression.kind {
        ast::ExprKind::Member { base, field } if matches!(&base.kind, ast::ExprKind::Name(name) if name == "core") => {
            Some(field)
        }
        _ => None,
    }
}

/// Build a parameter-name table borrowing one header; duplicate names were rejected earlier.
fn parameters(function: &ast::Function) -> HashMap<&str, usize> {
    function
        .args
        .iter()
        .enumerate()
        .map(|(index, argument)| (argument.name.as_str(), index))
        .collect()
}

/// Return typed formal slots in source order after header validation.
/// The owned vector is compiler metadata only; byte parameters borrow rather than
/// allocate/copy target payloads. Unsupported kinds were rejected by `header`.
fn parameter_kinds(function: &ast::Function) -> Vec<ir::Parameter> {
    function
        .args
        .iter()
        .map(|argument| {
            // Header checking restricts the only alternatives to these exact two kinds.
            if argument.ty == "Bytes" {
                ir::Parameter::Bytes
            } else {
                ir::Parameter::U64
            }
        })
        .collect()
}
