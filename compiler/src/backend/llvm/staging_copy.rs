//! Opt-in lowering for checked copies between initialized byte fields.
//!
//! Only a single forward element assignment with invariant base offsets can
//! enter the experiment. The fast path proves both bounds and disjointness
//! before touching payload memory. The original loop handles overlap, wrapping,
//! and late failure with its original read, write, and trap order.

use crate::ir::{Binary, Compare, Condition, Expr, Op};

use super::{context::Lowering, copy, input::Input, storage};

/// Maximum transfer left to the original scalar loop before range proof.
///
/// A fixed-cost bounds and disjointness proof can outweigh four byte stores.
/// This experimental code-generation threshold is independent of syntax.
const SCALAR_MAX: u64 = 4;

/// One matched field projection and its loop-invariant starting index.
pub(super) struct Range<'a> {
    /// Live record binding from the original checked operation.
    pub(super) reference: &'a str,
    /// Nested field path ending in a published byte payload.
    pub(super) path: &'a [String],
    /// Starting scalar operand, or literal zero for an unshifted index.
    pub(super) start: Base<'a>,
}

/// A copied scalar leaf which cannot observe the store's payload or effects.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Base<'a> {
    /// A literal unsigned index offset.
    Constant(u64),
    /// An already established lexical scalar before entering this loop.
    Local(&'a str),
}

/// A normalized pure capture admitted for source and destination indices.
#[derive(Clone, Copy)]
enum Scalar<'a> {
    /// A literal or copied local unaffected by every loop iteration.
    Invariant(Base<'a>),
    /// The loop index plus the given invariant starting offset.
    Index(Base<'a>),
}

/// One captured payload location, with its original extent and starting index.
pub(super) struct CapturedRange {
    /// First payload byte, derived from initialized frozen shape metadata.
    contents: String,
    /// Full unsigned payload extent in the active layout.
    extent: String,
    /// Captured unsigned starting offset from the first payload byte.
    start: String,
}

impl Range<'_> {
    /// Capture stable shape and scalar inputs once, without accessing payload.
    pub(super) fn capture(&self, lowering: &mut Lowering<'_>) -> CapturedRange {
        let Input::Bytes {
            contents,
            len: extent,
        } = lowering.capture_byte_field(self.reference, self.path)
        else {
            unreachable!("a checked byte field yields a byte descriptor")
        };
        self.with_descriptor(lowering, contents, extent)
    }

    /// Evaluate only the scalar offset at the current transfer.
    /// The supplied descriptor belongs to this exact reference slot and path.
    fn with_descriptor(
        &self,
        lowering: &mut Lowering<'_>,
        contents: String,
        extent: String,
    ) -> CapturedRange {
        // Resolve the scalar in the enclosing scope, prior to the loop's
        // pure capture statements. Literals require no native operation.
        let start = match self.start {
            Base::Constant(value) => value.to_string(),
            Base::Local(name) => lowering.expression(&Expr::Local(name.to_owned())),
        };
        CapturedRange {
            contents,
            extent,
            start,
        }
    }
}

impl CapturedRange {
    /// Prove an entire positive interval with unsigned, nontrapping checks.
    ///
    /// Starts outside the extent cannot use the result of the wrapping
    /// subtraction. An accepted interval proves all original index additions.
    fn fits(&self, lowering: &mut Lowering<'_>, count: &str) -> String {
        let starts_inside = lowering
            .f
            .value(format!("icmp ule i64 {}, {}", self.start, self.extent));
        let remaining = lowering
            .f
            .value(format!("sub i64 {}, {}", self.extent, self.start));
        let stays_inside = lowering
            .f
            .value(format!("icmp ule i64 {count}, {remaining}"));
        lowering
            .f
            .value(format!("and i1 {starts_inside}, {stays_inside}"))
    }

    /// Form the interval's first byte after the complete bounds proof.
    pub(super) fn address(&self, lowering: &mut Lowering<'_>) -> String {
        storage::advance(&mut lowering.f, &self.contents, &self.start)
    }
}

/// Identify one inert capture, or the loop index plus an inert offset.
///
/// Prefix lets are visited in order so only earlier captures can be expanded.
/// Field reads, calls, and other eager expressions reject the candidate.
fn scalar<'a>(
    expression: &'a Expr,
    loop_index: &str,
    captures: &[(&str, Scalar<'a>)],
) -> Option<Scalar<'a>> {
    // Both constant and lexical inputs are nontrapping copied operands.
    match expression {
        Expr::Const(value) => Some(Scalar::Invariant(Base::Constant(*value))),
        Expr::Local(name) if name == loop_index => Some(Scalar::Index(Base::Constant(0))),
        Expr::Local(name) => Some(
            captures
                .iter()
                .find_map(|(captured, value)| (*captured == name).then_some(*value))
                .unwrap_or(Scalar::Invariant(Base::Local(name))),
        ),
        Expr::Binary {
            op: Binary::AddWrapping,
            left,
            right,
        } => {
            // Commuting two nontrapping scalar leaves preserves the expression.
            match (
                scalar(left, loop_index, captures)?,
                scalar(right, loop_index, captures)?,
            ) {
                (Scalar::Index(Base::Constant(0)), Scalar::Invariant(base))
                | (Scalar::Invariant(base), Scalar::Index(Base::Constant(0))) => {
                    Some(Scalar::Index(base))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Match a forward byte store and its exact pure assignment scaffolding.
///
/// Parsed assignment captures index operands in scalar lets and checks the
/// destination before evaluating the source. The guard is redundant only
/// when it checks exactly the matched destination index and byte length.
/// Raw checked IR may express the same assignment without that prefix.
pub(super) fn ranges<'a>(body: &'a [Op], loop_index: &str) -> Option<(Range<'a>, Range<'a>)> {
    let (store, prefix) = body.split_last()?;
    let Op::WriteByte {
        reference: destination,
        path: destination_path,
        index: destination_index,
        value:
            Expr::Byte {
                reference: source,
                path: source_path,
                index: source_index,
            },
    } = store
    else {
        return None;
    };
    let mut captures = Vec::with_capacity(prefix.len());
    let mut checked_index = None;
    // Every omitted operation must be an inert capture or this one matching
    // generated guard. Reject any other work before considering the transfer.
    for operation in prefix {
        match operation {
            Op::Let { name, value } => {
                captures.push((name.as_str(), scalar(value, loop_index, &captures)?));
            }
            Op::Assert(Condition {
                op: Compare::Lt,
                left,
                right: Expr::BytesLen { reference, path },
            }) if checked_index.is_none()
                && reference == destination
                && path == destination_path =>
            {
                let Scalar::Index(base) = scalar(left, loop_index, &captures)? else {
                    return None;
                };
                checked_index = Some(base);
            }
            _ => return None,
        }
    }
    let Scalar::Index(destination_start) = scalar(destination_index, loop_index, &captures)? else {
        return None;
    };
    let Scalar::Index(source_start) = scalar(source_index, loop_index, &captures)? else {
        return None;
    };
    // A guard for a different destination could fail on a valid copy.
    if checked_index.is_some_and(|checked| checked != destination_start) {
        return None;
    }
    Some((
        Range {
            reference: destination,
            path: destination_path,
            start: destination_start,
        },
        Range {
            reference: source,
            path: source_path,
            start: source_start,
        },
    ))
}

/// Emit a checked fast path for one eligible staging loop, or leave emission untouched.
///
/// The selected policy admits a proved slot pair, or may permit the earlier
/// per-transfer guard. A declined site leaves count evaluation to the caller.
/// An admitted site evaluates its count once and retains the scalar loop on
/// any runtime proof failure, including the original trap and prefix writes.
pub(super) fn try_lower(
    lowering: &mut Lowering<'_>,
    count: &Expr,
    loop_index: &str,
    body: &[Op],
) -> bool {
    let Some(use_marker) = lowering.staging_copies else {
        return false;
    };
    let Some((destination, source)) = ranges(body, loop_index) else {
        return false;
    };
    let slots = super::staging_slots::lookup(lowering, &destination, &source);
    // The release candidate never emits guards or declarations for an unproved
    // copy. The caller evaluates its count and emits the ordinary scalar loop.
    if slots.is_none() && !lowering.guarded_staging_copies {
        return false;
    }
    use_marker.set(true);
    lowering.f.line("; Holo checked staging copy");
    let count = lowering.expression(count);
    // The outer proof admits this exact live pair. Dynamic offsets still need
    // checked subranges, even when the underlying fields are disjoint.
    if let Some((dst, src)) = slots {
        let destination = destination.with_descriptor(lowering, dst.contents, dst.extent);
        let source = source.with_descriptor(lowering, src.contents, src.extent);
        stable_copy(lowering, &count, &destination, &source, loop_index, body);
        return true;
    }
    let bounds = lowering.f.fresh_label();
    let addresses = lowering.f.fresh_label();
    let dispatch = lowering.f.fresh_label();
    let inline = lowering.f.fresh_label();
    let bulk = lowering.f.fresh_label();
    let scalar = lowering.f.fresh_label();
    let done = lowering.f.fresh_label();

    // Skip the fixed-cost proof on tiny transfers. For zero, the ordinary
    // loop skips its body and therefore both fields and starting offsets.
    let larger = lowering
        .f
        .value(format!("icmp ugt i64 {count}, {SCALAR_MAX}"));
    lowering
        .f
        .line(format!("br i1 {larger}, label %{bounds}, label %{scalar}"));
    lowering.f.label(&bounds);
    // Shape is frozen, reference slots and base scalars cannot be changed by
    // this body, and these observations read no potentially aliased payload.
    let destination = destination.capture(lowering);
    let source = source.capture(lowering);
    let destination_valid = destination.fits(lowering, &count);
    let source_valid = source.fits(lowering, &count);
    let valid = lowering
        .f
        .value(format!("and i1 {destination_valid}, {source_valid}"));
    // A failed range proof is not a trap. Replay the checked iterations, so
    // the first invalid access still happens after any valid prefix writes.
    lowering.f.line(format!(
        "br i1 {valid}, label %{addresses}, label %{scalar}"
    ));
    lowering.f.label(&addresses);
    let destination = destination.address(lowering);
    let source = source.address(lowering);
    let disjoint = disjoint(lowering, &destination, &source, &count);
    // No copy intrinsic can observe aliased source bytes. Keep the forward
    // loop for overlap rather than substituting snapshot or memmove semantics.
    lowering.f.line(format!(
        "br i1 {disjoint}, label %{dispatch}, label %{scalar}"
    ));

    lowering.f.label(&dispatch);
    let short = lowering
        .f
        .value(format!("icmp ule i64 {count}, {}", copy::LIMIT));
    lowering
        .f
        .line(format!("br i1 {short}, label %{inline}, label %{bulk}"));
    lowering.f.label(&bulk);
    lowering.f.line(format!(
        "call void @llvm.memcpy.p0.p0.i64(ptr align 1 {destination}, ptr align 1 {source}, i64 {count}, i1 false)"
    ));
    lowering.f.line(format!("br label %{done}"));
    lowering.f.label(&inline);
    copy::short_copy(&mut lowering.f, &destination, &source, &count, &done);

    lowering.f.label(&scalar);
    lowering.repeat_counted(&count, loop_index, body);
    lowering.f.line(format!("br label %{done}"));
    lowering.f.label(&done);
    true
}

/// Emit a bounded transfer using the active distinct allocation proof.
///
/// The offsets are inert copied scalars. The count is evaluated once. Whole
/// transfer validation precedes address formation, and any failed proof enters
/// the original loop so prefix writes and fatal check precedence are preserved.
fn stable_copy(
    lowering: &mut Lowering<'_>,
    count: &str,
    destination: &CapturedRange,
    source: &CapturedRange,
    loop_index: &str,
    body: &[Op],
) {
    let bounds = lowering.f.fresh_label();
    let transfer = lowering.f.fresh_label();
    let tiny = lowering.f.fresh_label();
    let larger = lowering.f.fresh_label();
    let inline = lowering.f.fresh_label();
    let bulk = lowering.f.fresh_label();
    let scalar = lowering.f.fresh_label();
    let done = lowering.f.fresh_label();
    // Empty transfers do not form a payload address, even with invalid offsets.
    let nonempty = lowering.f.value(format!("icmp ne i64 {count}, 0"));
    lowering
        .f
        .line(format!("br i1 {nonempty}, label %{bounds}, label %{done}"));
    lowering.f.label(&bounds);
    let destination_valid = destination.fits(lowering, count);
    let source_valid = source.fits(lowering, count);
    let valid = lowering
        .f
        .value(format!("and i1 {destination_valid}, {source_valid}"));
    lowering
        .f
        .line(format!("br i1 {valid}, label %{transfer}, label %{scalar}"));
    lowering.f.label(&transfer);
    let destination = destination.address(lowering);
    let source = source.address(lowering);
    // Test tiny fragments first so they avoid the larger-copy geometry chain.
    let is_tiny = lowering
        .f
        .value(format!("icmp ule i64 {count}, {SCALAR_MAX}"));
    lowering
        .f
        .line(format!("br i1 {is_tiny}, label %{tiny}, label %{larger}"));
    lowering.f.label(&tiny);
    copy::tiny_copy(&mut lowering.f, &destination, &source, count, &done);

    lowering.f.label(&larger);
    let short = lowering
        .f
        .value(format!("icmp ule i64 {count}, {}", copy::LIMIT));
    lowering
        .f
        .line(format!("br i1 {short}, label %{inline}, label %{bulk}"));
    lowering.f.label(&bulk);
    lowering.f.line(format!(
        "call void @llvm.memcpy.p0.p0.i64(ptr align 1 {destination}, ptr align 1 {source}, i64 {count}, i1 false)"
    ));
    lowering.f.line(format!("br label %{done}"));
    lowering.f.label(&inline);
    copy::short_copy(&mut lowering.f, &destination, &source, count, &done);

    lowering.f.label(&scalar);
    lowering.repeat_counted(count, loop_index, body);
    lowering.f.line(format!("br label %{done}"));
    lowering.f.label(&done);
}

/// Check separation of two equally sized nonempty valid address intervals.
///
/// The selected target uses ordinary flat 64-bit backing addresses. Subtract
/// the smaller address from the larger, avoiding an address-end addition that
/// could wrap. The generated pointers retain their original LLVM provenance.
fn disjoint(lowering: &mut Lowering<'_>, destination: &str, source: &str, count: &str) -> String {
    let destination = lowering
        .f
        .value(format!("ptrtoint ptr {destination} to i64"));
    let source = lowering.f.value(format!("ptrtoint ptr {source} to i64"));
    let ascending = lowering
        .f
        .value(format!("icmp ule i64 {source}, {destination}"));
    let after_source = lowering.f.value(format!("sub i64 {destination}, {source}"));
    let before_source = lowering.f.value(format!("sub i64 {source}, {destination}"));
    let distance = lowering.f.value(format!(
        "select i1 {ascending}, i64 {after_source}, i64 {before_source}"
    ));
    lowering
        .f
        .value(format!("icmp uge i64 {distance}, {count}"))
}
