//! Resolves source error families into ordered leaves and direct nominal edges.
//!
//! Dependency traversal uses compiler-owned storage. Contiguous child blocks
//! let the backend translate failure tags with a constant offset. The checked
//! IR independently validates each block before the runtime can rely on it.

use super::{Diagnostic, HashMap, HashSet, Span, ast, ir};

/// Index declarations and expand them in dependency order. The shared budget
/// bounds generated leaf paths, including paths through deeply nested unions.
pub(super) fn lower(
    module: &ast::Module,
    budget: &mut usize,
) -> Result<HashMap<String, ir::ErrorFamily>, Diagnostic> {
    let records: HashSet<_> = module.records.iter().map(|record| &record.name).collect();
    let mut declarations = HashMap::new();
    // Index all nominal names before checking edges so children can be declared later.
    for declaration in &module.errors {
        charge(
            budget,
            declaration.cases.len().saturating_add(1),
            declaration.span,
        )?;
        if records.contains(&declaration.name)
            || matches!(declaration.name.as_str(), "u64" | "Bytes" | "Result")
            || declarations
                .insert(declaration.name.as_str(), declaration)
                .is_some()
        {
            return Err(Diagnostic::new(
                declaration.span,
                format!("duplicate or reserved error type `{}`", declaration.name),
            ));
        }
        validate_members(declaration)?;
    }

    let mut errors = HashMap::new();
    let mut active = HashSet::new();
    // Each frame resumes at its next member after resolving a child. The explicit
    // stack avoids compiler stack growth and gives a cycle the closing edge's span.
    for declaration in &module.errors {
        if errors.contains_key(&declaration.name) {
            continue;
        }
        let mut pending = vec![(declaration, declaration.cases.iter())];
        active.insert(declaration.name.as_str());
        while let Some((parent, members)) = pending.last_mut() {
            if let Some(member) = members.next() {
                let Some(source) = &member.source else {
                    continue;
                };
                let name = child_name(source)?;
                let child = declarations.get(name).copied().ok_or_else(|| {
                    Diagnostic::new(source.span, format!("unknown child error family `{name}`"))
                })?;
                if errors.contains_key(name) {
                    continue;
                }
                if !active.insert(name) {
                    return Err(Diagnostic::new(
                        source.span,
                        format!("cyclic error union through `{}.{name}`", parent.name),
                    ));
                }
                pending.push((child, child.cases.iter()));
            } else {
                let (parent, _) = pending.pop().expect("current error family");
                let family = expand(parent, &errors, budget)?;
                active.remove(parent.name.as_str());
                errors.insert(family.name.clone(), family);
            }
        }
    }
    Ok(errors)
}

/// Check the lexical member namespace and scalar payload shape before expansion.
/// Duplicate child identities would give a single `try` more than one route.
fn validate_members(declaration: &ast::ErrorDecl) -> Result<(), Diagnostic> {
    check_leaf_count(declaration.cases.len(), declaration.span)?;
    let mut names = HashSet::new();
    let mut children = HashSet::new();
    // Aliases and direct cases occupy the same namespace within the declaration.
    for member in &declaration.cases {
        if matches!(member.name.as_str(), "ok" | "error" | "_") || !names.insert(&member.name) {
            return Err(Diagnostic::new(
                member.span,
                format!(
                    "duplicate or reserved error case or alias `{}`",
                    member.name
                ),
            ));
        }
        if let Some(source) = &member.source {
            let name = child_name(source)?;
            if !children.insert(name) {
                return Err(Diagnostic::new(
                    source.span,
                    format!("child error family `{name}` is included more than once"),
                ));
            }
        }
        // One named u64 fits the existing scalar lane for every leaf.
        if let Some(payload) = &member.payload
            && (payload.ty != "u64" || payload.name == "_")
        {
            return Err(Diagnostic::new(
                payload.span,
                "error payload must be one named `u64` field",
            ));
        }
    }
    Ok(())
}

/// Read the local or package-canonical child identity. Qualified source paths
/// must already have passed package binding, including per-file visibility.
fn child_name(source: &ast::Expr) -> Result<&str, Diagnostic> {
    if let ast::ExprKind::Name(name) = &source.kind {
        return Ok(name);
    }
    Err(Diagnostic::new(
        source.span,
        "a qualified child error family requires package namespace resolution",
    ))
}

/// Materialize one complete family after its children. Prefixing each block
/// retains tag order and payload shape without introducing runtime metadata.
fn expand(
    declaration: &ast::ErrorDecl,
    resolved: &HashMap<String, ir::ErrorFamily>,
    budget: &mut usize,
) -> Result<ir::ErrorFamily, Diagnostic> {
    let mut cases = Vec::new();
    let mut children = Vec::new();
    // Visit members in written order, splicing each child's entire leaf range.
    for member in &declaration.cases {
        if let Some(source) = &member.source {
            let child = &resolved[child_name(source)?];
            check_leaf_count(cases.len() + child.cases.len(), member.span)?;
            children.push(ir::ErrorChild {
                alias: member.name.clone(),
                family: child.name.clone(),
            });
            // Count each copied path segment so a deep chain cannot build
            // quadratic output beyond the shared generated-node budget.
            for leaf in &child.cases {
                charge(budget, 2 + leaf.name.matches('.').count(), member.span)?;
                cases.push(ir::ErrorCase {
                    name: format!("{}.{}", member.name, leaf.name),
                    payload: leaf.payload,
                });
            }
        } else {
            check_leaf_count(cases.len() + 1, member.span)?;
            cases.push(ir::ErrorCase {
                name: member.name.clone(),
                payload: member.payload.is_some(),
            });
        }
    }
    Ok(ir::ErrorFamily {
        name: declaration.name.clone(),
        cases,
        children,
    })
}

/// Keep the closed set within the one-byte tag representation, reserving zero
/// for success. The count applies to leaves after inclusion.
fn check_leaf_count(count: usize, span: Span) -> Result<(), Diagnostic> {
    if !(1..=255).contains(&count) {
        return Err(Diagnostic::new(
            span,
            "an experimental error family requires 1 through 255 cases after expanding children",
        ));
    }
    Ok(())
}

/// Reserve generated compiler nodes before cloning declarations or paths.
fn charge(budget: &mut usize, nodes: usize, span: Span) -> Result<(), Diagnostic> {
    *budget = budget.checked_sub(nodes).ok_or_else(|| {
        Diagnostic::new(
            span,
            "error declarations exceed the compilation node budget",
        )
    })?;
    Ok(())
}

/// Resolve a case against its expected nominal family and return its checked
/// leaf. A constructor requires the family prefix. An arm also accepts a
/// relative path, with an explicit prefix of the expected family taking precedence.
pub(in crate::frontend) fn case_path<'a>(
    errors: &'a HashMap<String, ir::ErrorFamily>,
    expected: &str,
    expression: &ast::Expr,
    qualified: bool,
) -> Result<&'a ir::ErrorCase, Diagnostic> {
    let mut expression_part = expression;
    let mut tail = Vec::new();
    // Peel only member access. The bound root may be a canonical dotted name.
    while let ast::ExprKind::Member { base, field } = &expression_part.kind {
        tail.push(field.as_str());
        expression_part = base;
    }
    // A package-local spelling may name both a relative alias and a local
    // family. Keep both identities until the expected family is available.
    let (root, nominal) = match &expression_part.kind {
        ast::ExprKind::Name(root) => (root, root),
        ast::ExprKind::ErrorFamilyRoot { name, nominal } => (name, nominal),
        _ => {
            return Err(Diagnostic::new(
                expression.span,
                "expected a named error case",
            ));
        }
    };
    let explicit = nominal == expected && !tail.is_empty();
    // Bare cases and relative child paths are contextual to an `on` subject.
    if qualified && !explicit {
        if !tail.is_empty() && errors.contains_key(nominal) {
            return Err(Diagnostic::new(
                expression.span,
                format!("expected error family `{expected}`, found `{nominal}`"),
            ));
        }
        return Err(Diagnostic::new(
            expression.span,
            format!("expected an error case qualified by `{expected}`"),
        ));
    }
    if !explicit {
        tail.push(root);
    }
    tail.reverse();
    let name = tail.join(".");
    // A relative alias can share a spelling with an unrelated nominal type.
    if let Some(leaf) = errors[expected].cases.iter().find(|leaf| leaf.name == name) {
        return Ok(leaf);
    }
    if !explicit && tail.len() > 1 && errors.contains_key(nominal) {
        return Err(Diagnostic::new(
            expression.span,
            format!("expected error family `{expected}`, found `{nominal}`"),
        ));
    }
    Err(Diagnostic::new(
        expression.span,
        format!("unknown error case `{expected}.{name}`"),
    ))
}
