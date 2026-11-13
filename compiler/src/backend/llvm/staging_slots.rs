//! Lexical proof and preheader capture for the stable staging-slot experiment.
//!
//! A checked reference carries an acquisition identity only when its lexical
//! name is never rebound. Distinct simultaneously live regions have separate
//! backing. Published record shape is frozen even during writable payload use.
//! The trial reuses one pair of descriptors in a counted loop and emits no
//! pointer alias attributes, native proof state, or extra backing.

use std::collections::HashSet;

use crate::ir::Op;

use super::{
    context::{Binding, Lowering},
    input::Input,
    staging_copy::{Range, ranges},
};

/// Compile-time state for entry loops using the opt-in policy.
pub(super) struct StableStaging {
    /// All potentially reassigned reference names, including shadowed names.
    reassigned: HashSet<String>,
    /// The one field pair whose preheader values dominate the current loop.
    active: Option<Pair>,
}

impl StableStaging {
    /// Find possible rebindings once over checked entry IR.
    /// Excluding names conservatively avoids a reference dataflow join.
    pub(super) fn new(body: &[Op]) -> Self {
        let mut reassigned = HashSet::new();
        visit(body, &mut |operation| {
            // Ordinary calls cannot rebind a caller's reference slot.
            if let Op::AssignRef { name, .. } = operation {
                reassigned.insert(name.clone());
            }
        });
        Self {
            reassigned,
            active: None,
        }
    }

    /// Whether an initialized binding may keep its acquisition fact.
    pub(super) fn keeps_origin(&self, name: &str) -> bool {
        !self.reassigned.contains(name)
    }
}

/// Two distinct checked fields selected at the enclosing loop's preheader.
struct Pair {
    /// Writable field that receives the bytes.
    destination: Field,
    /// Live field that provides initialized source bytes.
    source: Field,
}

/// A captured byte descriptor tied to a native lexical reference slot.
struct Field {
    /// The compiler-assigned reference slot, including its shadowing identity.
    slot: String,
    /// Checked path to this particular field within its record.
    path: Vec<String>,
    /// Contents and length already emitted at the dominating preheader.
    descriptor: Descriptor,
}

/// Invariant LLVM operands for a frozen byte field.
#[derive(Clone)]
pub(super) struct Descriptor {
    /// Address of the first payload byte.
    pub(super) contents: String,
    /// Unsigned initialized extent.
    pub(super) extent: String,
}

impl Field {
    /// Read the checked field's immutable metadata and keep its lexical identity.
    /// This reads only published shape, without accessing or modifying payload.
    fn capture(lowering: &mut Lowering<'_>, range: &Range<'_>, slot: String) -> Self {
        let Input::Bytes {
            contents,
            len: extent,
        } = lowering.capture_byte_field(range.reference, range.path)
        else {
            unreachable!("checked byte field must lower to a byte descriptor")
        };
        Self {
            slot,
            path: range.path.to_vec(),
            descriptor: Descriptor { contents, extent },
        }
    }

    /// Confirm the copy site still names the exact field, even after shadowing.
    fn matches(&self, lowering: &Lowering<'_>, range: &Range<'_>) -> bool {
        matches!(lowering.binding(range.reference), Binding::Reference { slot, .. } if slot == self.slot)
            && range.path == self.path
    }
}

/// Look up cached operands only when both sites use the exact dominating pair.
/// The clones contain compiler strings, not target storage.
pub(super) fn lookup(
    lowering: &Lowering<'_>,
    destination: &Range<'_>,
    source: &Range<'_>,
) -> Option<(Descriptor, Descriptor)> {
    let pair = lowering.stable_staging.as_ref()?.active.as_ref()?;
    (pair.destination.matches(lowering, destination) && pair.source.matches(lowering, source)).then(
        || {
            (
                pair.destination.descriptor.clone(),
                pair.source.descriptor.clone(),
            )
        },
    )
}

/// Hoist one eligible descriptor pair before an enclosing counted loop.
///
/// The already evaluated count dominates the preheader. Zero trips bypass
/// even the metadata loads. A descendant may use the pair across published
/// payload writes, appends, refill, or opaque observation. None change frozen
/// shape and the checker prevents reclaiming either live reference's backing.
/// The uncached emitter is used when no pair can be proved.
pub(super) fn try_repeat(
    lowering: &mut Lowering<'_>,
    count: &str,
    index: &str,
    body: &[Op],
) -> bool {
    // One active pair bounds register pressure and avoids reloading it in descendants.
    if lowering
        .stable_staging
        .as_ref()
        .is_none_or(|trial| trial.active.is_some())
    {
        return false;
    }
    let Some((destination, source, destination_slot, source_slot)) = select(lowering, body) else {
        return false;
    };
    let preheader = lowering.f.fresh_label();
    let done = lowering.f.fresh_label();
    let nonempty = lowering.f.value(format!("icmp ne i64 {count}, 0"));
    lowering.f.line(format!(
        "br i1 {nonempty}, label %{preheader}, label %{done}"
    ));
    lowering.f.label(&preheader);
    lowering.f.line("; Holo stable staging slots");
    let pair = Pair {
        destination: Field::capture(lowering, &destination, destination_slot),
        source: Field::capture(lowering, &source, source_slot),
    };
    lowering.stable_staging.as_mut().unwrap().active = Some(pair);
    lowering.repeat_counted(count, index, body);
    lowering.stable_staging.as_mut().unwrap().active = None;
    lowering.f.line(format!("br label %{done}"));
    lowering.f.label(&done);
    true
}

/// Select the first eligible nested copy using two fields already in scope.
///
/// Reject names introduced anywhere below the preheader. This conservative
/// rule keeps nested scopes and cursor iteration identities out of the proof.
/// A copied alias can qualify when its own slot and origin were established
/// before the outer loop. No fact follows from read or write permission.
fn select<'a>(
    lowering: &Lowering<'_>,
    body: &'a [Op],
) -> Option<(Range<'a>, Range<'a>, String, String)> {
    let mut introduced = HashSet::new();
    let mut candidates = Vec::new();
    visit(body, &mut |operation| {
        match operation {
            Op::Borrow { name, .. }
            | Op::Alias { name, .. }
            | Op::ForEach {
                reference: name, ..
            } => {
                introduced.insert(name.as_str());
            }
            Op::ConstructHandled { arms, .. } => {
                // A success binding can shadow a reference from the preheader.
                introduced.extend(arms.iter().filter_map(|arm| arm.reference.as_deref()));
            }
            Op::Repeat { index, body, .. } => {
                // Collect only eligible loop shapes, leaving other operations untouched.
                if let Some(pair) = ranges(body, index) {
                    candidates.push(pair);
                }
            }
            _ => {}
        }
    });
    candidates.into_iter().find_map(|(destination, source)| {
        if introduced.contains(destination.reference) || introduced.contains(source.reference) {
            return None;
        }
        let Binding::Reference {
            slot: destination_slot,
            region: Some(destination_region),
            ..
        } = lowering.find_binding(destination.reference)?
        else {
            return None;
        };
        let Binding::Reference {
            slot: source_slot,
            region: Some(source_region),
            ..
        } = lowering.find_binding(source.reference)?
        else {
            return None;
        };
        // Separate successful region acquisitions cannot share a live byte.
        (destination_region != source_region).then_some((
            destination,
            source,
            destination_slot,
            source_slot,
        ))
    })
}

/// Visit lexical entry operations in deterministic source order.
///
/// Both branches and any nested owner body are scanned conservatively, even
/// when unreachable. Functions have private reference slots and stay outside
/// the entry proof. This traverses compiler IR only.
fn visit<'a>(body: &'a [Op], visitor: &mut impl FnMut(&'a Op)) {
    for operation in body {
        visitor(operation);
        match operation {
            Op::Region {
                body, on_failure, ..
            } => {
                visit(body, visitor);
                visit(on_failure, visitor);
            }
            Op::Refill(refill) => {
                visit(&refill.data_body, visitor);
                visit(&refill.other_body, visitor);
            }
            // A lexical stream borrow can wrap a refill with nested scopes.
            Op::TcpIo { operation, .. } => visit(std::slice::from_ref(operation.as_ref()), visitor),
            Op::ConstructHandled { arms, .. } => {
                // Assignments in every handler must inform stable-slot eligibility.
                for arm in arms {
                    visit(&arm.body, visitor);
                }
            }
            Op::If {
                then_body,
                else_body,
                ..
            } => {
                visit(then_body, visitor);
                visit(else_body, visitor);
            }
            Op::Loop(body)
            | Op::Batch { body, .. }
            | Op::TcpScope { body, .. }
            | Op::Scope(body)
            | Op::Repeat { body, .. }
            | Op::ForEach { body, .. } => visit(body, visitor),
            // These operations contain no nested entry blocks.
            Op::Function(_)
            | Op::ErrorFamily(_)
            | Op::WaitInput { .. }
            | Op::WaitOutput { .. }
            | Op::NextSession { .. }
            | Op::TcpClose { .. }
            | Op::Send { .. }
            | Op::Let { .. }
            | Op::TryLet { .. }
            | Op::Set { .. }
            | Op::Append { .. }
            | Op::AppendHandled { .. }
            | Op::Borrow { .. }
            | Op::Alias { .. }
            | Op::AssignRef { .. }
            | Op::Write { .. }
            | Op::WriteByte { .. }
            | Op::Reset { .. }
            | Op::Assert(_)
            | Op::Print(_)
            | Op::PrintText(_)
            | Op::Return(_)
            | Op::ReturnResult(_)
            | Op::ExitZero
            | Op::OpaqueRegion(_) => {}
        }
    }
}
