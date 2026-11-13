//! Schema, destination producer, and ordinary helper validation before entry checking.
//!
//! Named inline schemas and producer calls must be acyclic, fields are initialized
//! exactly once in declaration order, and constructor expressions access
//! published storage only through typed readonly input parameters. Ordinary
//! bodies have separately validated local frames and a closed call graph. Host lookup
//! maps and graph work lists are compiler costs; no target allocations or
//! per-object metadata are introduced by this phase.

use super::CheckError;
use super::expressions::{self, Context};
use super::scopes;
use crate::ir::{
    Build, ErrorCase, ErrorFamily, FieldKind, Function, OPTION_U64_FAMILY, Op, Producer, Program,
    Record,
};
use std::collections::{HashMap, HashSet};

/// Immutable name tables retained for lexical and projection validation.
pub(super) struct Definitions<'p> {
    /// Unique record definitions borrowing the input program.
    records: HashMap<&'p str, &'p Record>,
    /// Unique destination producers borrowing the input program.
    producers: HashMap<&'p str, &'p Producer>,
    /// Statically resolved synchronous helpers with independently checked bodies.
    functions: HashMap<&'p str, &'p Function>,
    /// Closed nominal error families used by ordinary results and destination producers.
    errors: HashMap<&'p str, &'p ErrorFamily>,
}

impl<'p> Definitions<'p> {
    /// Resolve a record name, rejecting a missing schema before lowering.
    pub(super) fn record(&self, name: &str) -> Result<&'p Record, CheckError> {
        self.records
            .get(name)
            .copied()
            .ok_or_else(|| CheckError::new(format!("unknown record {name}")))
    }

    /// Resolve a producer name, rejecting unknown effects/targets by default.
    pub(super) fn producer(&self, name: &str) -> Result<&'p Producer, CheckError> {
        self.producers
            .get(name)
            .copied()
            .ok_or_else(|| CheckError::new(format!("unknown producer {name}")))
    }

    /// Resolve an ordinary call target, rejecting unknown effects before lowering.
    pub(super) fn function(&self, name: &str) -> Result<&'p Function, CheckError> {
        self.functions
            .get(name)
            .copied()
            .ok_or_else(|| CheckError::new(format!("unknown ordinary function {name}")))
    }

    /// Look up a static family; no runtime registry or numeric case fabrication is allowed.
    pub(super) fn error_family(&self, name: &str) -> Result<&'p ErrorFamily, CheckError> {
        self.errors
            .get(name)
            .copied()
            .ok_or_else(|| CheckError::new(format!("unknown error family {name}")))
    }

    /// Resolve `family.case` and its payload shape against the expected nominal identity.
    pub(super) fn error_case(
        &self,
        expected: &str,
        family: &str,
        case: &str,
    ) -> Result<&'p ErrorCase, CheckError> {
        // Equally spelled cases in distinct families cannot cross this boundary.
        if family != expected {
            return Err(CheckError::new(format!(
                "expected error family {expected}, found {family}"
            )));
        }
        // Only declared cases can be returned or matched by a handler.
        self.error_family(family)?
            .cases
            .iter()
            .find(|declared| declared.name == case)
            .ok_or_else(|| CheckError::new(format!("unknown error case {family}.{case}")))
    }

    /// Check that an ordinary state parameter has only scalar leaves. The
    /// compiler worklist visits each schema once without recursing on the stack.
    pub(super) fn require_scalar_record(&self, name: &'p str) -> Result<(), CheckError> {
        let mut pending = vec![name];
        let mut visited = HashSet::new();
        // Shared nested schemas need only one inspection for this use.
        while let Some(record) = pending.pop() {
            if !visited.insert(record) {
                continue;
            }
            // Following logical fields excludes byte payloads and representation metadata.
            for field in &self.record(record)?.fields {
                match &field.kind {
                    FieldKind::U64 => {}
                    FieldKind::Record(child) => pending.push(child),
                    FieldKind::Bytes => {
                        return Err(CheckError::new(format!(
                            "ordinary state parameter {name} must have only fixed scalar fields"
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Validate the program's finite tables and return borrowed lookup structures.
pub(super) fn validate(program: &Program) -> Result<Definitions<'_>, CheckError> {
    let mut definitions = Definitions {
        records: HashMap::new(),
        producers: HashMap::new(),
        functions: HashMap::new(),
        errors: HashMap::new(),
    };
    // Unique nonempty identities prevent ambiguous schema resolution or diagnostics.
    for record in &program.records {
        if record.name.is_empty() || definitions.records.insert(&record.name, record).is_some() {
            return Err(CheckError::new(format!(
                "duplicate or empty record name {}",
                record.name
            )));
        }
    }
    // Producer names are resolved statically; duplicates cannot select arbitrary effects.
    for producer in &program.producers {
        if producer.name.is_empty()
            || definitions
                .producers
                .insert(&producer.name, producer)
                .is_some()
        {
            return Err(CheckError::new(format!(
                "duplicate or empty producer name {}",
                producer.name
            )));
        }
    }
    let mut record_edges = HashMap::new();
    // Field identities and kinds are checked before graph traversal can follow them.
    for record in &program.records {
        let mut names = HashSet::new();
        let mut nested = Vec::new();
        for field in &record.fields {
            if field.name.is_empty() || !names.insert(field.name.as_str()) {
                return Err(CheckError::new(format!(
                    "duplicate or empty field in record {}",
                    record.name
                )));
            }
            if let FieldKind::Record(name) = &field.kind {
                definitions.record(name)?;
                nested.push(name.as_str());
            }
        }
        record_edges.insert(record.name.as_str(), nested);
    }
    acyclic(&record_edges, "inline record")?;
    validate_error_families(&program.body, &mut definitions)?;
    let mut producer_edges = HashMap::new();
    // Complete ordered initialization and direct calls establish destination-only effects.
    for producer in &program.producers {
        let result = definitions.record(&producer.record)?;
        // Typed construction carries its error outside the destination suffix.
        // Both conventions use the same field, input, and construction checks.
        if let Some(family) = producer.error.as_deref() {
            if family == OPTION_U64_FAMILY {
                return Err(CheckError::new(
                    "destination constructors cannot return Option<u64>",
                ));
            }
            definitions.error_family(family)?;
        }
        let context = Context::Producer {
            params: &producer.params,
            index: false,
        };
        // A whole-result forward owns no part of initialization. Its callee
        // has the same ABI and owns every check and write for this destination.
        if let [
            Build::Forward {
                producer: callee,
                args,
            },
        ] = producer.body.as_slice()
        {
            let destination = definitions.producer(callee)?;
            if destination.record != producer.record || destination.error != producer.error {
                return Err(CheckError::new(format!(
                    "producer {} must forward the exact record and error family",
                    producer.name
                )));
            }
            expressions::arguments(args, &destination.params, &definitions, context)?;
            producer_edges.insert(producer.name.as_str(), vec![callee.as_str()]);
            continue;
        }
        let mut next_field = 0;
        let mut calls = Vec::new();
        for step in &producer.body {
            // Rejection does not initialize a field; all potential success paths still need one.
            match step {
                Build::FailIf(condition) => {
                    // Legacy failures carry an untyped status and cannot enter a typed path.
                    if producer.error.is_some() {
                        return Err(CheckError::new("typed producer requires named rejection"));
                    }
                    expressions::condition(condition, &definitions, context)?;
                    continue;
                }
                Build::Reject {
                    condition,
                    case,
                    payload,
                } => {
                    let family = producer.error.as_deref().ok_or_else(|| {
                        CheckError::new("named rejection requires a typed producer")
                    })?;
                    // A producer checks local guards before reserving its fields.
                    // Its layout prefix and enclosing records can already occupy space.
                    if next_field != 0 {
                        return Err(CheckError::new(
                            "named rejection must precede producer fields",
                        ));
                    }
                    let declared = definitions.error_case(family, family, case)?;
                    // Reject malformed payloads before checking the scalar expression.
                    if declared.payload != payload.is_some() {
                        return Err(CheckError::new(format!(
                            "error case {family}.{case} has incompatible payload"
                        )));
                    }
                    expressions::condition(condition, &definitions, context)?;
                    if let Some(value) = payload {
                        expressions::expression(value, &definitions, context)?;
                    }
                    continue;
                }
                Build::Forward { .. } => {
                    return Err(CheckError::new(
                        "whole-result forwarding must be the only producer operation",
                    ));
                }
                _ => {}
            }
            let expected = result.fields.get(next_field).ok_or_else(|| {
                CheckError::new(format!(
                    "producer {} initializes too many fields",
                    producer.name
                ))
            })?;
            let (name, compatible) = match step {
                Build::U64 { field, value } => {
                    expressions::expression(
                        value,
                        &definitions,
                        Context::Producer {
                            params: &producer.params,
                            index: false,
                        },
                    )?;
                    (field.as_str(), matches!(&expected.kind, FieldKind::U64))
                }
                Build::Bytes { field, len, value } => {
                    expressions::expression(
                        len,
                        &definitions,
                        Context::Producer {
                            params: &producer.params,
                            index: false,
                        },
                    )?;
                    expressions::expression(
                        value,
                        &definitions,
                        Context::Producer {
                            params: &producer.params,
                            index: true,
                        },
                    )?;
                    (field.as_str(), matches!(&expected.kind, FieldKind::Bytes))
                }
                Build::Record {
                    field,
                    producer: callee,
                    args,
                } => {
                    let callee_definition = definitions.producer(callee)?;
                    // Keep the numeric convention separate. Typed failures lift only
                    // through an explicitly declared direct child with the same payload lane.
                    let compatible = match (&producer.error, &callee_definition.error) {
                        (None, None) => true,
                        (Some(target), Some(source)) => definitions
                            .error_family(target)?
                            .try_error_offset(source)
                            .is_some(),
                        _ => false,
                    };
                    if !compatible {
                        return Err(CheckError::new(format!(
                            "nested producer {callee} must have the same error family as {} or a declared child",
                            producer.name
                        )));
                    }
                    if args.len() != callee_definition.params.len() {
                        return Err(CheckError::new(format!(
                            "producer {callee} has wrong argument count"
                        )));
                    }
                    // Typed forwarding preserves readonly inputs across the closed call graph.
                    expressions::arguments(
                        args,
                        &callee_definition.params,
                        &definitions,
                        Context::Producer {
                            params: &producer.params,
                            index: false,
                        },
                    )?;
                    calls.push(callee.as_str());
                    (
                        field.as_str(),
                        matches!(&expected.kind, FieldKind::Record(name) if name == &callee_definition.record),
                    )
                }
                Build::FailIf(_) | Build::Reject { .. } | Build::Forward { .. } => {
                    unreachable!("non-field steps were handled before field initialization")
                }
            };
            if name != expected.name || !compatible {
                return Err(CheckError::new(format!(
                    "producer {} must initialize field {} with its declared type in order",
                    producer.name, expected.name
                )));
            }
            next_field += 1;
        }
        if next_field != result.fields.len() {
            return Err(CheckError::new(format!(
                "producer {} leaves fields uninitialized",
                producer.name
            )));
        }
        producer_edges.insert(producer.name.as_str(), calls);
    }
    acyclic(&producer_edges, "producer call")?;
    // Register signatures before checking bodies, allowing forward calls while
    // keeping constructors and ordinary helpers separately checked.
    for operation in &program.body {
        if let Op::Function(function) = operation
            && (function.name.is_empty()
                || definitions.producers.contains_key(function.name.as_str())
                || definitions
                    .functions
                    .insert(&function.name, function)
                    .is_some())
        {
            return Err(CheckError::new(format!(
                "duplicate or empty ordinary function name {}",
                function.name
            )));
        }
    }
    let mut function_edges = HashMap::new();
    // Check even unused helpers in source order. Every edge comes from a
    // validated call and therefore has known effects within this closed graph.
    for operation in &program.body {
        if let Op::Function(function) = operation {
            if function.returns_bool && function.result.is_some() {
                return Err(
                    CheckError::new("boolean return cannot carry a result family")
                        .in_function(&function.name),
                );
            }
            // Even unused typed functions need a valid nominal result family.
            if let Some(family) = &function.result {
                definitions
                    .error_family(family)
                    .map_err(|error| error.in_function(&function.name))?;
            }
            let calls = scopes::validate_function(function, &definitions)
                .map_err(|error| error.in_function(&function.name))?;
            function_edges.insert(function.name.as_str(), calls);
        }
    }
    acyclic(&function_edges, "ordinary call")?;
    Ok(definitions)
}

/// Register and validate closed unions before signatures can use their tags.
///
/// Host maps resolve forward references and finite child graphs. Requiring each
/// child's exact contiguous leaf block makes a checked lift a constant offset,
/// with the original payload representation and no target lookup.
fn validate_error_families<'p>(
    body: &'p [Op],
    definitions: &mut Definitions<'p>,
) -> Result<(), CheckError> {
    let families: Vec<_> = body
        .iter()
        .filter_map(|operation| match operation {
            Op::ErrorFamily(family) => Some(family),
            _ => None,
        })
        .collect();
    // Register all names before following any child edge.
    for family in &families {
        // The type namespace is shared by records and error families.
        if family.name.is_empty()
            || definitions.records.contains_key(family.name.as_str())
            || definitions.errors.insert(&family.name, family).is_some()
        {
            return Err(CheckError::new(format!(
                "duplicate or empty error family name {}",
                family.name
            )));
        }
        if family.name == OPTION_U64_FAMILY
            && (!family.children.is_empty()
                || !matches!(family.cases.as_slice(), [case] if case.name == "None" && !case.payload))
        {
            return Err(CheckError::new(
                "Option<u64> requires a single tag-only None case",
            ));
        }
        // Ordinary result tags fit in a byte and reserve zero for success.
        if family.cases.is_empty() || family.cases.len() > usize::from(u8::MAX) {
            return Err(CheckError::new(format!(
                "error family {} must declare between 1 and 255 cases",
                family.name
            )));
        }
        let mut names = HashSet::new();
        // Reject duplicates before any handler can select a case by name.
        for case in &family.cases {
            if case.name.is_empty() || !names.insert(case.name.as_str()) {
                return Err(CheckError::new(format!(
                    "duplicate or empty case in error family {}",
                    family.name
                )));
            }
        }
    }

    let mut edges = HashMap::new();
    // Validate child identity independently of the leaf representation, then
    // reject cycles even when fabricated leaves would also be malformed.
    for family in &families {
        let mut aliases = HashSet::new();
        let mut sources = HashSet::new();
        let mut children = Vec::new();
        // Each alias identifies one subtree and each source has one possible lift.
        for child in &family.children {
            if child.family == OPTION_U64_FAMILY {
                return Err(CheckError::new(
                    "Option<u64> cannot be an error union child",
                ));
            }
            if child.alias.is_empty()
                || child.alias.contains('.')
                || !aliases.insert(child.alias.as_str())
            {
                return Err(CheckError::new(format!(
                    "invalid or duplicate child alias {} in error family {}",
                    child.alias, family.name
                )));
            }
            if !sources.insert(child.family.as_str()) {
                return Err(CheckError::new(format!(
                    "duplicate child family {} in error family {}",
                    child.family, family.name
                )));
            }
            children.push(definitions.error_family(&child.family)?.name.as_str());
        }
        edges.insert(family.name.as_str(), children);
    }
    acyclic(&edges, "error family")?;

    // Every child is independently verified, so this check handles arbitrary
    // leaf depth without recursive path validation or additional runtime data.
    for family in families {
        validate_error_leaves(family, definitions)?;
    }
    Ok(())
}

/// Verify the parent's ordered leaves encode exact copies of its children.
/// The scan uses only compiler-side iterators over the borrowed declarations.
fn validate_error_leaves(
    family: &ErrorFamily,
    definitions: &Definitions<'_>,
) -> Result<(), CheckError> {
    let mut children = family.children.iter();
    let mut index = 0;
    // A direct leaf consumes one tag, while a child consumes its whole tag range.
    while let Some(case) = family.cases.get(index) {
        let Some((alias, _)) = case.name.split_once('.') else {
            // Even an already verified child reserves its alias against direct leaves.
            if family.children.iter().any(|child| child.alias == case.name) {
                return Err(CheckError::new(format!(
                    "direct case {} conflicts with a child alias in error family {}",
                    case.name, family.name
                )));
            }
            index += 1;
            continue;
        };
        // Dotted paths must start the next declared block. This also rejects
        // unlisted paths, split blocks, and blocks in a different order.
        let child = children
            .next()
            .filter(|child| child.alias == alias)
            .ok_or_else(|| {
                CheckError::new(format!(
                    "unexpected child leaf {} in error family {}",
                    case.name, family.name
                ))
            })?;
        let child_cases = &definitions.error_family(&child.family)?.cases;
        let end = index + child_cases.len();
        // Both the names and payload flags must agree at every mapped ordinal.
        let exact = family.cases.get(index..end).is_some_and(|leaves| {
            leaves.iter().zip(child_cases).all(|(leaf, child_case)| {
                leaf.name.split_once('.') == Some((alias, child_case.name.as_str()))
                    && leaf.payload == child_case.payload
            })
        });
        if !exact {
            return Err(CheckError::new(format!(
                "child {alias} in error family {} must have a contiguous block of exact leaves",
                family.name
            )));
        }
        index = end;
    }
    // A remaining child would make one of its variants impossible to lift.
    if let Some(child) = children.next() {
        return Err(CheckError::new(format!(
            "missing leaves for child {} in error family {}",
            child.alias, family.name
        )));
    }
    Ok(())
}

/// Reject cycles in a previously resolved named dependency graph.
///
/// The iterative depth-first traversal colors active and finished nodes. Its
/// memory is proportional to the finite definitions/edges, avoiding recursive
/// checker calls for deeply nested schemas or producer chains.
fn acyclic<'p>(
    edges: &HashMap<&'p str, Vec<&'p str>>,
    description: &str,
) -> Result<(), CheckError> {
    let mut colors = HashMap::new();
    let mut roots: Vec<_> = edges.keys().copied().collect();
    roots.sort_unstable();
    // Sorted seeds keep diagnostics independent of randomized hash-table order.
    for root in roots {
        let mut pending = vec![(root, false)];
        while let Some((name, exiting)) = pending.pop() {
            if exiting {
                colors.insert(name, 2_u8);
                continue;
            }
            match colors.get(name).copied().unwrap_or(0) {
                1 => {
                    return Err(CheckError::new(format!(
                        "recursive {description} graph at {name} is unsupported"
                    )));
                }
                2 => continue,
                _ => {}
            }
            colors.insert(name, 1);
            pending.push((name, true));
            // Dependencies finish before their parent is colored complete.
            for child in edges[name].iter().rev() {
                pending.push((child, false));
            }
        }
    }
    Ok(())
}
