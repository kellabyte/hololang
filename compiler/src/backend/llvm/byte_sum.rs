//! Checked, exact-width byte-sum selection and LLVM emission for macOS arm64.
//!
//! A counted loop can reduce initialized bytes in 16-byte groups when the
//! checked loop walks one live extent and only writes a private scalar sum.
//! A lower-bound proof can select a separate 64-byte routine. Unmatched loops
//! keep their original checks and evaluation order. Neither routine needs
//! scratch payload, allocation, aligned input, or alias claims.

use std::cell::Cell;

use crate::ir::{Binary, BytesSource, Expr, Op};

use super::{context::Binding, context::Lowering, input::Input};

/// Minimum proved extent for selecting the wider reduction.
/// Four full 64-byte blocks amortize the transition to the original tail.
pub(super) const LARGE_MIN: u64 = 256;

/// One complete three-block prefix, with no call to the narrow remainder path.
const THREE_BLOCK_BYTES: u64 = 3 * 64;

/// Private helpers actually used in one emitted module.
pub(super) struct Helpers {
    /// Whether a checked literal of three full blocks may use the wide routine.
    exact_three_blocks: bool,
    /// A general reduction has been selected or is needed by a wide tail.
    narrow: Cell<bool>,
    /// At least one call has proved the lower bound for the wider reduction.
    wide: Cell<bool>,
}

impl Helpers {
    /// Set the compile-time policy before any uses of the native helpers.
    pub(super) fn new(exact_three_blocks: bool) -> Self {
        Self {
            exact_three_blocks,
            narrow: Cell::new(false),
            wide: Cell::new(false),
        }
    }

    /// Test a literal width that has passed checked call-shape selection.
    /// The three-block policy adds one exact multiple to the large-prefix rule.
    pub(super) fn selects_wide(&self, width: u64) -> bool {
        width >= LARGE_MIN || (self.exact_three_blocks && width == THREE_BLOCK_BYTES)
    }

    /// Mark the chosen helper used and return its private LLVM name.
    /// A wide call can delegate its exact remainder to the narrow routine.
    pub(super) fn select(&self, wide: bool) -> &'static str {
        self.narrow.set(true);
        // A wide helper is emitted only for a module with an actual proof.
        if wide {
            self.wide.set(true);
            "holo.byte.sum.large"
        } else {
            "holo.byte.sum"
        }
    }

    /// Append only the helper definitions required by selected calls.
    /// Modules without a wide call keep the original narrow helper text.
    pub(super) fn emit(self, module: &mut String) {
        if self.narrow.into_inner() {
            module.push_str(HELPER);
        }
        if self.wide.into_inner() {
            module.push_str(LARGE_HELPER);
        }
    }
}

/// Return whether `target` matches the measured macOS arm64 native configuration.
/// The check reads target text only and does not query or alter the host machine.
pub(super) fn supports_target(target: &str) -> bool {
    let mut parts = target.split('-');
    // Unsupported or malformed triples leave loop selection disabled.
    matches!(parts.next(), Some("arm64" | "aarch64"))
        && parts.next() == Some("apple")
        && parts
            .next()
            .is_some_and(|os| os.starts_with("macosx") || os.starts_with("darwin"))
}

/// Prove one counted loop's private wrapping accumulator and initialized span.
/// `count` is the bound evaluated before iteration, `index_name` is the checked
/// immutable counter, and `body` must have no other observable work. A match
/// returns the outer scalar name and existing bytes descriptor. A field match
/// clones compiler-owned names only. An unproved loop returns `None`.
pub(super) fn recognize<'a>(
    count: &Expr,
    index_name: &str,
    body: &'a [Op],
) -> Option<(&'a str, BytesSource)> {
    // Check the whole body. Parsed wrapping calls capture both actuals in order;
    // checked IR can express the same operation without those private locals.
    let (name, op, left, right) = match body {
        [
            Op::Set {
                name,
                value: Expr::Binary { op, left, right },
            },
        ] => (name, op, left.as_ref(), right.as_ref()),
        [
            Op::Let {
                name: first,
                value: first_value,
            },
            Op::Let {
                name: second,
                value: second_value,
            },
            Op::Set {
                name,
                value: Expr::Binary { op, left, right },
            },
        ] if first != second
            && first != index_name
            && second != index_name
            && first != name
            && second != name
            && is_local(left, first)
            && is_local(right, second) =>
        {
            (name, op, first_value, second_value)
        }
        _ => return None,
    };
    // A different scalar operator, counter mutation, or second expression can
    // change the result or trap. Only modulo-u64 addition can be regrouped here.
    if !matches!(op, Binary::AddWrapping) || name == index_name {
        return None;
    }
    let byte = if is_local(left, name) {
        right
    } else if is_local(right, name) {
        left
    } else {
        return None;
    };
    // Count and read must use the exact same live projection or captured formal.
    // Nothing in the matched loop can rebind its reference, republish an extent,
    // expose a scalar slot, or mutate payload through an alias.
    let source = match (count, byte) {
        (
            Expr::BytesLen { reference, path },
            Expr::Byte {
                reference: read_reference,
                path: read_path,
                index,
            },
        ) if reference == read_reference && path == read_path && is_local(index, index_name) => {
            BytesSource::Field {
                reference: reference.clone(),
                path: path.clone(),
            }
        }
        (Expr::InputLen(param), Expr::InputByte { param: read, index })
            if param == read && is_local(index, index_name) =>
        {
            BytesSource::Param(*param)
        }
        _ => return None,
    };
    Some((name, source))
}

/// Test whether `expr` reads the exact checked scalar named `name`.
/// This does not evaluate the expression or alter lexical state.
fn is_local(expr: &Expr, name: &str) -> bool {
    matches!(expr, Expr::Local(local) if local == name)
}

/// Attempt native byte-sum lowering for a checked loop without entering its scope.
/// Success emits one extent capture and one private accumulator update and marks
/// the helper used. Failure emits nothing and leaves ordinary lowering in charge.
/// The helper takes its seed modulo u64 and performs no load on an empty extent.
pub(super) fn try_lower(
    lowering: &mut Lowering<'_>,
    count: &Expr,
    index: &str,
    body: &[Op],
) -> bool {
    // Check both target support and the proof before changing function state.
    let Some(used) = lowering.byte_sums else {
        return false;
    };
    let Some((name, source)) = recognize(count, index, body) else {
        return false;
    };
    // Dynamic loops retain the narrow helper, even inside a checked branch.
    let helper = used.select(false);
    let Input::Bytes { contents, len } = lowering.capture_bytes(&source) else {
        unreachable!("checked byte reduction uses an initialized span");
    };
    let Binding::Scalar(slot) = lowering.binding(name) else {
        unreachable!("checked byte reduction updates a scalar");
    };
    let seed = lowering.f.value(format!("load i64, ptr {slot}"));
    let sum = lowering.f.value(format!(
        "call i64 @{helper}(ptr {contents}, i64 {len}, i64 {seed})"
    ));
    lowering.f.line(format!("store i64 {sum}, ptr {slot}"));
    true
}

/// The bounded helper from the measured prototype, shared by all selected loops.
/// Full blocks and descending binary tails partition the exact initialized extent.
/// A 16-byte partial sum is at most 4,080, so its i16 reduction is exact. The
/// outer additions have no overflow flags and wrap at 64 bits. Each load has
/// alignment 1, every pointer advance is ordinary GEP, and a zero-length call
/// reaches the return without reading payload memory.
pub(super) const HELPER: &str = r#"
define internal i64 @holo.byte.sum(ptr %data, i64 %length, i64 %seed) noinline {
entry:
  %has.block = icmp uge i64 %length, 16
  br i1 %has.block, label %block, label %tail8
block:
  ; Process an entire block only when at least sixteen bytes remain.
  %cursor = phi ptr [ %data, %entry ], [ %block.next, %block ]
  %remaining = phi i64 [ %length, %entry ], [ %block.left, %block ]
  %sum = phi i64 [ %seed, %entry ], [ %block.total, %block ]
  %block.bytes = load <16 x i8>, ptr %cursor, align 1
  %block.wide = zext <16 x i8> %block.bytes to <16 x i16>
  %block.reduced = call i16 @llvm.vector.reduce.add.v16i16(<16 x i16> %block.wide)
  %block.sum = zext i16 %block.reduced to i64
  %block.total = add i64 %sum, %block.sum
  %block.next = getelementptr i8, ptr %cursor, i64 16
  %block.left = sub nuw i64 %remaining, 16
  %has.more = icmp uge i64 %block.left, 16
  br i1 %has.more, label %block, label %tail8
tail8:
  ; Each guard selects a disjoint chunk; after this sequence no bytes remain.
  %tail8.cursor = phi ptr [ %data, %entry ], [ %block.next, %block ]
  %tail8.left = phi i64 [ %length, %entry ], [ %block.left, %block ]
  %tail8.total = phi i64 [ %seed, %entry ], [ %block.total, %block ]
  %tail8.fits = icmp uge i64 %tail8.left, 8
  br i1 %tail8.fits, label %tail8.read, label %tail4
tail8.read:
  %tail8.bytes = load <8 x i8>, ptr %tail8.cursor, align 1
  %tail8.wide = zext <8 x i8> %tail8.bytes to <8 x i16>
  %tail8.reduced = call i16 @llvm.vector.reduce.add.v8i16(<8 x i16> %tail8.wide)
  %tail8.sum = zext i16 %tail8.reduced to i64
  %tail8.result = add i64 %tail8.total, %tail8.sum
  %tail8.next = getelementptr i8, ptr %tail8.cursor, i64 8
  %tail8.remainder = sub nuw i64 %tail8.left, 8
  br label %tail4
tail4:
  %tail4.cursor = phi ptr [ %tail8.cursor, %tail8 ], [ %tail8.next, %tail8.read ]
  %tail4.left = phi i64 [ %tail8.left, %tail8 ], [ %tail8.remainder, %tail8.read ]
  %tail4.total = phi i64 [ %tail8.total, %tail8 ], [ %tail8.result, %tail8.read ]
  %tail4.fits = icmp uge i64 %tail4.left, 4
  br i1 %tail4.fits, label %tail4.read, label %tail2
tail4.read:
  %tail4.bytes = load <4 x i8>, ptr %tail4.cursor, align 1
  %tail4.wide = zext <4 x i8> %tail4.bytes to <4 x i16>
  %tail4.reduced = call i16 @llvm.vector.reduce.add.v4i16(<4 x i16> %tail4.wide)
  %tail4.sum = zext i16 %tail4.reduced to i64
  %tail4.result = add i64 %tail4.total, %tail4.sum
  %tail4.next = getelementptr i8, ptr %tail4.cursor, i64 4
  %tail4.remainder = sub nuw i64 %tail4.left, 4
  br label %tail2
tail2:
  %tail2.cursor = phi ptr [ %tail4.cursor, %tail4 ], [ %tail4.next, %tail4.read ]
  %tail2.left = phi i64 [ %tail4.left, %tail4 ], [ %tail4.remainder, %tail4.read ]
  %tail2.total = phi i64 [ %tail4.total, %tail4 ], [ %tail4.result, %tail4.read ]
  %tail2.fits = icmp uge i64 %tail2.left, 2
  br i1 %tail2.fits, label %tail2.read, label %tail1
tail2.read:
  ; Summing the low and high bytes is independent of their order in this word.
  %tail2.bytes = load i16, ptr %tail2.cursor, align 1
  %tail2.low = and i16 %tail2.bytes, 255
  %tail2.high = lshr i16 %tail2.bytes, 8
  %tail2.reduced = add nuw i16 %tail2.low, %tail2.high
  %tail2.sum = zext i16 %tail2.reduced to i64
  %tail2.result = add i64 %tail2.total, %tail2.sum
  %tail2.next = getelementptr i8, ptr %tail2.cursor, i64 2
  %tail2.remainder = sub nuw i64 %tail2.left, 2
  br label %tail1
tail1:
  %tail1.cursor = phi ptr [ %tail2.cursor, %tail2 ], [ %tail2.next, %tail2.read ]
  %tail1.left = phi i64 [ %tail2.left, %tail2 ], [ %tail2.remainder, %tail2.read ]
  %tail1.total = phi i64 [ %tail2.total, %tail2 ], [ %tail2.result, %tail2.read ]
  %tail1.fits = icmp uge i64 %tail1.left, 1
  br i1 %tail1.fits, label %tail1.read, label %done
tail1.read:
  %tail1.bytes = load i8, ptr %tail1.cursor, align 1
  %tail1.sum = zext i8 %tail1.bytes to i64
  %tail1.result = add i64 %tail1.total, %tail1.sum
  br label %done
done:
  %result = phi i64 [ %tail1.total, %tail1 ], [ %tail1.result, %tail1.read ]
  ret i64 %result
}

declare i16 @llvm.vector.reduce.add.v16i16(<16 x i16>)

declare i16 @llvm.vector.reduce.add.v8i16(<8 x i16>)

declare i16 @llvm.vector.reduce.add.v4i16(<4 x i16>)
"#;

/// Separate wide reduction for a proved large initialized input. The first
/// load is safe by the checked literal at the call site; the back
/// edge tests the remaining count. A 64-byte partial fits i16 (at most 16,320).
/// Nonempty tails reuse the unchanged narrow helper with the advanced pointer.
const LARGE_HELPER: &str = r#"
define internal i64 @holo.byte.sum.large(ptr %data, i64 %length, i64 %seed) noinline {
entry:
  br label %block
block:
  %cursor = phi ptr [ %data, %entry ], [ %next, %block ]
  %remaining = phi i64 [ %length, %entry ], [ %left, %block ]
  %sum = phi i64 [ %seed, %entry ], [ %total, %block ]
  %bytes = load <64 x i8>, ptr %cursor, align 1
  %words = zext <64 x i8> %bytes to <64 x i16>
  %reduced = call i16 @llvm.vector.reduce.add.v64i16(<64 x i16> %words)
  %partial = zext i16 %reduced to i64
  %total = add i64 %sum, %partial
  %next = getelementptr i8, ptr %cursor, i64 64
  %left = sub nuw i64 %remaining, 64
  %more = icmp uge i64 %left, 64
  br i1 %more, label %block, label %finish
finish:
  %empty = icmp eq i64 %left, 0
  br i1 %empty, label %done, label %tail
tail:
  %result = tail call i64 @holo.byte.sum(ptr %next, i64 %left, i64 %total)
  ret i64 %result
done:
  ret i64 %total
}

declare i16 @llvm.vector.reduce.add.v64i16(<64 x i16>)
"#;
