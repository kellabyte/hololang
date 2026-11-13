//! Destination call commit/rollback, scoped records, and batch reset lowering.
//!
//! A producer only writes an unpublished suffix. The append boundary snapshots
//! one marker and invokes the final-destination producer. Success publishes
//! the new count/end and commits deferred advances. Failure restores the
//! marker only for a producer that can change it. A scoped record lends its final
//! address to one success arm and restores the marker on exit. These operations
//! have fixed metadata work, while propagation can cost work per producer frame.

use crate::ir::{AppendArm, AppendPattern, Argument, ConstructArm};

use super::{
    context::{Binding, Cleanup, Lowering},
    function::Function,
    producer::{self, Placement, ProducerReturn},
    storage,
};

/// Compiler-owned operands for one attempt at an already selected destination.
struct DestinationCall {
    /// Final address passed to the producer before evaluating its actuals.
    destination: String,
    /// Original checked region cursor before construction began.
    start: String,
    /// Complete native result, with the second lane active only when applicable.
    returned: String,
    /// Native outcome discriminator used for publication and handler dispatch.
    status: String,
    /// Checked producer ABI, including whether it mutates the marker.
    placement: Placement,
    /// Callee-specific branch layout hint for checked append.
    prefers_success: bool,
}

impl DestinationCall {
    /// Commit a checked success and return the end if this call committed it.
    ///
    /// Marked producers already advanced the initialized marker. A batch may
    /// subsequently need to read it, while a scoped record needs only its address.
    fn commit(&self, f: &mut Function, region: &str) -> Option<String> {
        // Fixed and deferred results already checked every constituent reservation.
        match self.placement {
            Placement::Fixed(bytes) => Some(storage::commit_fixed(f, region, &self.start, bytes)),
            Placement::Deferred => {
                let end = f.value(format!("extractvalue {{ i64, i64 }} {}, 1", self.returned));
                storage::restore(f, region, &end);
                Some(end)
            }
            Placement::Marked => None,
        }
    }

    /// Restore a failed attempt only if its ABI can have advanced the marker.
    ///
    /// No published value points into this private suffix. Failed cursor-based
    /// calls may have written private bytes but have left the marker unchanged.
    fn rollback(&self, f: &mut Function, region: &str) {
        if self.placement == Placement::Marked {
            storage::restore(f, region, &self.start);
        }
    }
}

impl Lowering<'_> {
    /// Attempt an append and introduce its unsigned status binding.
    ///
    /// `batch_name` identifies the checked destination; `producer` its compatible
    /// schema constructor; `args` evaluate in order; `status_name` is a new scalar.
    /// Success returns status 0. Failure preserves all previously published bytes,
    /// references, count, and end without a per-record or per-field rollback walk.
    pub(super) fn append(
        &mut self,
        batch_name: &str,
        producer: &str,
        args: &[Argument],
        status_name: &str,
    ) {
        let (status, _) = self.append_outcome(batch_name, producer, args);
        let slot = self.f.slot("i64");
        self.f.line(format!("store i64 {status}, ptr {slot}"));
        self.bind(status_name, Binding::Scalar(slot));
    }

    /// Run the common append and merge the selected handler's scalar in SSA.
    ///
    /// Each arm can observe the receiving batch only after the shared outcome
    /// helper publishes or restores it. The payload lane is read only on an
    /// active rejection edge that binds it. No native slot stores the result.
    pub(super) fn append_handled(
        &mut self,
        batch: &str,
        producer: &str,
        args: &[Argument],
        result_name: &str,
        arms: &[AppendArm],
    ) {
        let (status, returned) = self.append_outcome(batch, producer, args);
        let join = self.f.fresh_label();
        let labels = self.destination_arms(&status, arms.iter().map(|arm| &arm.pattern));
        let mut incoming = Vec::with_capacity(arms.len());
        // Enter only the selected arm at runtime; compile each with its own
        // scalar binding. An ignored error never forces a payload extraction.
        for (arm, label) in arms.iter().zip(&labels) {
            self.f.label(label);
            self.push_scope();
            self.bind_rejection(&arm.pattern, &returned);
            let value = self.expression(&arm.value);
            self.pop_scope();
            // A nested result handler can finish in its own merge block.
            incoming.push(format!("[ {value}, %{} ]", self.f.current_label()));
            self.f.line(format!("br label %{join}"));
        }
        self.f.label(&join);
        let value = self.f.value(format!("phi i64 {}", incoming.join(", ")));
        self.bind(result_name, Binding::ScalarValue(value));
    }

    /// Construct one typed local directly into the checked region's free tail.
    ///
    /// The selected success arm receives the final address in SSA and owns one
    /// active marker cleanup. Failure arms run after rollback and may reuse the
    /// region. No aggregate temporary, batch metadata, or indexed lookup is used.
    pub(super) fn construct_handled(
        &mut self,
        region: &str,
        producer: &str,
        args: &[Argument],
        arms: &[ConstructArm],
    ) {
        let region = self.region(region);
        let attempt = self.call_destination_producer(&region, producer, args, true);
        let record = &self.program.producers[self.producer_index(producer)].record;
        let join = self.f.fresh_label();
        let labels = self.destination_arms(&attempt.status, arms.iter().map(|arm| &arm.pattern));
        // Each path commits or rolls back before any user operations. Marked
        // failure arms each emit one restore, avoiding a second runtime dispatch.
        for (arm, label) in arms.iter().zip(&labels) {
            self.f.label(label);
            self.push_scope();
            let success = matches!(arm.pattern, AppendPattern::Ok);
            if success {
                let _ = attempt.commit(&mut self.f, &region);
                self.cleanups.push(Cleanup::Tail {
                    region: region.clone(),
                    start: attempt.start.clone(),
                });
                // A discarded result still owns the tail for the complete arm.
                if let Some(name) = &arm.reference {
                    self.bind(
                        name,
                        Binding::ScopedReference {
                            pointer: attempt.destination.clone(),
                            record: record.clone(),
                        },
                    );
                }
            } else {
                attempt.rollback(&mut self.f, &region);
                self.bind_rejection(&arm.pattern, &attempt.returned);
            }
            self.operations(&arm.body);
            self.pop_scope();
            // Entry return emits its active cleanup and leaves an unreachable
            // continuation. The normal arm exit below covers other paths.
            if success {
                self.cleanups
                    .pop()
                    .expect("scoped record cleanup is active");
                storage::restore(&mut self.f, &region, &attempt.start);
            }
            self.f.line(format!("br label %{join}"));
        }
        self.f.label(&join);
    }

    /// Emit the common destination status switch and return its ordered arm labels.
    ///
    /// The SSA discriminator is valid for a checked exhaustive partition.
    /// Payload extraction is deferred to the active handler.
    fn destination_arms<'p>(
        &mut self,
        status: &str,
        patterns: impl ExactSizeIterator<Item = &'p AppendPattern> + DoubleEndedIterator + Clone,
    ) -> Vec<String> {
        let labels: Vec<_> = (0..patterns.len()).map(|_| self.f.fresh_label()).collect();
        // A final fallback owns all unlisted errors. Otherwise the checked
        // signature and exhaustive patterns make the default edge unreachable.
        let fallback = patterns
            .clone()
            .next_back()
            .is_some_and(|pattern| matches!(pattern, AppendPattern::Fallback(_)));
        let default = if fallback {
            labels.last().expect("fallback has a block").clone()
        } else {
            self.f.fresh_label()
        };
        self.f
            .line(format!("switch i64 {status}, label %{default} ["));
        // All named outcomes are checked and disjoint before encoding.
        for (pattern, label) in patterns.zip(&labels) {
            let code = match pattern {
                AppendPattern::Ok => 0,
                AppendPattern::Full => 1,
                AppendPattern::SizeOverflow => 2,
                AppendPattern::Rejected { family, case, .. } => {
                    self.append_error_status(family, case)
                }
                AppendPattern::Fallback(_) => continue,
            };
            self.f.line(format!("  i64 {code}, label %{label}"));
        }
        self.f.line("]");
        // Any other status is impossible for a checked exhaustive typed call.
        if !fallback {
            self.f.label(&default);
            self.f.line("unreachable");
        }
        labels
    }

    /// Bind the active scalar payload for a selected, explicitly bound rejection.
    /// Other statuses do not need to read the producer's second lane.
    fn bind_rejection(&mut self, pattern: &AppendPattern, returned: &str) {
        if let AppendPattern::Rejected {
            binding: Some(name),
            ..
        } = pattern
        {
            let value = self
                .f
                .value(format!("extractvalue {{ i64, i64 }} {returned}, 1"));
            self.bind(name, Binding::ScalarValue(value));
        }
    }

    /// Call the checked producer, commit or roll back once, and return its lanes.
    ///
    /// The status drives publication. The original call result remains
    /// available after the join, outside the discarded private region suffix.
    /// The numeric wrapper keeps its original scalar status slot.
    fn append_outcome(
        &mut self,
        batch_name: &str,
        producer: &str,
        args: &[Argument],
    ) -> (String, String) {
        let (batch, _) = self.batch(batch_name);
        let region_slot = storage::member(&mut self.f, "%holo.batch", &batch, 0);
        let region = self.f.value(format!("load ptr, ptr {region_slot}"));
        let attempt = self.call_destination_producer(&region, producer, args, false);
        let success = self.f.value(format!("icmp eq i64 {}, 0", attempt.status));
        let published = self.f.fresh_label();
        let failed = self.f.fresh_label();
        let done = self.f.fresh_label();
        // Only a fully initialized result can advance visible batch metadata.
        self.f.construction_branch(
            &success,
            &published,
            &failed,
            attempt.prefers_success.then_some(true),
        );
        self.f.label(&published);
        let end = attempt
            .commit(&mut self.f, &region)
            .unwrap_or_else(|| storage::top(&mut self.f, &region));
        let end_slot = storage::member(&mut self.f, "%holo.batch", &batch, 2);
        self.f.line(format!("store i64 {end}, ptr {end_slot}"));
        let count_slot = storage::member(&mut self.f, "%holo.batch", &batch, 3);
        let old_count = self.f.value(format!("load i64, ptr {count_slot}"));
        // Each published record occupies at least one backing byte, so a
        // finite u64 byte region cannot publish enough records to wrap this count.
        let count = self.f.value(format!("add i64 {old_count}, 1"));
        self.f.line(format!("store i64 {count}, ptr {count_slot}"));
        self.f.line(format!("br label %{done}"));
        self.f.label(&failed);
        attempt.rollback(&mut self.f, &region);
        self.f.line(format!("br label %{done}"));
        self.f.label(&done);
        (attempt.status, attempt.returned)
    }

    /// Resolve a region tail and invoke the matching checked producer ABI.
    ///
    /// The saved marker and final address dominate eager actuals and dispatch.
    /// Scoped construction reuses the snapshot for its address. Batch append
    /// keeps the previous marked-mode load sequence for exact code identity.
    fn call_destination_producer(
        &mut self,
        region: &str,
        producer: &str,
        args: &[Argument],
        reuse_saved: bool,
    ) -> DestinationCall {
        let start = storage::top(&mut self.f, region);
        let producer = self.producer_index(producer);
        let definition = &self.program.producers[producer];
        let returned = ProducerReturn::for_producer(definition);
        let placement = producer::placement(definition, self.layout);
        let prefers_success = producer::prefers_success(definition);
        // Cursor-based producers take the same cursor that establishes the
        // receiving address. Marker-based producers retain their original ABI.
        let destination = if reuse_saved || placement != Placement::Marked {
            storage::address_at(&mut self.f, region, &start)
        } else {
            storage::tail(&mut self.f, region)
        };
        let mut call = format!("call {} @holo.producer.{producer}(", returned.llvm_type());
        // Scalar/borrow actuals are captured only after resolving the receiving slot.
        let arguments = self.call_arguments(args);
        // The cursor-based graph inherits capacity without reloading after writes.
        // This reads initialized metadata only; every producer still compares
        // the bound after its own guard and checked addition.
        if placement != Placement::Marked {
            let capacity = storage::load_capacity(&mut self.f, region);
            call.push_str(&format!("ptr {destination}, i64 {start}, i64 {capacity}"));
        } else {
            call.push_str(&format!("ptr {region}, ptr {destination}"));
        }
        call.push_str(&arguments);
        call.push(')');
        let outcome = self.f.value(call);
        let status = returned.status(&mut self.f, &outcome);
        DestinationCall {
            destination,
            start,
            returned: outcome,
            status,
            placement,
            prefers_success,
        }
    }

    /// Reset a checked batch after all references into it have become unusable.
    ///
    /// `batch_name` resolves to live owner metadata. The checker established the
    /// reference precondition; native work is a fixed set of marker/count stores.
    pub(super) fn reset(&mut self, batch_name: &str) {
        let (batch, _) = self.batch(batch_name);
        let region_slot = storage::member(&mut self.f, "%holo.batch", &batch, 0);
        let region = self.f.value(format!("load ptr, ptr {region_slot}"));
        let start_slot = storage::member(&mut self.f, "%holo.batch", &batch, 1);
        let start = self.f.value(format!("load i64, ptr {start_slot}"));
        storage::restore(&mut self.f, &region, &start);
        let end_slot = storage::member(&mut self.f, "%holo.batch", &batch, 2);
        self.f.line(format!("store i64 {start}, ptr {end_slot}"));
        let count_slot = storage::member(&mut self.f, "%holo.batch", &batch, 3);
        self.f.line(format!("store i64 0, ptr {count_slot}"));
    }
}

#[cfg(test)]
mod tests {
    use crate::{frontend::parse_experimental, layout::LayoutMode};

    /// Inspect the reachable early exits in both destination ABI paths.
    ///
    /// The receiver must be the producer operand. A success exit restores one
    /// marker before freeing, and a rejected arm restores only for Marked.
    #[test]
    fn scoped_record_early_exits_restore_only_the_live_checkpoint() {
        let source = r#"
struct Ticket { id u64 }
error IssueError { Denied }
func issue arg id u64 as readonly copy returns Result<Ticket, IssueError> {
    if id > 20 { return error IssueError.Denied }
    return ok Ticket:
        id = id
}
func main returns u64 {
    let memory = core.region(16) else { return 0 }
    core.construct(memory, issue(9))
        on ok(ticket) then {
            core.print(ticket.id)
            return 0
        }
        on error then {
            core.print(77)
            return 0
        }
    return 0
}
"#;
        let parsed = parse_experimental(source).expect("scoped fixture lowers");
        let checked = parsed.check().expect("scoped fixture checks");
        // Wide mutates the marker in the producer. Derived64/Compact32 pass a
        // fixed cursor and commit it on success, with no failed marker change.
        for layout in [
            LayoutMode::Wide,
            LayoutMode::Derived64,
            LayoutMode::Compact32,
        ] {
            let module =
                super::super::emit_with_layout(&checked, "arm64-apple-macosx11.0.0", layout)
                    .expect("checked fixture layout fits");
            let (_, entry) = module.split_once("define i32 @main()").unwrap();
            let (entry, _) = entry.split_once("\n}").unwrap();
            let lines: Vec<_> = entry.lines().map(str::trim).collect();
            // Match a marker address immediately loaded to an SSA checkpoint.
            // Capacity, initialization, and rewinds do not match that sequence.
            let checkpoints: Vec<_> = lines
                .windows(2)
                .filter_map(|pair| {
                    let (slot, address) = pair[0].split_once(" = getelementptr %holo.region,")?;
                    let (value, loaded) = pair[1].split_once(" = load i64, ptr ")?;
                    (address.ends_with(", i32 0, i32 2") && loaded == slot).then_some(value)
                })
                .collect();
            let [checkpoint] = checkpoints.as_slice() else {
                panic!("expected one saved marker for {layout:?}: {checkpoints:?}");
            };
            let restore = format!("store i64 {checkpoint}, ptr ");
            let call = lines
                .iter()
                .find(|line| line.contains("call { i64, i64 } @holo.producer."))
                .expect("direct producer call");
            let (_, operands) = call.split_once('(').unwrap();
            let mut operands = operands.split(", ");
            // Marked passes the region before the destination. Cursor-based
            // producers take the final destination as their first argument.
            let destination = if layout == LayoutMode::Wide {
                operands.nth(1)
            } else {
                operands.next()
            }
            .and_then(|operand| operand.strip_prefix("ptr "))
            .expect("receiving address operand");
            let success = entry
                .split("\nblock")
                .find(|block| block.contains("@holo.number_format, i64 %"))
                .expect("success print and cleanup block");
            let failure = entry
                .split("\nblock")
                .find(|block| block.contains("@holo.number_format, i64 77)"))
                .expect("fallback print and cleanup block");
            assert!(success.contains(&format!("getelementptr i8, ptr {destination}, i64 ")));
            assert!(!entry.contains("alloca ptr"));
            assert_eq!(success.matches(&restore).count(), 1);
            assert!(success.find(&restore).unwrap() < success.find("@free").unwrap());
            // Only the Wide rejection has a marker to restore. Its restore
            // must run before user code, including an early failure exit.
            if layout == LayoutMode::Wide {
                assert_eq!(failure.matches(&restore).count(), 1);
                assert!(failure.find(&restore).unwrap() < failure.find("@printf").unwrap());
            } else {
                assert!(!failure.contains(&restore));
            }
            assert!(failure.contains("@free"));
        }
    }
}
