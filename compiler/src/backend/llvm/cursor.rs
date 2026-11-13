//! Linear traversal of one immutable-shape published batch prefix.
//!
//! A cursor captures publication metadata before entering its body. Appending may
//! extend the batch but never changes the captured end or existing record sizes.
//! Traversal advances its own physical offset, independent of any rebindings of
//! the source reference. Native work is one typed frozen-extent hop per record,
//! rather than repeatedly looking up an increasing index from the batch start.

use crate::ir::Op;

use super::{
    context::{Binding, Lowering},
    storage,
};

impl Lowering<'_> {
    /// Traverse a checked batch's original published records in physical order.
    ///
    /// `batch_name` names the live owner, `reference_name` is fresh per iteration,
    /// and `body` is already permission/root checked. Captured count/end identify
    /// the same finite prefix because publication and shape invariants establish
    /// their consistency. Advancing by end needs no second native counter. The
    /// checker prevents reset while the per-iteration reference remains usable.
    pub(super) fn for_each(&mut self, batch_name: &str, reference_name: &str, body: &[Op]) {
        let origin = self.batch_origin(batch_name);
        let shape = self.uniform_batch_shape(batch_name);
        let (record, base, start, end) = match self.binding(batch_name) {
            Binding::Batch {
                slot: batch,
                record,
                ..
            } => {
                let region_slot = storage::member(&mut self.f, "%holo.batch", &batch, 0);
                let region = self.f.value(format!("load ptr, ptr {region_slot}"));
                let base_slot = storage::member(&mut self.f, "%holo.region", &region, 0);
                let base = self.f.value(format!("load ptr, ptr {base_slot}"));
                let start_slot = storage::member(&mut self.f, "%holo.batch", &batch, 1);
                let start = self.f.value(format!("load i64, ptr {start_slot}"));
                let end_slot = storage::member(&mut self.f, "%holo.batch", &batch, 2);
                let end = self.f.value(format!("load i64, ptr {end_slot}"));
                let count_slot = storage::member(&mut self.f, "%holo.batch", &batch, 3);
                let _count = self.f.value(format!("load i64, ptr {count_slot}"));
                (record, base, start, end)
            }
            Binding::BorrowedBatch { prefix, record } => {
                (record, prefix.contents, "0".to_owned(), prefix.extent)
            }
            _ => unreachable!("checked cursor requires a batch"),
        };
        let offset_slot = self.f.slot("i64");
        let reference_slot = self.f.slot("ptr");
        self.f.line(format!("store i64 {start}, ptr {offset_slot}"));
        let condition = self.f.fresh_label();
        let iteration = self.f.fresh_label();
        let advance = self.f.fresh_label();
        let done = self.f.fresh_label();
        self.f.line(format!("br label %{condition}"));
        self.f.label(&condition);
        let offset = self.f.value(format!("load i64, ptr {offset_slot}"));
        let remaining = self.f.value(format!("icmp ult i64 {offset}, {end}"));
        // Every published record has a positive frozen extent and ends within
        // the captured prefix. Empty batches skip the body, and new appends cannot
        // lengthen this traversal because the end value is already captured.
        self.f.line(format!(
            "br i1 {remaining}, label %{iteration}, label %{done}"
        ));
        self.f.label(&iteration);
        let pointer = storage::advance(&mut self.f, &base, &offset);
        // A positive proved stride covers the same captured prefix as the frozen
        // sizes. Unproved batches still read each record's own representation.
        let size = match shape {
            Some(shape) => shape.stride.to_string(),
            None => self.record_size(&record, &pointer),
        };
        let following = self.f.value(format!("add i64 {offset}, {size}"));
        self.f
            .line(format!("store ptr {pointer}, ptr {reference_slot}"));
        self.push_scope();
        self.bind_reference(
            reference_name,
            reference_slot,
            record,
            origin,
            shape.map(|shape| shape.extent),
        );
        self.push_loop(&done, &advance);
        self.operations(body);
        self.pop_loop();
        self.pop_scope();
        self.f.line(format!("br label %{advance}"));
        self.f.label(&advance);
        self.f
            .line(format!("store i64 {following}, ptr {offset_slot}"));
        self.f.line(format!("br label %{condition}"));
        self.f.label(&done);
    }
}
