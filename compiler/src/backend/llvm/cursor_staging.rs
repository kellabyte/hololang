//! Emit a checked transfer already selected by constructed-slot induction.
//!
//! The entry-only plan establishes both initialized ranges and distinct live
//! backing. The small-width dispatch adds no range check, new storage, or
//! source-level precondition to the selected operation. The qualified range
//! uses a private shared leaf for wide transfers.

use std::{cell::Cell, ops::RangeInclusive};

use crate::ir::{Expr, Op};

use super::{context::Lowering, copy, staging_copy::ranges};

/// Private native symbol for the proof-selected wide edge.
const OUTLINED_BULK: &str = "holo.stage.bulk.transfer";

/// Compiler-only admission bounds and use marker for the shared leaf.
pub(super) struct Outline {
    /// Maximum-transfer bounds eligible for the selected compiler policy.
    maxima: RangeInclusive<u64>,
    /// Whether a proved entry site emitted a call to the leaf.
    used: Cell<bool>,
}

impl Outline {
    /// Admit proved bounded copies by default and the full experimental range
    /// for explicit API callers. These bounds affect only compiler selection.
    pub(super) fn new(experimental: bool) -> Self {
        // Four 64-byte groups cover the measured small-buffer workload. The
        // ordinary intrinsic stays available for larger copies, and a site
        // proved always short needs no extra helper in its unoptimized IR.
        let maxima = if experimental {
            0..=u64::MAX
        } else {
            (copy::LIMIT + 1)..=(4 * copy::LIMIT)
        };
        Self {
            maxima,
            used: Cell::new(false),
        }
    }

    /// Report whether emission must append the shared definition.
    pub(super) fn used(&self) -> bool {
        self.used.get()
    }
}

/// Emit one exact-width copy of proved initialized, disjoint intervals.
/// The original count remains single-evaluation and a zero count bypasses
/// address formation. No extra range guard or runtime proof is emitted.
pub(super) fn try_lower(
    lowering: &mut Lowering<'_>,
    count: &Expr,
    index: &str,
    body: &[Op],
) -> bool {
    let Some(maximum) = lowering
        .cursor_staging
        .and_then(|plan| plan.get(&body.as_ptr()))
        .copied()
    else {
        return false;
    };
    let (destination, source) = ranges(body, index).expect("plan selected a matched staging loop");
    let count = lowering.expression(count);
    let active = lowering.f.fresh_label();
    let done = lowering.f.fresh_label();
    let nonempty = lowering.f.value(format!("icmp ne i64 {count}, 0"));
    lowering.f.line("; Holo constructed slot cursor proof");
    lowering
        .f
        .line(format!("br i1 {nonempty}, label %{active}, label %{done}"));
    lowering.f.label(&active);
    let destination = destination.capture(lowering).address(lowering);
    let source = source.capture(lowering).address(lowering);
    let one = lowering.f.fresh_label();
    let more = lowering.f.fresh_label();
    let two = lowering.f.fresh_label();
    let more_than_two = lowering.f.fresh_label();
    let three = lowering.f.fresh_label();
    let larger = lowering.f.fresh_label();
    let short = lowering.f.fresh_label();
    let bulk = lowering.f.fresh_label();

    // Keep the exact singleton path ahead of the other cases. The branch hint
    // is an isolated code-placement choice and is not used as a proof.
    let singleton = lowering.f.value(format!("icmp eq i64 {count}, 1"));
    lowering
        .f
        .construction_branch(&singleton, &one, &more, Some(true));
    lowering.f.label(&one);
    copy::chunk(&mut lowering.f, &destination, &source, "0", 1);
    lowering.f.line(format!("br label %{done}"));

    lowering.f.label(&more);
    // The halfword arm accesses exactly the two already proved bytes.
    let halfword = lowering.f.value(format!("icmp eq i64 {count}, 2"));
    lowering
        .f
        .construction_branch(&halfword, &two, &more_than_two, None);
    lowering.f.label(&two);
    copy::chunk(&mut lowering.f, &destination, &source, "0", 2);
    lowering.f.line(format!("br label %{done}"));

    lowering.f.label(&more_than_two);
    // A complete three-byte transfer uses one halfword and one trailing byte.
    let triple = lowering.f.value(format!("icmp eq i64 {count}, 3"));
    lowering
        .f
        .construction_branch(&triple, &three, &larger, None);
    lowering.f.label(&three);
    copy::chunk(&mut lowering.f, &destination, &source, "0", 2);
    copy::chunk(&mut lowering.f, &destination, &source, "2", 1);
    lowering.f.line(format!("br label %{done}"));
    lowering.f.label(&larger);
    let bounded = lowering
        .f
        .value(format!("icmp ule i64 {count}, {}", copy::LIMIT));
    lowering
        .f
        .line(format!("br i1 {bounded}, label %{short}, label %{bulk}"));
    lowering.f.label(&short);
    copy::short_copy(&mut lowering.f, &destination, &source, &count, &done);
    lowering.f.label(&bulk);
    // The existing branch proves the lower bound at runtime. The compile-time
    // maximum chooses the leaf or bulk intrinsic with identical dispatch.
    if let Some(outline) = lowering.outlined_cursor_staging
        && outline.maxima.contains(&maximum)
    {
        outline.used.set(true);
        lowering.f.line(format!(
            "call preserve_mostcc void @{OUTLINED_BULK}(ptr {destination}, ptr {source}, i64 {count})"
        ));
    } else {
        lowering.f.line(format!("call void @llvm.memcpy.p0.p0.i64(ptr align 1 {destination}, ptr align 1 {source}, i64 {count}, i1 false)"));
    }
    lowering.f.line(format!("br label %{done}"));
    lowering.f.label(&done);
    true
}

/// Append the private native leaf shared by all outlined entry copy sites.
///
/// Its caller proves that `count` exceeds the block width and supplies live,
/// initialized, disjoint spans of that length. The leaf borrows those spans,
/// reads source bytes, and changes only the destination bytes. The loop starts
/// each full block strictly before `count - width`, so advancing by `width`
/// cannot overflow. The end-aligned block completes the copy and any overlap
/// repeats a write of the same value from the disjoint source. There are no
/// fallible operations or backing acquisitions inside the leaf. The private
/// convention gives LLVM more register-placement choices at the call site.
pub(super) fn emit_outlined_bulk(module: &mut String) {
    module.push_str(&format!(
        r#"
define internal preserve_mostcc void @{OUTLINED_BULK}(ptr %destination, ptr %source, i64 %count) noinline {{
entry:
  %holo.stage.bulk.last = sub i64 %count, {width}
  br label %holo.stage.bulk.loop
holo.stage.bulk.loop:
  %holo.stage.bulk.offset = phi i64 [ 0, %entry ], [ %holo.stage.bulk.next, %holo.stage.bulk.loop ]
  %holo.stage.bulk.to = getelementptr i8, ptr %destination, i64 %holo.stage.bulk.offset
  %holo.stage.bulk.from = getelementptr i8, ptr %source, i64 %holo.stage.bulk.offset
  call void @llvm.memcpy.inline.p0.p0.i64(ptr align 1 %holo.stage.bulk.to, ptr align 1 %holo.stage.bulk.from, i64 {width}, i1 false)
  %holo.stage.bulk.next = add i64 %holo.stage.bulk.offset, {width}
  %holo.stage.bulk.more = icmp ult i64 %holo.stage.bulk.next, %holo.stage.bulk.last
  br i1 %holo.stage.bulk.more, label %holo.stage.bulk.loop, label %holo.stage.bulk.tail
holo.stage.bulk.tail:
  %holo.stage.bulk.tail.to = getelementptr i8, ptr %destination, i64 %holo.stage.bulk.last
  %holo.stage.bulk.tail.from = getelementptr i8, ptr %source, i64 %holo.stage.bulk.last
  call void @llvm.memcpy.inline.p0.p0.i64(ptr align 1 %holo.stage.bulk.tail.to, ptr align 1 %holo.stage.bulk.tail.from, i64 {width}, i1 false)
  ret void
}}
"#,
        width = copy::LIMIT
    ));
}
