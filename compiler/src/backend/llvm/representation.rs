//! Typed record extents and indexed borrowing for explicitly selected layouts.
//!
//! A retained header permits a single size load. Headerless schemas either have a
//! constant extent or derive it from their final variable field's frozen metadata.
//! Helpers never scan payload bytes or enumerate records to free them. Indexed
//! borrowing remains O(index), while cursor traversal performs one typed hop.

use crate::{
    ir::{FieldKind, Program},
    layout::LayoutPlan,
};

use super::{context::Lowering, function::Function, storage};

/// Emit one positive record-size helper for a checked schema and selected plan.
/// The result owns compiler text; native reads concern initialized shape metadata
/// only. Publication and immutable shape establish nonwrapping valid extents.
pub(super) fn size(program: &Program, layout: &LayoutPlan, index: usize) -> String {
    let record = &program.records[index];
    let representation = layout.record(&record.name);
    let header = format!("define internal i64 @holo.record_size.{index}(ptr %record)");
    let mut lowering = Lowering::new(program, layout, header, Vec::new());
    // Wide/retained records keep the direct frozen-size load, preserving their
    // constant-work skip even when earlier fields have several variable components.
    if representation.header_bytes != 0 {
        let size = storage::load_u64(&mut lowering.f, "%record");
        lowering.f.line(format!("ret i64 {size}"));
        return lowering.f.finish();
    }
    // Fixed schemas, including the initialized empty unit byte, need no memory read.
    if let Some(size) = representation.fixed_size {
        lowering.f.line(format!("ret i64 {size}"));
        return lowering.f.finish();
    }
    let mut pointer = "%record".to_owned();
    // A headerless variable schema has one final variable field. Fixed prefixes
    // and the final frozen length/child size determine its extent without a payload walk.
    for field in &record.fields {
        pointer = match &field.kind {
            FieldKind::U64 => storage::advance(&mut lowering.f, &pointer, "8"),
            FieldKind::Bytes => {
                let length = lowering.byte_length(&pointer);
                let contents = storage::advance(
                    &mut lowering.f,
                    &pointer,
                    &layout.length_bytes().to_string(),
                );
                storage::advance(&mut lowering.f, &contents, &length)
            }
            FieldKind::Record(name) => {
                let size = lowering.record_size(name, &pointer);
                storage::advance(&mut lowering.f, &pointer, &size)
            }
        };
    }
    let end = lowering.f.value(format!("ptrtoint ptr {pointer} to i64"));
    let start = lowering.f.value("ptrtoint ptr %record to i64");
    let size = lowering.f.value(format!("sub i64 {end}, {start}"));
    lowering.f.line(format!("ret i64 {size}"));
    lowering.f.finish()
}

/// Emit bounds-checked sequential lookup for one statically known record schema.
/// `index` identifies the helper and its size callee; no runtime callback or layout
/// dispatch is introduced. The loop advances exactly the requested valid prefix.
pub(super) fn indexed(index: usize) -> String {
    let mut f = Function::new(format!(
        "define internal ptr @holo.record_at.{index}(ptr %batch, i64 %index)"
    ));
    let count_slot = storage::member(&mut f, "%holo.batch", "%batch", 3);
    let count = f.value(format!("load i64, ptr {count_slot}"));
    let valid = f.value(format!("icmp ult i64 %index, {count}"));
    f.line(format!("call void @holo.assert(i1 {valid})"));
    let region_slot = storage::member(&mut f, "%holo.batch", "%batch", 0);
    let region = f.value(format!("load ptr, ptr {region_slot}"));
    let base_slot = storage::member(&mut f, "%holo.region", &region, 0);
    let base = f.value(format!("load ptr, ptr {base_slot}"));
    let start_slot = storage::member(&mut f, "%holo.batch", "%batch", 1);
    let start = f.value(format!("load i64, ptr {start_slot}"));
    f.line("br label %lookup");
    f.label("lookup");
    f.line("%position = phi i64 [ 0, %entry ], [ %following.position, %advance ]");
    f.line(format!(
        "%offset = phi i64 [ {start}, %entry ], [ %following.offset, %advance ]"
    ));
    f.line(format!(
        "%address = getelementptr i8, ptr {base}, i64 %offset"
    ));
    f.line("%found = icmp eq i64 %position, %index");
    // Published positive extents guarantee termination after index valid record hops.
    f.line("br i1 %found, label %done, label %advance");
    f.label("advance");
    f.line(format!(
        "%size = call i64 @holo.record_size.{index}(ptr %address)"
    ));
    f.line("%following.offset = add i64 %offset, %size");
    f.line("%following.position = add i64 %position, 1");
    f.line("br label %lookup");
    f.label("done");
    f.line("ret ptr %address");
    f.finish()
}
