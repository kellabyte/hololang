//! Lexical facts for checked constructed-slot cursor induction.
//!
//! A unique checked binding inventory accounts for writes and dynamic scope
//! entry. Exact byte extents come from a bounded, observed-success publication
//! prefix. Capture and interval reasoning allocate only in the compiler.

use std::collections::{HashMap, HashSet};

use crate::ir::{AppendPattern, Argument, Binary, Build, Compare, Condition, Expr, Op, Program};

/// Limit exact publication tracking to a bounded straight-line setup.
const MAX_SETUP_OPS: usize = 256;

/// An unsigned literal or the value of a lexical scalar at the point of use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scalar<'a> {
    /// Exact constant.
    Constant(u64),
    /// Source binding identity. A phase boundary excludes stale aliases.
    Local(&'a str),
}

/// Small expression vocabulary needed by the inductive transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Term<'a> {
    /// Value used without arithmetic.
    Scalar(Scalar<'a>),
    /// Wrapping addition.
    Add(Scalar<'a>, Scalar<'a>),
    /// Wrapping subtraction.
    Sub(Scalar<'a>, Scalar<'a>),
}

/// One frozen byte field published at an exact constructor extent.
#[derive(Clone, Copy)]
pub(super) struct Slot<'a> {
    /// Name of the separate live backing acquisition.
    pub(super) region: &'a str,
    /// Field of a recognized single-byte-field constructor.
    pub(super) field: &'a str,
    /// Constructor actual proven unchanged since publication.
    pub(super) extent: Scalar<'a>,
}

/// Compile-time lexical inventory. Nonunique entry bindings disqualify the
/// analysis rather than risking a proof across shadowed source names.
#[derive(Default)]
pub(super) struct Inventory<'a> {
    /// All declarations in entry, excluding private ordinary function scopes.
    names: HashSet<&'a str>,
    /// Entry scalar initializers, indexed by their unique lexical identities.
    pub(super) declarations: HashMap<&'a str, &'a Expr>,
    /// Declaration block and operation index, needed for dynamic reset boundaries.
    pub(super) locations: HashMap<&'a str, (usize, usize)>,
    /// Parent block and entry operation for each lexical block.
    parents: Vec<Option<(usize, usize)>>,
    /// Every possible scalar write, including nested alternatives.
    writes: HashMap<&'a str, Vec<&'a Expr>>,
    /// Every possible reference rebinding.
    rebinds: HashSet<&'a str>,
    /// Entry blocks scanned in source order.
    pub(super) blocks: Vec<&'a [Op]>,
}

impl<'a> Inventory<'a> {
    /// Record the block and every lexical binding and write in its descendants.
    /// A false result rejects the complete trial analysis.
    pub(super) fn gather(&mut self, operations: &'a [Op], parent: Option<(usize, usize)>) -> bool {
        let block = self.blocks.len();
        self.blocks.push(operations);
        self.parents.push(parent);
        for (offset, operation) in operations.iter().enumerate() {
            // Checked TCP wrappers add no scalar or reference bindings by
            // themselves. Account for the wrapped operation at this site.
            let operation = match operation {
                Op::TcpIo { operation, .. } => operation.as_ref(),
                operation => operation,
            };
            let (binding, children): (Option<&str>, &[&[Op]]) = match operation {
                Op::Let { name, value } => {
                    self.declarations.insert(name, value);
                    self.locations.insert(name, (block, offset));
                    (Some(name), &[])
                }
                Op::Set { name, value } => {
                    self.writes.entry(name).or_default().push(value);
                    (None, &[])
                }
                Op::AssignRef { name, .. } => {
                    self.rebinds.insert(name);
                    (None, &[])
                }
                Op::Region {
                    name,
                    body,
                    on_failure,
                    ..
                } => (Some(name), &[body, on_failure]),
                Op::Batch { name, body, .. } => (Some(name), &[body]),
                Op::TcpScope { stream, body } => (Some(stream), &[body]),
                Op::Repeat { index, body, .. } => (Some(index), &[body]),
                Op::ForEach {
                    reference, body, ..
                } => (Some(reference), &[body]),
                Op::Refill(refill) => {
                    if !self.names.insert(&refill.result_name) {
                        return false;
                    }
                    (
                        Some(&refill.data_name),
                        &[&refill.data_body, &refill.other_body],
                    )
                }
                Op::ConstructHandled { arms, .. } => {
                    // Both the active outcome binding and all possible writes
                    // must enter the inventory before any cursor is selected.
                    for arm in arms {
                        let local = match &arm.pattern {
                            AppendPattern::Rejected { binding, .. } => binding.as_deref(),
                            AppendPattern::Fallback(binding) => binding.as_deref(),
                            AppendPattern::Ok => arm.reference.as_deref(),
                            AppendPattern::Full | AppendPattern::SizeOverflow => None,
                        };
                        if local.is_some_and(|name| !self.names.insert(name))
                            || !self.gather(&arm.body, Some((block, offset)))
                        {
                            return false;
                        }
                    }
                    (None, &[])
                }
                Op::Scope(body) | Op::Loop(body) => (None, &[body]),
                Op::If {
                    then_body,
                    else_body,
                    ..
                } => (None, &[then_body, else_body]),
                Op::Append { status, .. } => (Some(status), &[]),
                Op::AppendHandled { result, .. }
                | Op::WaitInput { result }
                | Op::Send { result, .. }
                | Op::WaitOutput { result }
                | Op::TcpClose { result, .. }
                | Op::NextSession { result } => (Some(result), &[]),
                Op::Borrow { name, .. } | Op::Alias { name, .. } | Op::TryLet { name, .. } => {
                    (Some(name), &[])
                }
                _ => (None, &[]),
            };
            // Unique names rule out a cross-scope alias between proof atoms.
            if binding.is_some_and(|name| !self.names.insert(name))
                || children
                    .iter()
                    .any(|child| !self.gather(child, Some((block, offset))))
            {
                return false;
            }
        }
        true
    }

    /// Count every possible Set to a lexical scalar in entry.
    pub(super) fn writes(&self, name: &str) -> usize {
        self.writes.get(name).map_or(0, Vec::len)
    }

    /// Recover an extent argument's exact invariant value or immutable identity.
    /// A value computed by an arbitrary expression may still be a stable scalar
    /// once stored. Mutated locals cannot be identified with their old capture.
    fn invariant(&self, expr: &'a Expr, fuel: usize) -> Option<Scalar<'a>> {
        if fuel == 0 {
            return None;
        }
        match expr {
            Expr::Const(value) => Some(Scalar::Constant(*value)),
            Expr::Local(name) if self.writes(name) == 0 => match self
                .declarations
                .get(name.as_str())
            {
                Some(value @ (Expr::Const(_) | Expr::Local(_))) => self.invariant(value, fuel - 1),
                _ => Some(Scalar::Local(name)),
            },
            _ => None,
        }
    }

    /// Prove that a dynamic extent is evaluated outside the source cursor's
    /// current lifetime or earlier in its own lexical block. An immutable
    /// Let in a more frequently entered scope could otherwise shrink extent
    /// while an outer cursor still carries its previous value.
    pub(super) fn stable_during(&self, extent: Scalar<'_>, cursor: &str) -> bool {
        let Scalar::Local(extent) = extent else {
            return true;
        };
        let (Some(&(owner, declaration)), Some(&(mut block, mut offset))) =
            (self.locations.get(extent), self.locations.get(cursor))
        else {
            return false;
        };
        // Walk outward to the declaration scope and retain the operation
        // by which execution entered each child.
        while block != owner {
            let Some((parent, entry)) = self.parents[block] else {
                return false;
            };
            block = parent;
            offset = entry;
        }
        declaration < offset
    }

    /// Track prefix publication in direct batch scope. Every predecessor append
    /// must be verified successful before advancing its exact published index.
    /// A nested scope, reset, foreign append or other unknown effect ends this
    /// bounded setup scan. Previously bound exact borrows remain live and frozen.
    pub(super) fn slots(&self, program: &'a Program) -> HashMap<&'a str, Slot<'a>> {
        let mut result = HashMap::new();
        for operations in &self.blocks {
            for operation in *operations {
                let Op::Batch {
                    name, region, body, ..
                } = operation
                else {
                    continue;
                };
                let mut published = Vec::new();
                let mut pending = None;
                for (offset, operation) in body.iter().take(MAX_SETUP_OPS).enumerate() {
                    match operation {
                        Op::Let { .. } => {}
                        Op::Assert(condition) => {
                            if let Some((status, slot)) = pending
                                && equal(
                                    self,
                                    condition,
                                    local(status),
                                    constant(0),
                                    &body[..offset],
                                )
                            {
                                published.push(slot);
                                pending = None;
                            }
                        }
                        Op::Append {
                            batch,
                            producer,
                            args,
                            status,
                        } if batch == name && pending.is_none() => {
                            let Some(producer) =
                                program.producers.iter().find(|p| p.name == *producer)
                            else {
                                break;
                            };
                            let [Build::Bytes { field, len, .. }] = producer.body.as_slice() else {
                                break;
                            };
                            let extent = match len {
                                Expr::Const(value) => Some(Scalar::Constant(*value)),
                                Expr::Param(index) => match args.get(*index) {
                                    Some(Argument::Scalar(actual)) => self.invariant(actual, 16),
                                    _ => None,
                                },
                                _ => None,
                            };
                            let Some(extent) = extent else {
                                break;
                            };
                            if self.writes(status) != 0 {
                                break;
                            }
                            pending = Some((
                                status.as_str(),
                                Slot {
                                    region,
                                    field,
                                    extent,
                                },
                            ));
                        }
                        Op::Borrow {
                            name: reference,
                            batch,
                            index: Expr::Const(index),
                            ..
                        } if batch == name && pending.is_none() => {
                            if !self.rebinds.contains(reference.as_str())
                                && let Ok(index) = usize::try_from(*index)
                                && let Some(slot) = published.get(index)
                            {
                                result.insert(reference.as_str(), *slot);
                            }
                        }
                        _ => break,
                    }
                }
            }
        }
        result
    }

    /// Infer a closed interval over all possible stored values of an entry
    /// scalar. Calls contribute only when every successful return is a byte.
    /// Recursive dependencies, unknown expressions and wrapping arithmetic
    /// that could overflow are rejected.
    pub(super) fn interval(
        &self,
        program: &Program,
        expr: &Expr,
        fuel: &mut usize,
    ) -> Option<(u64, u64)> {
        *fuel = fuel.checked_sub(1)?;
        match expr {
            Expr::Const(value) => Some((*value, *value)),
            Expr::Local(name) => {
                let (mut lo, mut hi) =
                    self.interval(program, self.declarations.get(name.as_str())?, fuel)?;
                for value in self.writes.get(name.as_str()).into_iter().flatten() {
                    let (next_lo, next_hi) = self.interval(program, value, fuel)?;
                    lo = lo.min(next_lo);
                    hi = hi.max(next_hi);
                }
                Some((lo, hi))
            }
            Expr::Binary {
                op: Binary::AddWrapping,
                left,
                right,
            } => {
                let (left_lo, left_hi) = self.interval(program, left, fuel)?;
                let (right_lo, right_hi) = self.interval(program, right, fuel)?;
                Some((
                    left_lo.checked_add(right_lo)?,
                    left_hi.checked_add(right_hi)?,
                ))
            }
            Expr::Byte { .. } | Expr::InputByte { .. } => Some((0, 255)),
            Expr::Call { function, .. } => {
                let function = program.body.iter().find_map(|operation| match operation {
                    Op::Function(candidate) if candidate.name == *function => Some(candidate),
                    _ => None,
                })?;
                let (last, preceding) = function.body.split_last()?;
                // Straight-line setup and asserts cannot produce another return.
                (matches!(last, Op::Return(Expr::InputByte { .. }))
                    && preceding
                        .iter()
                        .all(|operation| matches!(operation, Op::Let { .. } | Op::Assert(_))))
                .then_some((0, 255))
            }
            _ => None,
        }
    }
}

/// Construct a named proof operand.
pub(super) fn local(name: &str) -> Term<'_> {
    Term::Scalar(Scalar::Local(name))
}

/// Construct a constant proof operand.
pub(super) fn constant(value: u64) -> Term<'static> {
    Term::Scalar(Scalar::Constant(value))
}

/// Resolve a generated scalar capture inside the current no-mutation phase.
/// Searching a prefix enforces actual evaluation order. Unsupported nested
/// arithmetic fails the proof rather than changing source arithmetic.
pub(super) fn term<'a>(
    inventory: &Inventory<'_>,
    expression: &'a Expr,
    prefix: &'a [Op],
    fuel: usize,
) -> Option<Term<'a>> {
    if fuel == 0 {
        return None;
    }
    match expression {
        Expr::Const(value) => Some(constant(*value)),
        Expr::Local(name) => {
            // Mutable bindings are phase atoms, never resolved through their
            // initializer. This also rejects temporary aliases with nested Sets.
            if inventory.writes(name) != 0 {
                return Some(local(name));
            }
            for (index, op) in prefix.iter().enumerate().rev() {
                match op {
                    Op::Set {
                        name: candidate, ..
                    } if candidate == name => break,
                    Op::Let {
                        name: candidate,
                        value,
                    } if candidate == name => {
                        return term(inventory, value, &prefix[..index], fuel - 1);
                    }
                    _ => {}
                }
            }
            Some(local(name))
        }
        Expr::Binary { op, left, right } => {
            let Term::Scalar(left) = term(inventory, left, prefix, fuel - 1)? else {
                return None;
            };
            let Term::Scalar(right) = term(inventory, right, prefix, fuel - 1)? else {
                return None;
            };
            match op {
                Binary::AddWrapping => Some(Term::Add(left, right)),
                Binary::SubWrapping => Some(Term::Sub(left, right)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Match an exact scalar capture at the comparison point, stopping at its
/// declared identity rather than expanding the amount's pre-minimum value.
pub(super) fn captures_local(
    inventory: &Inventory<'_>,
    expr: &Expr,
    name: &str,
    prefix: &[Op],
    fuel: usize,
) -> bool {
    let Expr::Local(candidate) = expr else {
        return false;
    };
    if candidate == name {
        return true;
    }
    if fuel == 0 || inventory.writes(candidate) != 0 {
        return false;
    }
    for (index, operation) in prefix.iter().enumerate().rev() {
        match operation {
            Op::Set { name, .. } if name == candidate => return false,
            Op::Let {
                name: capture,
                value,
            } if capture == candidate => {
                return captures_local(inventory, value, name, &prefix[..index], fuel - 1);
            }
            _ => {}
        }
    }
    false
}

/// Test exact unsigned equality, accepting either written operand order.
pub(super) fn equal(
    inventory: &Inventory<'_>,
    condition: &Condition,
    lhs: Term<'_>,
    rhs: Term<'_>,
    prefix: &[Op],
) -> bool {
    matches!(condition.op, Compare::Eq)
        && (terms_match(inventory, condition, lhs, rhs, prefix)
            || terms_match(inventory, condition, rhs, lhs, prefix))
}

/// Compare both operands after resolving their ordered scalar captures.
fn terms_match(
    inventory: &Inventory<'_>,
    condition: &Condition,
    lhs: Term<'_>,
    rhs: Term<'_>,
    prefix: &[Op],
) -> bool {
    term(inventory, &condition.left, prefix, 16) == Some(lhs)
        && term(inventory, &condition.right, prefix, 16) == Some(rhs)
}

/// Check an existing assertion of an unsigned upper bound.
fn proves_le(
    inventory: &Inventory<'_>,
    condition: &Condition,
    lhs: Term<'_>,
    rhs: Term<'_>,
    prefix: &[Op],
) -> bool {
    match condition.op {
        Compare::Lt | Compare::Le => terms_match(inventory, condition, lhs, rhs, prefix),
        Compare::Gt | Compare::Ge => terms_match(inventory, condition, rhs, lhs, prefix),
        Compare::Eq => equal(inventory, condition, lhs, rhs, prefix),
        Compare::Ne => false,
    }
}

/// Find a dominating direct assertion in a sequence with fixed proof atoms.
pub(super) fn guard(
    inventory: &Inventory<'_>,
    prefix: &[Op],
    lhs: Term<'_>,
    rhs: Term<'_>,
) -> bool {
    prefix.iter().enumerate().any(|(index, operation)| {
        matches!(operation, Op::Assert(condition) if proves_le(inventory, condition, lhs, rhs, &prefix[..index]))
    })
}
