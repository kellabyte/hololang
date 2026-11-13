//! Plan bounded rejection-tag specializations for terminal fixed producers.
//!
//! A specialization keeps the checked constructor ABI and storage plan. It
//! translates only named rejection constants into the enclosing range. This
//! lets a fixed parent return its terminal child's pair without inspecting it.

use std::collections::{HashMap, VecDeque};

use crate::ir::{Build, Op, Program};
use crate::layout::LayoutPlan;

use super::producer;

/// Cap extra LLVM bodies per producer. A missed specialization keeps checked
/// failure-edge translation in the caller, including within a cloned caller.
const MAX_LIFTS: usize = 4;

/// One checked fixed tail call and its local child-to-parent ordinal shift.
#[derive(Clone, Copy)]
struct Tail {
    /// Checked child producer index.
    child: usize,
    /// Zero-based start of the child's range in the parent's family.
    shift: u8,
}

/// Compiler-only table for bounded copies of fixed producer definitions.
pub(super) struct ProducerLifts {
    /// Optional fixed terminal edge per original producer index.
    tails: Vec<Option<Tail>>,
    /// Nonzero checked shifts, stored in deterministic discovery order.
    variants: Vec<Vec<u8>>,
}

impl ProducerLifts {
    /// Resolve checked tail edges, then compose and deduplicate their shifts.
    /// Each original or admitted clone contributes at most one further request.
    pub(super) fn new(program: &Program, layout: &LayoutPlan) -> Self {
        let indices: HashMap<_, _> = program
            .producers
            .iter()
            .enumerate()
            .map(|(index, producer)| (producer.name.as_str(), index))
            .collect();
        let families: HashMap<_, _> = program
            .body
            .iter()
            .filter_map(|op| match op {
                Op::ErrorFamily(family) => Some((family.name.as_str(), family)),
                _ => None,
            })
            .collect();
        let tails: Vec<_> = program
            .producers
            .iter()
            .map(|parent| {
                // A fixed tail needs no marker update or field work after the call.
                producer::fixed_size(parent, layout)?;
                let Some(Build::Record { producer, .. }) = parent.body.last() else {
                    return None;
                };
                let child = indices[producer.as_str()];
                let child_def = &program.producers[child];
                producer::fixed_size(child_def, layout)?;
                let parent_family = parent.error.as_deref().expect("fixed typed parent");
                let child_family = child_def.error.as_deref().expect("fixed typed child");
                let shift = families[parent_family]
                    .try_error_offset(child_family)
                    .expect("checked nominal child");
                Some(Tail { child, shift })
            })
            .collect();
        let mut variants = vec![Vec::new(); tails.len()];
        let mut pending: VecDeque<_> = (0..tails.len()).map(|index| (index, 0)).collect();
        // The checker has bounded all family ranges and rejected producer cycles.
        // Each admitted pair is queued once and emits at most one extra body.
        while let Some((index, lift)) = pending.pop_front() {
            let Some(tail) = tails[index] else {
                continue;
            };
            let shift = compose(lift, tail.shift);
            let known = &mut variants[tail.child];
            // Zero uses the original definition. Stop expanding when the
            // per-producer budget is exhausted, leaving its callers a fallback.
            if shift != 0 && known.len() < MAX_LIFTS && !known.contains(&shift) {
                known.push(shift);
                pending.push_back((tail.child, shift));
            }
        }
        Self { tails, variants }
    }

    /// List the requested nonzero variants for one checked producer.
    pub(super) fn variants(&self, index: usize) -> &[u8] {
        &self.variants[index]
    }

    /// Whether the terminal child can return directly in this encoding.
    /// Reserved storage statuses and payloads agree in both definitions.
    pub(super) fn forwards_tail(&self, index: usize, lift: u8) -> bool {
        self.tails[index].is_some_and(|tail| {
            let shift = compose(lift, tail.shift);
            shift == 0 || self.variants[tail.child].contains(&shift)
        })
    }
}

/// Compose two contiguous inclusion ranges checked in the nominal graph.
/// The outer family has at most 255 leaves, so its zero-based start fits u8.
pub(super) fn compose(lift: u8, shift: u8) -> u8 {
    lift.checked_add(shift).expect("checked error range")
}

/// Name the unchanged public-to-module producer or one internal specialization.
pub(super) fn symbol(index: usize, lift: u8) -> String {
    // Keep the original symbol for each ordinary append or zero-offset call.
    if lift == 0 {
        format!("@holo.producer.{index}")
    } else {
        format!("@holo.producer.{index}.lift.{lift}")
    }
}
