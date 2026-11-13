//! Select checked staging loops by source and receiving cursor induction.
//!
//! Every accepted scalar write must preserve the frozen slot bounds. The
//! immutable checked program supplies lexical scope and effect identities.
//! This module selects a plan and emits no runtime proof checks.

use std::collections::HashMap;

use crate::ir::{Build, Compare, Expr, Op, Program};

use super::{
    cursor_inventory::{
        Inventory, Scalar, Term, captures_local, constant, equal, guard, local, term,
    },
    staging_copy::{Base, ranges},
};

/// Bound normalization scans for one transition independently of module size.
const MAX_PROOF_OPS: usize = 256;
/// A recognized forward byte write needs only a small capture sequence.
const MAX_COPY_OPS: usize = 16;

/// Compiler-only maximum transfer widths keyed by proved copy-body identity.
/// The emitter compares these addresses without dereferencing them.
pub(super) type Plan = HashMap<*const Op, u64>;

/// Validate an exact direct cursor advance in the mutation-free prefix.
fn advance<'a>(
    inventory: &Inventory<'_>,
    operations: &'a [Op],
    name: &'a str,
    amount: &'a str,
) -> Option<usize> {
    for (index, operation) in operations.iter().enumerate() {
        if let Op::Set {
            name: candidate,
            value,
        } = operation
            && candidate == name
        {
            let lhs = Scalar::Local(name);
            let rhs = Scalar::Local(amount);
            let value = term(inventory, value, &operations[..index], 16);
            return (value == Some(Term::Add(lhs, rhs)) || value == Some(Term::Add(rhs, lhs)))
                .then_some(index);
        }
    }
    None
}

/// Count writes to the proved receiver state in an alternative. Only direct
/// writes are admitted. The caller compares this accounted count with the
/// inventory, which includes writes in every nested alternative and loop.
fn transition<'a>(
    program: &Program,
    inventory: &Inventory<'a>,
    body: &'a [Op],
    cursor: &'a str,
    required: &'a str,
    limit: u64,
    known_cursor: Option<u64>,
) -> Option<(usize, usize)> {
    if body.len() > MAX_PROOF_OPS {
        return None;
    }
    let mut receiver = None;
    let mut boundary = None;
    for (index, operation) in body.iter().enumerate() {
        // Duplicate writes are outside this simple transition shape.
        if let Op::Set { name, value } = operation
            && (name == cursor && receiver.replace(value).is_some()
                || name == required && boundary.replace((index, value)).is_some())
        {
            return None;
        }
    }
    match (receiver, boundary) {
        (None, None) => Some((0, 0)),
        (Some(Expr::Const(start)), Some((_, Expr::Const(end))))
            if start <= end && *end <= limit =>
        {
            Some((1, 1))
        }
        (None, Some((index, value))) => {
            let prefix = &body[..index];
            let bounded = known_cursor.is_some_and(|current| {
                inventory
                    .interval(program, value, &mut 128)
                    .is_some_and(|(lo, hi)| current <= lo && hi <= limit)
            });
            // A guarded dynamic boundary can be installed only after its
            // lower and upper inequalities both hold on this path.
            if bounded
                || term(inventory, value, prefix, 16).is_some_and(|end| {
                    matches!(end, Term::Scalar(Scalar::Local(name)) if inventory.writes(name) == 0)
                        && guard(inventory, prefix, local(cursor), end)
                        && guard(inventory, prefix, end, constant(limit))
                })
            {
                Some((0, 1))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Require one loop-carried receiver transition after the exact append.
/// A fresh initialization starts below the field limit. After the copy
/// advance, the old required bound holds until a verified branch changes it.
fn receiving_transition<'a>(
    program: &Program,
    inventory: &Inventory<'a>,
    tail: &'a [Op],
    cursor: &'a str,
    required: &'a str,
    limit: u64,
) -> bool {
    let mut cursor_writes = 1;
    let mut required_writes = 0;
    let mut changed = false;
    for (index, operation) in tail.iter().enumerate() {
        if let Op::If {
            condition,
            then_body,
            else_body,
        } = operation
        {
            let known = match term(inventory, &condition.right, &tail[..index], 16) {
                Some(Term::Scalar(Scalar::Constant(value)))
                    if equal(
                        inventory,
                        condition,
                        local(cursor),
                        constant(value),
                        &tail[..index],
                    ) =>
                {
                    Some(value)
                }
                _ => None,
            };
            let Some((then_cursor, then_required)) = transition(
                program, inventory, then_body, cursor, required, limit, known,
            ) else {
                return false;
            };
            let Some((else_cursor, else_required)) =
                transition(program, inventory, else_body, cursor, required, limit, None)
            else {
                return false;
            };
            let modifies = then_cursor + then_required + else_cursor + else_required != 0;
            // Prevent captures from a previous changed phase from proving this one.
            if changed && modifies {
                return false;
            }
            changed |= modifies;
            cursor_writes += then_cursor + else_cursor;
            required_writes += then_required + else_required;
        }
    }
    inventory.writes(cursor) == cursor_writes && inventory.writes(required) == required_writes
}

/// Two exact slots and the three scalars used by their shared transfer.
struct CursorPair<'a> {
    /// Monotonic source cursor.
    source: &'a str,
    /// Bounded receiving cursor.
    destination: &'a str,
    /// Local narrowed to the receiving room before the copy.
    amount: &'a str,
    /// Frozen exact initialized source extent.
    source_extent: Scalar<'a>,
    /// Frozen exact initialized destination extent.
    destination_extent: u64,
}

/// Recognize the proved guard and the two nonwrapping cursor transitions.
/// Global write counts close the induction even across enclosing counted loops:
/// every write from the exact zero initial state is accounted for here.
fn proved<'a>(
    program: &Program,
    inventory: &Inventory<'a>,
    before: &'a [Op],
    after: &'a [Op],
    pair: CursorPair<'a>,
) -> bool {
    let CursorPair {
        source: source_cursor,
        destination: destination_cursor,
        amount,
        source_extent,
        destination_extent,
    } = pair;
    if source_cursor == destination_cursor
        || source_cursor == amount
        || destination_cursor == amount
        || inventory.writes(source_cursor) != 1
        || inventory.writes(amount) != 1
        || !matches!(
            inventory.declarations.get(source_cursor),
            Some(Expr::Const(0))
        )
        || !matches!(
            inventory.declarations.get(destination_cursor),
            Some(Expr::Const(0))
        )
    {
        return false;
    }
    let destination = Scalar::Local(destination_cursor);
    let source = Scalar::Local(source_cursor);
    let Some((minimum, required)) = before.iter().enumerate().find_map(|(index, operation)| {
        let Op::If {
            condition,
            then_body,
            else_body,
        } = operation
        else {
            return None;
        };
        let [Op::Set { name, value }] = then_body.as_slice() else {
            return None;
        };
        if name != amount || !else_body.is_empty() || !matches!(condition.op, Compare::Lt) {
            return None;
        }
        let room = term(inventory, value, &before[..index], 16)?;
        let Term::Sub(Scalar::Local(required), cursor) = room else {
            return None;
        };
        (required != source_cursor
            && required != destination_cursor
            && required != amount
            && cursor == destination
            && term(inventory, &condition.left, &before[..index], 16) == Some(room)
            && captures_local(inventory, &condition.right, amount, &before[..index], 16))
        .then_some((index, required))
    }) else {
        return false;
    };
    // The min reads the initial amount, so treat its declaration as an atom in
    // the phase of comparison. Other captures before the min cannot see a write.
    let same_lifetime = inventory
        .locations
        .get(required)
        .zip(inventory.locations.get(destination_cursor))
        .is_some_and(|(required, cursor)| required.0 == cursor.0);
    if !same_lifetime
        || !inventory.stable_during(source_extent, source_cursor)
        || !matches!(inventory.declarations.get(required), Some(Expr::Const(initial)) if *initial <= destination_extent)
        || !guard(
            inventory,
            &before[minimum + 1..],
            local(amount),
            Term::Sub(source_extent, source),
        )
    {
        return false;
    }
    let Some(source_update) = advance(inventory, after, source_cursor, amount) else {
        return false;
    };
    let Some(destination_update) = advance(
        inventory,
        &after[source_update + 1..],
        destination_cursor,
        amount,
    ) else {
        return false;
    };
    let destination_update = source_update + 1 + destination_update;
    // Neither tracked receiver may be changed before its checked advance.
    let no_receiver_change = |operation: &Op| !matches!(operation, Op::Set { name, .. } if name == destination_cursor || name == required);
    if !after[..destination_update].iter().all(no_receiver_change) {
        return false;
    }
    receiving_transition(
        program,
        inventory,
        &after[destination_update + 1..],
        destination_cursor,
        required,
        destination_extent,
    )
}

/// Build the maximum transfer width for each proved copy body. Slice identities
/// are only used for lookup while lowering the same immutable checked Program.
pub(super) fn plan(program: &Program) -> Plan {
    // Without a direct byte constructor there can be no exact slot pair.
    if !program
        .producers
        .iter()
        .any(|producer| matches!(producer.body.as_slice(), [Build::Bytes { .. }]))
    {
        return Plan::new();
    }
    let mut inventory = Inventory::default();
    if !inventory.gather(&program.body, None) {
        return Plan::new();
    }
    let slots = inventory.slots(program);
    let mut result = Plan::new();
    if slots.len() < 2 {
        return result;
    }
    for operations in &inventory.blocks {
        // Matching captures scans prefixes of this block. Bound that work
        // independently of the size of the surrounding checked program.
        if operations.len() > MAX_PROOF_OPS {
            continue;
        }
        for (offset, operation) in operations.iter().enumerate() {
            let Op::Repeat {
                count: Expr::Local(amount),
                index,
                body,
            } = operation
            else {
                continue;
            };
            if body.len() > MAX_COPY_OPS {
                continue;
            }
            let Some((destination, source)) = ranges(body, index) else {
                continue;
            };
            let (Base::Local(destination_cursor), Base::Local(source_cursor)) =
                (destination.start, source.start)
            else {
                continue;
            };
            let (Some(destination_slot), Some(source_slot)) = (
                slots.get(destination.reference),
                slots.get(source.reference),
            ) else {
                continue;
            };
            let Scalar::Constant(limit) = destination_slot.extent else {
                continue;
            };
            if destination_slot.region == source_slot.region
                || destination.path != [destination_slot.field]
                || source.path != [source_slot.field]
            {
                continue;
            }
            if proved(
                program,
                &inventory,
                &operations[..offset],
                &operations[offset + 1..],
                CursorPair {
                    source: source_cursor,
                    destination: destination_cursor,
                    amount,
                    source_extent: source_slot.extent,
                    destination_extent: limit,
                },
            ) {
                // The receiving room bounds the copy. The sole write to the
                // requested amount is the proved minimum, so its initial
                // interval can tighten the maximum without a runtime branch.
                let maximum = inventory
                    .declarations
                    .get(amount.as_str())
                    .and_then(|initial| inventory.interval(program, initial, &mut 32))
                    .map_or(limit, |(_, high)| limit.min(high));
                result.insert(body.as_ptr(), maximum);
            }
        }
    }
    result
}
