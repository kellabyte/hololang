//! Target policy and bounded inline copies for checked byte initializers.
//!
//! This module recognizes a contiguous read from one captured byte input. The
//! caller owns ordered length evaluation and destination reservation. A short
//! range check allows fixed-size inline transfers or an ordinary LLVM bulk copy.
//! A failed proof keeps the checked initializer's original access and trap order.

use std::collections::HashMap;

use super::{
    context::{Lowering, Module},
    function::Function,
    input::Input,
    storage,
};
use crate::ir::{Binary, Build, Expr, Program};

/// Maximum byte count in the initial macOS arm64 code generation policy.
///
/// The bound limits each short path to two fixed chunks of up to 32 bytes or
/// three single-byte transfers.
/// Native comparisons determine whether the policy is profitable.
pub(super) const LIMIT: u64 = 64;

/// Fixed-size LLVM copy intrinsics allow the backend to choose instructions.
///
/// The length operand at every emitted call is a compiler constant. The inline
/// intrinsic guarantees that the operation introduces no external function call.
pub(super) const DECLARATIONS: &str = "\
declare void @llvm.memcpy.inline.p0.p0.i64(ptr, ptr, i64 immarg, i1 immarg)\n\
declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1 immarg)\n";

/// Backend lowering choice for one checked byte initializer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    /// The existing checked byte initializer for this target or expression.
    CheckedLoop(&'static str),
    /// Guard an eligible source subrange and inline a bounded short transfer.
    GuardedInline {
        /// Formal that holds the initialized source descriptor.
        input: usize,
        /// Compile-time byte offset from the beginning of the captured input.
        offset: u64,
    },
}

impl Choice {
    /// Return the compiler decision for source inspection and emitted IR comments.
    ///
    /// The message describes checked lowering. LLVM can further optimize the
    /// ordinary loop, so the explanation does not predict every native instruction.
    pub(super) fn explanation(self) -> String {
        // Each message distinguishes a guarded selection from an ordinary loop.
        match self {
            Self::CheckedLoop(reason) => format!("checked byte loop ({reason})"),
            Self::GuardedInline { input, offset } => format!(
                "guarded inline through {LIMIT} bytes (input {input}, offset {offset}); \
                 LLVM bulk copy for larger valid ranges; checked byte loop if the range proof fails"
            ),
        }
    }
}

/// Report whether the target uses the reviewed macOS arm64 copy policy.
///
/// Target matching operates on compiler text. A different architecture or
/// operating system uses the unchanged scalar paths.
pub(super) fn supports_target(target: &str) -> bool {
    let mut parts = target.split('-');
    matches!(parts.next(), Some("arm64" | "aarch64"))
        && parts.next() == Some("apple")
        && parts
            .next()
            .is_some_and(|os| os.starts_with("macosx") || os.starts_with("darwin"))
}

/// Select a lowering choice from the checked expression and native target triple.
///
/// Only a single input read at the comprehension index plus a nonzero constant
/// qualifies. Every other form can have different eager reads, arithmetic, or
/// traps, and uses the established lowering. This scopes the policy to skipping
/// a fixed prefix before copying an independently sized span. Copies that start
/// at byte zero keep their existing optimizer behavior after earlier regressions.
pub(super) fn choose(target: &str, length: &Expr, value: &Expr) -> Choice {
    // Match the tested CPU and OS explicitly. Other targets retain the checked loop.
    if !supports_target(target) {
        return Choice::CheckedLoop("inline copy policy is scoped to macOS arm64");
    }
    let Expr::InputByte { param, index } = value else {
        return Choice::CheckedLoop("initializer is not a direct input-byte read");
    };
    // Match exactly the index tree. In particular, an Apply could evaluate unused
    // actuals that trap, and arbitrary arithmetic may wrap or change byte order.
    let offset = match index.as_ref() {
        Expr::Index => return Choice::CheckedLoop("zero-offset copy uses the existing lowering"),
        Expr::Binary {
            op: Binary::AddWrapping,
            left,
            right,
        } => match (left.as_ref(), right.as_ref()) {
            (Expr::Index, Expr::Const(offset)) | (Expr::Const(offset), Expr::Index) => *offset,
            _ => return Choice::CheckedLoop("index is not a constant-offset byte position"),
        },
        _ => return Choice::CheckedLoop("index is not a constant-offset byte position"),
    };
    // A literal zero has the same scope as an unshifted index.
    if offset == 0 {
        return Choice::CheckedLoop("zero-offset copy uses the existing lowering");
    }
    // A matching full-input length keeps the previously measured whole-copy path.
    if matches!(length, Expr::InputLen(length_input) if length_input == param) {
        return Choice::CheckedLoop("captured whole-input length uses the existing lowering");
    }
    // Fixed empty or oversized lengths have no possible short path. Leaving their
    // original loop avoids an unreachable block in unoptimized native code.
    if matches!(length, Expr::Const(0)) || matches!(length, Expr::Const(length) if *length > LIMIT)
    {
        return Choice::CheckedLoop("constant length is outside the inline range");
    }
    Choice::GuardedInline {
        input: *param,
        offset,
    }
}

/// Find checked producers that can reach a guarded inline copy.
///
/// Each `true` index asks LLVM to consider exposing the site's size and source
/// extent at callers. LLVM still applies its inlining cost model. Ancestors are
/// visited once using the reverse graph so a deeply nested producer chain does
/// not recurse or repeat the analysis. Storage is compiler-local and proportional
/// to the checked producer definitions and calls.
pub(super) fn inline_candidates(program: &Program, target: &str) -> Vec<bool> {
    let mut candidates = vec![false; program.producers.len()];
    let mut pending = Vec::new();
    // Seed only producers owning an actual eligible initializer on this target.
    for (index, producer) in program.producers.iter().enumerate() {
        let direct = producer.body.iter().any(|operation| {
            matches!(operation, Build::Bytes { len, value, .. }
                if matches!(choose(target, len, value), Choice::GuardedInline { .. }))
        });
        if direct {
            candidates[index] = true;
            pending.push(index);
        }
    }
    // Programs outside the policy need no caller graph or hint changes.
    if pending.is_empty() {
        return candidates;
    }
    let indices: HashMap<_, _> = program
        .producers
        .iter()
        .enumerate()
        .map(|(index, producer)| (producer.name.as_str(), index))
        .collect();
    let mut callers = vec![Vec::new(); program.producers.len()];
    // The checked definitions already resolved every ordinary destination call.
    for (caller, producer) in program.producers.iter().enumerate() {
        for operation in &producer.body {
            if let Build::Record { producer, .. } | Build::Forward { producer, .. } = operation {
                callers[indices[producer.as_str()]].push(caller);
            }
        }
    }
    // Reachability is enough. A caller may own other fields or call multiple
    // producers, and LLVM retains the choice of leaving its calls out of line.
    while let Some(callee) = pending.pop() {
        for &caller in &callers[callee] {
            if !candidates[caller] {
                candidates[caller] = true;
                pending.push(caller);
            }
        }
    }
    candidates
}

/// Emit the short dispatch and leave the builder at the ordinary-loop label.
///
/// `input`/`extent` are the captured initialized source descriptor. `destination`
/// and `length` describe the already reserved unpublished payload. A checked
/// producer can only receive sources in separate live regions or a published
/// prefix preceding the private suffix. That range proof establishes disjointness
/// for each nonempty transfer. No pointer-wide alias attribute is emitted.
/// `done` rejoins the caller after an empty, inline, or bulk transfer. A failed
/// range proof enters the shared checked loop before either fast path touches the
/// payload. The helper owns only that uncommon path, keeping the normal producer
/// small enough for LLVM's caller-sensitive inlining and range simplification.
pub(super) fn dispatch(
    f: &mut Function,
    destination: &str,
    length: &str,
    input: &str,
    extent: &str,
    offset: u64,
    done: &str,
) {
    let bounds = f.fresh_label();
    let dispatch = f.fresh_label();
    let inline = f.fresh_label();
    let bulk = f.fresh_label();
    let fallback = f.fresh_label();
    // The original loop performs no input access for an empty output even when
    // its constant offset would be outside the input for a nonempty transfer.
    let nonempty = f.value(format!("icmp ne i64 {length}, 0"));
    f.line(format!("br i1 {nonempty}, label %{bounds}, label %{done}"));
    f.label(&bounds);
    // Both unsigned comparisons prove offset+length is in range without an
    // overflowing addition. No offset address is formed on the rejected edge.
    let starts_inside = f.value(format!("icmp ule i64 {offset}, {extent}"));
    let remaining = f.value(format!("sub i64 {extent}, {offset}"));
    let fits = f.value(format!("icmp ule i64 {length}, {remaining}"));
    let valid = f.value(format!("and i1 {starts_inside}, {fits}"));
    f.line(format!(
        "br i1 {valid}, label %{dispatch}, label %{fallback}"
    ));
    f.label(&dispatch);
    // A valid positive range proves address formation and original index addition.
    let source = storage::advance(f, input, &offset.to_string());
    let short = f.value(format!("icmp ule i64 {length}, {LIMIT}"));
    f.line(format!("br i1 {short}, label %{inline}, label %{bulk}"));
    f.label(&bulk);
    f.line(format!(
        "call void @llvm.memcpy.p0.p0.i64(ptr align 1 {destination}, ptr align 1 {source}, i64 {length}, i1 false)"
    ));
    f.line(format!("br label %{done}"));
    f.label(&inline);
    short_copy(f, destination, &source, length, done);
    f.label(&fallback);
    f.line(format!(
        "call void @holo.copy.checked(ptr {destination}, ptr {input}, i64 {extent}, i64 {length}, i64 {offset})"
    ));
    f.line(format!("br label %{done}"));
    f.label(done);
}

/// Generate the shared original checked loop for a rejected contiguous range.
///
/// The helper receives the same destination, length, input descriptor and offset
/// as the original byte
/// initializer. It uses that emitter's wrapping index, per-byte check and store
/// sequence. A late invalid index still traps after the valid preceding writes.
/// The cold, non-inlined function avoids duplicating this failure path in the
/// surrounding producer and introduces no region allocation or foreign calls.
pub(super) fn checked_loop(module: &Module<'_>) -> String {
    let mut lowering = Lowering::new(
        module,
        "define internal void @holo.copy.checked(ptr %dst, ptr %input, i64 %extent, i64 %length, i64 %offset) cold noinline"
            .to_owned(),
        vec![
            Input::Bytes {
                contents: "%input".to_owned(),
                len: "%extent".to_owned(),
            },
            Input::Scalar("%offset".to_owned()),
        ],
    );
    // Both original matcher orders have the same pure wrapping addition.
    let value = Expr::InputByte {
        param: 0,
        index: Box::new(Expr::binary(
            Binary::AddWrapping,
            Expr::Index,
            Expr::Param(1),
        )),
    };
    lowering.initialize_bytes("%dst", "%length", &value);
    lowering.f.line("ret void");
    lowering.f.finish()
}

/// Emit the aligned-1 copy of one fixed-size chunk within an established range.
///
/// `offset` is relative to both source and destination. The caller's length
/// branches keep every chunk inside both ranges. The fixed intrinsic preserves
/// the inline choice and introduces no runtime payload allocation or helper call.
pub(super) fn chunk(f: &mut Function, destination: &str, source: &str, offset: &str, width: u64) {
    let from = storage::advance(f, source, offset);
    let to = storage::advance(f, destination, offset);
    f.line(format!(
        "call void @llvm.memcpy.inline.p0.p0.i64(ptr align 1 {to}, ptr align 1 {from}, i64 {width}, i1 false)"
    ));
}

/// Copy a proved disjoint interval of one through four bytes in place.
///
/// Both spans are initialized and contain exactly `length` accessed bytes.
/// Two two-byte chunks cover lengths two through four without a wide load.
/// Overlapping receiving chunks write the same byte from the disjoint source.
pub(super) fn tiny_copy(
    f: &mut Function,
    destination: &str,
    source: &str,
    length: &str,
    done: &str,
) {
    let single = f.fresh_label();
    let pair = f.fresh_label();
    // A one-byte interval cannot admit either of the two-byte chunks.
    let one = f.value(format!("icmp eq i64 {length}, 1"));
    f.line(format!("br i1 {one}, label %{single}, label %{pair}"));
    f.label(&single);
    chunk(f, destination, source, "0", 1);
    f.line(format!("br label %{done}"));
    f.label(&pair);
    chunk(f, destination, source, "0", 2);
    let offset = f.value(format!("sub i64 {length}, 2"));
    chunk(f, destination, source, &offset, 2);
    f.line(format!("br label %{done}"));
}

/// Emit bounded head and tail copies and branch to the initialized continuation.
///
/// Both payload ranges contain exactly `length` valid bytes for this operation.
/// Repeated destination offsets receive the identical initialized source byte.
/// The empty case is excluded by dispatch and no load extends into neighboring
/// metadata, padding, or spare capacity in the emitted IR.
pub(super) fn short_copy(
    f: &mut Function,
    destination: &str,
    source: &str,
    length: &str,
    done: &str,
) {
    // Descending guards establish width <= length <= 2 * width. Each head and
    // tail pair then covers the entire range without reading across its edges.
    // A 32-byte pair can move more repeated bytes than a 16-byte loop, while
    // removing loop counters and branches from the short vector path.
    for (width, minimum) in [(32, 33), (16, 17), (8, 8), (4, 4)] {
        let pair = f.fresh_label();
        let smaller = f.fresh_label();
        let enough = f.value(format!("icmp uge i64 {length}, {minimum}"));
        f.line(format!("br i1 {enough}, label %{pair}, label %{smaller}"));
        f.label(&pair);
        chunk(f, destination, source, "0", width);
        let offset = f.value(format!("sub i64 {length}, {width}"));
        chunk(f, destination, source, &offset, width);
        f.line(format!("br label %{done}"));
        f.label(&smaller);
    }
    // For lengths 1..3, the first, middle, and last positions cover the range.
    let middle = f.value(format!("lshr i64 {length}, 1"));
    let last = f.value(format!("sub i64 {length}, 1"));
    chunk(f, destination, source, "0", 1);
    chunk(f, destination, source, &middle, 1);
    chunk(f, destination, source, &last, 1);
    f.line(format!("br label %{done}"));
}

#[cfg(test)]
mod tests {
    //! Eligibility tests keep strict expression and target boundaries explicit.

    use super::{Choice, choose};
    use crate::ir::{Binary, Expr};

    /// The arm64 macOS target used by the native driver.
    const MAC: &str = "arm64-apple-macosx11.0.0";

    /// Build one input read owning its index expression.
    fn read(index: Expr) -> Expr {
        Expr::InputByte {
            param: 0,
            index: Box::new(index),
        }
    }

    /// Both nonzero constant-offset addition orders qualify.
    #[test]
    fn accepts_only_direct_contiguous_input_subranges() {
        // Addition is commutative here. Each operand is a leaf with no eager work.
        for (index, offset) in [
            (
                Expr::binary(Binary::AddWrapping, Expr::Const(17), Expr::Index),
                17,
            ),
            (
                Expr::binary(Binary::AddWrapping, Expr::Index, Expr::Const(u64::MAX)),
                u64::MAX,
            ),
        ] {
            // The last offset is deliberately extreme. The emitted range check
            // rejects it for real finite backing and preserves original traps.
            assert_eq!(
                choose(MAC, &Expr::Param(1), &read(index)),
                Choice::GuardedInline { input: 0, offset }
            );
        }
        let trapping_actual = Expr::Apply {
            args: vec![read(Expr::Index), read(Expr::Const(u64::MAX))],
            body: Box::new(Expr::Param(0)),
        };
        assert!(matches!(
            choose(MAC, &Expr::Param(1), &trapping_actual),
            Choice::CheckedLoop(_)
        ));
        assert!(matches!(
            choose(MAC, &Expr::Param(1), &read(Expr::Const(0))),
            Choice::CheckedLoop(_)
        ));
    }

    /// Untested targets and the existing whole-input lowering stay on the loop.
    #[test]
    fn preserves_existing_policy_outside_the_measured_scope() {
        let value = read(Expr::binary(
            Binary::AddWrapping,
            Expr::Index,
            Expr::Const(8),
        ));
        assert!(matches!(
            choose("aarch64-unknown-linux-gnu", &Expr::Param(1), &value),
            Choice::CheckedLoop(_)
        ));
        assert!(matches!(
            choose("x86_64-apple-macosx10.13.0", &Expr::Param(1), &value),
            Choice::CheckedLoop(_)
        ));
        // Whole extent, empty and fixed longer lengths retain the established path.
        for length in [Expr::InputLen(0), Expr::Const(0), Expr::Const(65)] {
            assert!(matches!(
                choose(MAC, &length, &value),
                Choice::CheckedLoop(_)
            ));
        }
        assert_eq!(
            choose(MAC, &Expr::Const(64), &value),
            Choice::GuardedInline {
                input: 0,
                offset: 8
            }
        );
        // Both zero-offset forms are outside the initial policy even with an
        // independently sized destination.
        for index in [
            Expr::Index,
            Expr::binary(Binary::AddWrapping, Expr::Index, Expr::Const(0)),
        ] {
            assert!(matches!(
                choose(MAC, &Expr::Param(1), &read(index)),
                Choice::CheckedLoop(_)
            ));
        }
    }
}
