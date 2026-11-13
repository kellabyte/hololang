//! Structured native scopes and counted/conditional control flow.
//!
//! Region acquisition/free and batch marker restoration occur at explicit lexical
//! boundaries. Entry completion emits the same active cleanups before termination.
//! Fixed native metadata/scalar slots are hoisted to function entry, so repeating
//! a source scope reuses slots rather than growing the call stack each iteration.

use crate::ir::{Condition, Expr, Op};

use super::{
    byte_sum,
    context::{Binding, Cleanup, Lowering},
    cursor_staging, staging_copy, staging_slots, storage, uniform_batch,
};

impl Lowering<'_> {
    /// Acquire bounded backing and emit exactly one success/failure scope.
    ///
    /// `capacity` is evaluated once. `fail_acquire` selects the real acquisition
    /// failure edge for tests; normal allocation failure takes the same scope.
    /// The owner is visible only in `body`, and successful backing is freed once.
    pub(super) fn region_scope(
        &mut self,
        name: &str,
        capacity: &Expr,
        fail_acquire: bool,
        body: &[Op],
        on_failure: &[Op],
    ) {
        let capacity = self.expression(capacity);
        let metadata = self.f.slot("%holo.region");
        let acquire = self.f.fresh_label();
        let acquired = self.f.fresh_label();
        let failed = self.f.fresh_label();
        let done = self.f.fresh_label();
        // The injected fault uses the normal failure path without fabricating an
        // owner or allocating unused backing that would need separate cleanup.
        self.f.line(format!(
            "br i1 {}, label %{failed}, label %{acquire}",
            if fail_acquire { "true" } else { "false" }
        ));
        self.f.label(&acquire);
        let empty = self.f.value(format!("icmp eq i64 {capacity}, 0"));
        let physical = self
            .f
            .value(format!("select i1 {empty}, i64 1, i64 {capacity}"));
        let base = self.f.value(format!("call ptr @malloc(i64 {physical})"));
        let unavailable = self.f.value(format!("icmp eq ptr {base}, null"));
        // A zero-byte logical region still gets a non-null one-byte allocation;
        // its capacity remains zero and no payload reservation can use that byte.
        self.f.line(format!(
            "br i1 {unavailable}, label %{failed}, label %{acquired}"
        ));
        self.f.label(&acquired);
        let base_slot = storage::member(&mut self.f, "%holo.region", &metadata, 0);
        self.f.line(format!("store ptr {base}, ptr {base_slot}"));
        let capacity_slot = storage::member(&mut self.f, "%holo.region", &metadata, 1);
        self.f
            .line(format!("store i64 {capacity}, ptr {capacity_slot}"));
        storage::restore(&mut self.f, &metadata, "0");
        self.push_scope();
        self.bind(name, Binding::Region(metadata));
        self.cleanups.push(Cleanup::Region { base: base.clone() });
        self.operations(body);
        self.cleanups.pop().expect("region cleanup remains active");
        self.pop_scope();
        self.f.line(format!("call void @free(ptr {base})"));
        self.f.line(format!("br label %{done}"));
        self.f.label(&failed);
        self.push_scope();
        self.operations(on_failure);
        self.pop_scope();
        self.f.line(format!("br label %{done}"));
        self.f.label(&done);
    }

    /// Give one checked batch temporary ownership of a region's construction tail.
    ///
    /// `region_name` is an initialized region and `record` its checked element
    /// schema. Scope exit restores one saved marker, independent of record count.
    pub(super) fn batch_scope(&mut self, name: &str, region_name: &str, record: &str, body: &[Op]) {
        let region = self.region(region_name);
        let start = storage::top(&mut self.f, &region);
        let metadata = self.f.slot("%holo.batch");
        let region_slot = storage::member(&mut self.f, "%holo.batch", &metadata, 0);
        self.f
            .line(format!("store ptr {region}, ptr {region_slot}"));
        // The start/end/count metadata has fixed size regardless of payload size.
        for (index, value) in [(1, start.as_str()), (2, start.as_str()), (3, "0")] {
            let slot = storage::member(&mut self.f, "%holo.batch", &metadata, index);
            self.f.line(format!("store i64 {value}, ptr {slot}"));
        }
        self.push_scope();
        // The proof visits every append in this owner before emitting any cursor.
        let shape = if self.uniform_batch_extents {
            uniform_batch::prove(self.program, self.layout, name, record, body)
        } else {
            None
        };
        self.bind(
            name,
            Binding::Batch {
                slot: metadata,
                record: record.to_owned(),
                region: region.clone(),
                shape,
            },
        );
        self.cleanups.push(Cleanup::Tail {
            region: region.clone(),
            start: start.clone(),
        });
        self.operations(body);
        self.cleanups.pop().expect("batch cleanup remains active");
        self.pop_scope();
        storage::restore(&mut self.f, &region, &start);
    }

    /// Emit a back edge to an unbounded lexical entry body with no break.
    /// Returning from the body runs ordinary active-owner cleanup. A detached
    /// continuation keeps unreachable following source valid for LLVM.
    pub(super) fn repeat_forever(&mut self, body: &[Op]) {
        let repeat = self.f.fresh_label();
        self.f.line(format!("br label %{repeat}"));
        self.f.label(&repeat);
        self.push_scope();
        self.operations(body);
        self.pop_scope();
        self.f.line(format!("br label %{repeat}"));
        let unreachable = self.f.fresh_label();
        self.f.label(&unreachable);
    }

    /// Emit a counted loop with one evaluation of its unsigned bound.
    ///
    /// `index_name` is a new scalar scoped to each iteration. The index slot is
    /// fixed native storage; body references disappear before the next iteration.
    pub(super) fn repeat(&mut self, count: &Expr, index_name: &str, body: &[Op]) {
        // A proved pure byte sum emits its own exact bounds; all other loops
        // keep the original checked evaluation and iteration order.
        if byte_sum::try_lower(self, count, index_name, body) {
            return;
        }
        if cursor_staging::try_lower(self, count, index_name, body) {
            return;
        }
        // An opted-in staging site emits its guarded fast path and preserves
        // this same scalar loop on any unsupported runtime input.
        if staging_copy::try_lower(self, count, index_name, body) {
            return;
        }
        let count = self.expression(count);
        // The stable-slot trial can capture metadata once outside nested copies.
        if staging_slots::try_repeat(self, &count, index_name, body) {
            return;
        }
        self.repeat_counted(&count, index_name, body);
    }

    /// Emit the original checked loop after its count has been evaluated once.
    ///
    /// `count` is the captured unsigned SSA operand. This path keeps exact byte
    /// write, read, and trap ordering, including on staging guard fallback.
    pub(super) fn repeat_counted(&mut self, count: &str, index_name: &str, body: &[Op]) {
        let index_slot = self.f.slot("i64");
        self.f.line(format!("store i64 0, ptr {index_slot}"));
        let condition = self.f.fresh_label();
        let iteration = self.f.fresh_label();
        let done = self.f.fresh_label();
        self.f.line(format!("br label %{condition}"));
        self.f.label(&condition);
        let index = self.f.value(format!("load i64, ptr {index_slot}"));
        let remaining = self.f.value(format!("icmp ult i64 {index}, {count}"));
        // A finite unsigned count terminates before incrementing beyond its bound.
        self.f.line(format!(
            "br i1 {remaining}, label %{iteration}, label %{done}"
        ));
        self.f.label(&iteration);
        self.push_scope();
        self.bind(index_name, Binding::Scalar(index_slot.clone()));
        self.operations(body);
        self.pop_scope();
        let following = self.f.value(format!("add i64 {index}, 1"));
        self.f
            .line(format!("store i64 {following}, ptr {index_slot}"));
        self.f.line(format!("br label %{condition}"));
        self.f.label(&done);
    }

    /// Emit exactly one branch scope after evaluating its checked condition.
    ///
    /// Writes to outer scalar/reference slots remain observable after the merge;
    /// bindings declared inside a branch do not escape it.
    pub(super) fn branch(&mut self, condition: &Condition, then_body: &[Op], else_body: &[Op]) {
        let condition = self.condition(condition);
        let selected = self.f.fresh_label();
        let alternate = self.f.fresh_label();
        let done = self.f.fresh_label();
        self.f.line(format!(
            "br i1 {condition}, label %{selected}, label %{alternate}"
        ));
        self.f.label(&selected);
        self.push_scope();
        self.operations(then_body);
        self.pop_scope();
        self.f.line(format!("br label %{done}"));
        self.f.label(&alternate);
        self.push_scope();
        self.operations(else_body);
        self.pop_scope();
        self.f.line(format!("br label %{done}"));
        self.f.label(&done);
    }
}
