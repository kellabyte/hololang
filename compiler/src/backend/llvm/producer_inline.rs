//! Cost-bounded inlining of checked terminal destination producer pairs.
//!
//! Select the whole pair so removing the parent cannot move the native
//! call to the child. Checked shape and finite expansion costs select LLVM
//! attributes without changing the construction bodies or private ABI.

use std::collections::HashMap;

use crate::ir::{Argument, Build, Expr, Op, Producer, Program};
use crate::layout::{LayoutMode, LayoutPlan};

use super::{copy, producer};

/// Total checked-node estimate per pair.
const PAIR_BUDGET: usize = 64;
/// Approximate lowering allowance for the guarded byte-copy dispatcher.
const COPY_UNITS: usize = 16;
/// Expanded cost across all selected syntactic append sites.
const MODULE_BUDGET: usize = 256;

/// LLVM inlining preference for one private producer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Inline {
    /// Leave the optimizer's ordinary cost model in charge.
    Default,
    /// Preserve the guarded-copy inlining hint.
    Hint,
    /// Inline both checked definitions in a bounded pair.
    Pair,
}

impl Inline {
    /// Return LLVM attribute text with its leading separator.
    pub(super) fn attribute(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Hint => " inlinehint",
            Self::Pair => " alwaysinline",
        }
    }
}

/// Select terminal parent/leaf pairs with exclusive checked uses and small bodies.
///
/// A selected root is reached only by appends. Its leaf is used only by the
/// root. This bounds every use affected by the definition-level attributes.
pub(super) fn plan(
    program: &Program,
    layout: &LayoutPlan,
    target: &str,
    hints: &[bool],
) -> Vec<Inline> {
    let mut attributes: Vec<_> = hints
        .iter()
        .map(|hint| if *hint { Inline::Hint } else { Inline::Default })
        .collect();
    // Scope the policy to the measured Derived64 layout on targets with
    // checked guarded copies.
    if layout.mode() != LayoutMode::Derived64 || !hints.iter().any(|hint| *hint) {
        return attributes;
    }
    let indices: HashMap<_, _> = program
        .producers
        .iter()
        .enumerate()
        .map(|(index, definition)| (definition.name.as_str(), index))
        .collect();
    let entry = destination_uses(&program.body, &indices);
    let mut nested = vec![0usize; program.producers.len()];
    // Count every constructor edge, including forwards and repeated calls.
    for definition in &program.producers {
        for op in &definition.body {
            if let Build::Record { producer, .. } | Build::Forward { producer, .. } = op {
                nested[indices[producer.as_str()]] += 1;
            }
        }
    }
    let mut module_budget = MODULE_BUDGET;
    // Source order breaks ties when several pairs fit individually.
    for (index, parent) in program.producers.iter().enumerate() {
        if !hints[index] || nested[index] != 0 || entry[index] == 0 || !eligible(parent, layout) {
            continue;
        }
        let Some((Build::Record { producer, args, .. }, prefix)) = parent.body.split_last() else {
            continue;
        };
        let child_index = indices[producer.as_str()];
        let child = &program.producers[child_index];
        // The leaf can neither expand other call sites nor require a third
        // definition. Its typed result already uses the parent's encoding.
        if nested[child_index] != 1
            || entry[child_index] != 0
            || child.error != parent.error
            || !eligible(child, layout)
        {
            continue;
        }
        let Some(cost) = pair_cost(prefix, args, &child.body, target) else {
            continue;
        };
        let Some(remaining) = cost
            .checked_mul(entry[index])
            .and_then(|expanded| module_budget.checked_sub(expanded))
        else {
            continue;
        };
        module_budget = remaining;
        attributes[index] = Inline::Pair;
        attributes[child_index] = Inline::Pair;
    }
    attributes
}

/// Check that a typed producer can return its end without filling a size header.
fn eligible(definition: &Producer, layout: &LayoutPlan) -> bool {
    producer::placement(definition, layout) == producer::Placement::Deferred
        && layout.record(&definition.record).header_bytes == 0
}

/// Count destination calls across lexical blocks without expanding loops.
fn destination_uses(body: &[Op], indices: &HashMap<&str, usize>) -> Vec<usize> {
    let mut uses = vec![0; indices.len()];
    let mut pending = vec![body];
    // Visit each block once. Dynamic iterations repeat one native site.
    while let Some(block) = pending.pop() {
        for operation in block {
            match operation {
                Op::Append { producer, .. } | Op::AppendHandled { producer, .. } => {
                    uses[indices[producer.as_str()]] += 1;
                }
                Op::ConstructHandled { producer, arms, .. } => {
                    // The subject is one entry use. Every handler may also
                    // contain independent attempts, including failure retries.
                    uses[indices[producer.as_str()]] += 1;
                    pending.extend(arms.iter().map(|arm| arm.body.as_slice()));
                }
                Op::HandleResult { arms, .. } | Op::HandleSavedResult { arms, .. } => {
                    pending.extend(arms.iter().map(|arm| arm.body.as_slice()));
                }
                Op::Refill(refill) => {
                    pending.push(&refill.data_body);
                    pending.push(&refill.other_body);
                }
                // Count destination calls in both wrapped refill outcomes.
                Op::TcpIo { operation, .. } => {
                    pending.push(std::slice::from_ref(operation.as_ref()))
                }
                Op::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    pending.push(then_body);
                    pending.push(else_body);
                }
                Op::Region {
                    body, on_failure, ..
                } => {
                    pending.push(body);
                    pending.push(on_failure);
                }
                Op::Batch { body, .. }
                | Op::TcpScope { body, .. }
                | Op::Repeat { body, .. }
                | Op::ForEach { body, .. }
                | Op::Loop(body)
                | Op::Scope(body) => pending.push(body),
                Op::Function(function) => pending.push(&function.body),
                _ => {}
            }
        }
    }
    uses
}

/// Estimate pair expansion, including the scalar expressions in terminal actuals.
fn pair_cost(prefix: &[Build], args: &[Argument], child: &[Build], target: &str) -> Option<usize> {
    // One final guarded copy gets a fixed allowance. Other operations must
    // fit the bounded scalar grammar counted below.
    let (Build::Bytes { len, value, .. }, child_prefix) = child.split_last()? else {
        return None;
    };
    if !matches!(
        copy::choose(target, len, value),
        copy::Choice::GuardedInline { .. }
    ) {
        return None;
    }
    let mut remaining = (PAIR_BUDGET - COPY_UNITS - 2).checked_sub(args.len())?;
    for op in prefix.iter().chain(child_prefix) {
        remaining = remaining.checked_sub(1)?;
        match op {
            Build::U64 { value, .. } => charge(value, &mut remaining)?,
            Build::Reject {
                condition, payload, ..
            } => {
                charge(&condition.left, &mut remaining)?;
                charge(&condition.right, &mut remaining)?;
                if let Some(payload) = payload {
                    charge(payload, &mut remaining)?;
                }
            }
            // Nested calls, other copies and numeric rejects retain their hints.
            _ => return None,
        }
    }
    for arg in args {
        match arg {
            Argument::Scalar(expr) => charge(expr, &mut remaining)?,
            Argument::ByteView { ranges, .. } => {
                for range in ranges {
                    remaining = remaining.checked_sub(1)?;
                    if let Some(offset) = &range.offset {
                        charge(offset, &mut remaining)?;
                    }
                    charge(&range.length, &mut remaining)?;
                }
            }
            Argument::Bytes(_) | Argument::Received(_) => {}
        }
    }
    charge(len, &mut remaining)?;
    charge(value, &mut remaining)?;
    Some(PAIR_BUDGET - remaining)
}

/// Charge supported scalar nodes without host recursion or runtime evaluation.
fn charge(expr: &Expr, remaining: &mut usize) -> Option<()> {
    let mut pending = vec![expr];
    // Every iteration consumes one unit. Other expression forms retain
    // LLVM's ordinary model, including helper applications with their own work.
    while let Some(expr) = pending.pop() {
        *remaining = remaining.checked_sub(1)?;
        match expr {
            Expr::Binary { op, left, right } if !op.is_checked() => {
                pending.push(left);
                pending.push(right);
            }
            Expr::InputByte { index, .. } => pending.push(index),
            Expr::Const(_) | Expr::Param(_) | Expr::InputLen(_) | Expr::Index => {}
            _ => return None,
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use crate::{frontend::parse_experimental, layout::LayoutMode};

    /// A checked nested byte-bearing source with a reachable rejection edge.
    const PAIR: &str = r#"
error FrameError { Truncated(required u64) }
struct Input { data Bytes }
struct Frame { data Bytes }
struct Received { frame Frame }

func input returns Input { return Input { data: core.bytes(2, 1) } }
func decode arg wire Bytes as readonly reference returns Result<Frame, FrameError> {
    if core.len(wire) < 2 { return error FrameError.Truncated(2) }
    return ok Frame:
        data = core.bytes(1, wire[core.add_wrapping(1, core.index())])
}
func receive arg wire Bytes as readonly reference returns Result<Received, FrameError> {
    return ok Received:
        frame = try decode(wire)
}

func main returns u64 {
    let input_memory = core.region(256) else { return 0 }
    let inputs = core.batch(input_memory, Input)
    let ready = core.append(inputs, input())
    core.assert(ready == 0)
    let wire = core.borrow_readonly(inputs, 0)
    let output_memory = core.region(256) else { return 0 }
    let rows = core.batch(output_memory, Received)
"#;

    /// Check real source lowering before counting definition attributes.
    fn attributes(count: usize, shared_leaf: bool, triple: &str, mode: LayoutMode) -> usize {
        let mut source = PAIR.to_owned();
        // Static append sites affect expansion even when repeated at runtime.
        for i in 0..count {
            source.push_str(&format!(
                "    let row{i} = core.append(rows, receive(wire.data))\n        on ok then 0\n        on error then 1\n"
            ));
        }
        // A standalone leaf call also uses its private body. The pair must
        // retain ordinary hints in this shape.
        if shared_leaf {
            source.push_str(
                r#"
    let other_memory = core.region(256) else { return 0 }
    let other = core.batch(other_memory, Frame)
    let leaf = core.append(other, decode(wire.data))
        on ok then 0
        on error then 1
"#,
            );
        }
        source.push_str("    return 0\n}\n");
        let input = parse_experimental(&source)
            .unwrap_or_else(|error| panic!("{}", error.render("inline.holo", &source)));
        let checked = input.check().unwrap();
        super::super::emit_with_layout(&checked, triple, mode)
            .unwrap()
            .matches(" alwaysinline {")
            .count()
    }

    /// Only eligible whole pairs get forced inline. Shared uses, expansion
    /// budget, target, and control layouts independently retain ordinary costing.
    #[test]
    fn pair_selection_is_atomic_and_bounded() {
        let mac = "arm64-apple-macosx11.0.0";
        let linux = "aarch64-unknown-linux-gnu";
        let derived = LayoutMode::Derived64;
        assert_eq!(attributes(2, false, mac, derived), 2);
        assert_eq!(attributes(2, false, mac, LayoutMode::Compact32), 0);
        assert_eq!(attributes(16, false, mac, derived), 0);
        assert_eq!(attributes(2, true, mac, derived), 0);
        assert_eq!(attributes(2, false, mac, LayoutMode::Wide), 0);
        assert_eq!(attributes(2, false, linux, derived), 0);
    }
}
