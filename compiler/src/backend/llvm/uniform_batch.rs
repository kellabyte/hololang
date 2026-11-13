//! Compile-time extent proof for a captured prefix of single-byte-field records.
//!
//! The proof visits every append that can publish into one lexical batch. A
//! successful append commits a complete record of its proven extent. A rejection
//! changes no published shape, and reset restores the empty invariant. The trial
//! emits no metadata, shape guard, allocation, or cleanup at runtime.

use crate::ir::{Argument, Build, Expr, FieldKind, Op, Program};
use crate::layout::LayoutPlan;

/// Bound the work and recursion of one optional proof over checked operations.
const PROOF_BUDGET: usize = 256;

/// Frozen extent shared by all records published by this lexical owner.
#[derive(Clone, Copy)]
pub(super) struct Shape {
    /// Initialized length of the schema's sole byte field.
    pub(super) extent: u64,
    /// Positive physical record size, including the existing length metadata.
    pub(super) stride: u64,
}

/// Derive one uniform stride from the existing checked layout and append sites.
/// Returns no fact for an unknown producer extent, mixed extents, exhausted
/// budget, or batch without a syntactic append. No native value is evaluated.
pub(super) fn prove(
    program: &Program,
    layout: &LayoutPlan,
    batch: &str,
    record: &str,
    body: &[Op],
) -> Option<Shape> {
    let schema = program
        .records
        .iter()
        .find(|schema| schema.name == record)?;
    // One byte field makes its zero-payload layout the only stride overhead.
    if !matches!(schema.fields.as_slice(), [field] if field.kind == FieldKind::Bytes) {
        return None;
    }
    let mut proof = Proof {
        program,
        batch,
        extent: None,
        budget: PROOF_BUDGET,
    };
    proof.visit(body)?;
    let extent = proof.extent?;
    let stride = layout.record(record).minimum_size.checked_add(extent)?;
    Some(Shape { extent, stride })
}

/// Bounded compiler-only analysis of one active lexical batch owner.
struct Proof<'a> {
    /// Checked constructor definitions used by every target append.
    program: &'a Program,
    /// Lexical target name. A same-name nested owner hides it in its body.
    batch: &'a str,
    /// The common extent of all target append sites visited so far.
    extent: Option<u64>,
    /// Remaining operations and constructor steps allowed in this proof.
    budget: usize,
}

impl Proof<'_> {
    /// Check every potentially executed target append in a structured block.
    /// Recurse into both arms and loop bodies without depending on reachability,
    /// trip count, append status, or which side effects occur at runtime.
    fn visit(&mut self, operations: &[Op]) -> Option<()> {
        for (offset, operation) in operations.iter().enumerate() {
            self.budget = self.budget.checked_sub(1)?;
            match operation {
                Op::Append {
                    batch,
                    producer,
                    args,
                    ..
                }
                | Op::AppendHandled {
                    batch,
                    producer,
                    args,
                    ..
                } => {
                    // Other owners publish disjoint records and add no obligation.
                    if batch == self.batch {
                        self.append(producer, args, &operations[..offset])?;
                    }
                }
                Op::Batch { name, body, .. } => {
                    // A same-name owner shadows the target in its entire body.
                    if name != self.batch {
                        self.visit(body)?;
                    }
                }
                Op::Region {
                    body, on_failure, ..
                }
                | Op::If {
                    then_body: body,
                    else_body: on_failure,
                    ..
                } => {
                    self.visit(body)?;
                    self.visit(on_failure)?;
                }
                Op::Refill(refill) => {
                    self.visit(&refill.data_body)?;
                    self.visit(&refill.other_body)?;
                }
                // Refills inside a checked TCP borrow can still append to
                // this batch from either outcome scope.
                Op::TcpIo { operation, .. } => {
                    self.visit(std::slice::from_ref(operation.as_ref()))?;
                }
                Op::ConstructHandled { arms, .. } => {
                    // The scoped subject creates no batch element, but any
                    // handler can append to the independently owned batch.
                    for arm in arms {
                        self.visit(&arm.body)?;
                    }
                }
                Op::HandleResult { arms, .. } | Op::HandleSavedResult { arms, .. } => {
                    for arm in arms {
                        self.visit(&arm.body)?;
                    }
                }
                Op::ForEach { body, .. }
                | Op::Loop(body)
                | Op::Repeat { body, .. }
                | Op::TcpScope { body, .. }
                | Op::Scope(body) => self.visit(body)?,
                // Scalar calls have no batch authority. Checked writes and refill
                // only mutate contents, and reset/exit cannot create records.
                Op::Function(_)
                | Op::ErrorFamily(_)
                | Op::WaitInput { .. }
                | Op::WaitOutput { .. }
                | Op::NextSession { .. }
                | Op::TcpClose { .. }
                | Op::Send { .. }
                | Op::Let { .. }
                | Op::LetBool { .. }
                | Op::BindBytes { .. }
                | Op::BindResult { .. }
                | Op::TryLet { .. }
                | Op::TrySaved { .. }
                | Op::Set { .. }
                | Op::Borrow { .. }
                | Op::Alias { .. }
                | Op::AssignRef { .. }
                | Op::Write { .. }
                | Op::WriteByte { .. }
                | Op::WriteInputByte { .. }
                | Op::WriteViewByte { .. }
                | Op::MoveInputBytes { .. }
                | Op::Reset { .. }
                | Op::Assert(_)
                | Op::Print(_)
                | Op::PrintText(_)
                | Op::Return(_)
                | Op::ReturnBool(_)
                | Op::ReturnResult(_)
                | Op::ExitZero
                | Op::OpaqueRegion(_) => {}
            }
        }
        Some(())
    }

    /// Establish the sole field's exact extent for one checked append site.
    /// A literal or a formal receiving a directly captured literal qualifies.
    /// Rejection guards do not change the extent of a successful publication.
    fn append(&mut self, producer: &str, args: &[Argument], prefix: &[Op]) -> Option<()> {
        let producer = self
            .program
            .producers
            .iter()
            .find(|item| item.name == producer)?;
        let mut extent = None;
        for step in &producer.body {
            self.budget = self.budget.checked_sub(1)?;
            match step {
                Build::Bytes {
                    len: Expr::Const(value),
                    ..
                } => extent = Some(*value),
                Build::Bytes {
                    len: Expr::Param(index),
                    ..
                } => {
                    // A checked scalar actual can be inline or have the frontend's
                    // eager let capture. Neither form changes evaluation order.
                    let Argument::Scalar(actual) = args.get(*index)? else {
                        return None;
                    };
                    extent = Some(self.literal(actual, prefix)?);
                }
                Build::FailIf(_) | Build::Reject { .. } => {}
                // Other expressions or construction shapes retain generic lowering.
                _ => return None,
            }
        }
        let extent = extent?;
        // An unexecuted mixed site still disqualifies this lexical owner.
        if self.extent.is_some_and(|previous| previous != extent) {
            return None;
        }
        self.extent = Some(extent);
        Some(())
    }

    /// Find a literal inline actual or a fresh same-block scalar capture.
    /// Adjacent lets cannot reassign the value, including through their calls.
    /// Any other operation or computed initializer ends this local reasoning.
    fn literal(&mut self, actual: &Expr, prefix: &[Op]) -> Option<u64> {
        let capture = match actual {
            Expr::Const(value) => return Some(*value),
            Expr::Local(name) => name,
            _ => return None,
        };
        for operation in prefix.iter().rev() {
            self.budget = self.budget.checked_sub(1)?;
            // A branch, write, or owner scope could invalidate an earlier fact.
            let Op::Let { name, value } = operation else {
                return None;
            };
            if name == capture {
                return match value {
                    Expr::Const(value) => Some(*value),
                    _ => None,
                };
            }
        }
        None
    }
}
