//! Source schema resolution before constructor lowering.
//!
//! Inline u64, bool, Bytes, and named-record fields are representable. Graph
//! validation rejects unknown types and recursive layouts before a constructor
//! can be treated as checked. All maps and work lists are compiler-only storage.

use super::{MAX_DEPTH, ast};
use crate::ir;
use crate::source::Diagnostic;
use std::collections::{HashMap, HashSet};

/// Resolve all fields without reordering declarations and reject cyclic inline types.
/// The returned schemas own names; borrowed source declarations retain diagnostic spans.
pub(super) fn lower(
    module: &ast::Module,
    enums: &super::EnumNames,
) -> Result<Vec<ir::Record>, Diagnostic> {
    let mut records = HashMap::new();
    // Builtin type identities cannot be shadowed, because fields use their fixed scalar meanings.
    for record in &module.records {
        if matches!(record.name.as_str(), "u64" | "bool" | "Bytes" | "Unit") {
            return Err(Diagnostic::new(
                record.span,
                format!(
                    "a struct cannot redefine the builtin `{}` type",
                    record.name
                ),
            ));
        }
        if records.insert(record.name.as_str(), record).is_some() {
            return Err(Diagnostic::new(
                record.span,
                format!("duplicate struct `{}`", record.name),
            ));
        }
    }
    let mut result = Vec::with_capacity(module.records.len());
    // Every declaration is checked even when no entry operation constructs it.
    for record in &module.records {
        let mut names = HashSet::new();
        let mut fields = Vec::with_capacity(record.fields.len());
        for field in &record.fields {
            if !names.insert(field.name.as_str()) {
                return Err(Diagnostic::new(
                    field.span,
                    format!("duplicate field `{}`", field.name),
                ));
            }
            let kind = match field.ty.as_str() {
                "u64" => ir::FieldKind::U64,
                "bool" => ir::FieldKind::Bool,
                "Bytes" => ir::FieldKind::Bytes,
                name if enums.contains_key(name) => ir::FieldKind::Enum(name.to_owned()),
                name if records.contains_key(name) => ir::FieldKind::Record(name.to_owned()),
                name => {
                    return Err(Diagnostic::new(
                        field.span,
                        format!("unsupported or unknown field type `{name}`"),
                    ));
                }
            };
            fields.push(ir::Field {
                name: field.name.clone(),
                kind,
            });
        }
        result.push(ir::Record {
            name: record.name.clone(),
            fields,
        });
    }
    // Explicit enter/exit events avoid recursive traversal of malformed schema graphs.
    let mut colors = HashMap::new();
    let mut heights = HashMap::new();
    for record in &module.records {
        let mut pending = vec![(record, false, 0usize)];
        while let Some((current, exiting, depth)) = pending.pop() {
            if exiting {
                let mut height = 1usize;
                // Finished child heights make the limit independent of declaration/cache order.
                for field in &current.fields {
                    if let Some(child_height) = heights.get(field.ty.as_str()) {
                        height = height.max(child_height + 1);
                    }
                }
                if height > MAX_DEPTH {
                    return Err(Diagnostic::new(
                        current.span,
                        "inline schema nesting exceeds the experimental depth limit of 64",
                    ));
                }
                heights.insert(current.name.as_str(), height);
                colors.insert(current.name.as_str(), 2u8);
                continue;
            }
            match colors.get(current.name.as_str()).copied().unwrap_or(0) {
                1 => {
                    return Err(Diagnostic::new(
                        current.span,
                        format!(
                            "recursive inline struct graph at `{}` is unsupported",
                            current.name
                        ),
                    ));
                }
                2 => continue,
                _ => {}
            }
            if depth >= MAX_DEPTH {
                return Err(Diagnostic::new(
                    current.span,
                    "inline schema nesting exceeds the experimental depth limit of 64",
                ));
            }
            colors.insert(current.name.as_str(), 1);
            pending.push((current, true, depth));
            // Reversed pushes preserve declaration order when reporting the first bad dependency.
            for field in current.fields.iter().rev() {
                if let Some(child) = records.get(field.ty.as_str()) {
                    pending.push((child, false, depth + 1));
                }
            }
        }
    }
    Ok(result)
}
