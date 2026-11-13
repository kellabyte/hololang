//! Entry-operation lowering for scalars, references, and observable data access.
//!
//! Structured lifetime transitions live in `scope`, and destination commit,
//! rollback, and reset live in `publication`. This dispatcher keeps ordinary data
//! operations separate from storage ownership. Access rights and lifetime roots
//! are validated before any native instruction is emitted.

use crate::ir::Op;

use super::{
    context::{Binding, Lowering},
    storage,
};

impl Lowering<'_> {
    /// Emit ordered checked operations in the current lexical scope.
    ///
    /// `operations` borrows immutable compiler IR. Native state changes are emitted
    /// in order; source object destruction is never inferred from this host loop.
    pub(super) fn operations(&mut self, operations: &[Op]) {
        // The IR sequence establishes the required observable evaluation order.
        for operation in operations {
            self.operation(operation);
        }
    }

    /// Emit one checked operation, delegating ownership/control-flow transitions.
    ///
    /// No returned native value is needed: explicit scalar/status bindings hold
    /// operation results. Host failures here indicate violated checker invariants.
    fn operation(&mut self, operation: &Op) {
        // Each arm lowers a distinct typed operation without weakening checks.
        match operation {
            // Declarations are emitted as separate definitions, never entry instructions.
            Op::Function(_) | Op::ErrorFamily(_) => {}
            Op::TcpScope { body, .. } => self.tcp_scope(body),
            // The checker resolves the sole live owner. Named waits use its
            // selected readiness queue; sends and receives keep their range path.
            Op::TcpIo { operation, .. } => match operation.as_ref() {
                Op::WaitInput { result } => self.wait_tcp(result, false),
                Op::WaitOutput { result } => self.wait_tcp(result, true),
                _ => self.operation(operation),
            },
            Op::TcpClose { result, .. } => self.close_tcp(result),
            Op::Refill(refill) => self.refill(refill),
            Op::WaitInput { result } => self.wait_input(result),
            Op::Send {
                reference,
                path,
                offset,
                count,
                result,
            } => self.send(reference, path, offset, count, result),
            Op::WaitOutput { result } => self.wait_output(result),
            Op::NextSession { result } => self.next_session(result),
            Op::Loop(body) => self.repeat_forever(body),
            Op::Region {
                name,
                capacity,
                fail_acquire,
                body,
                on_failure,
            } => {
                self.region_scope(name, capacity, *fail_acquire, body, on_failure);
            }
            Op::Batch {
                name,
                region,
                record,
                body,
            } => {
                self.batch_scope(name, region, record, body);
            }
            Op::Scope(body) => {
                self.push_scope();
                self.operations(body);
                self.pop_scope();
            }
            Op::Let { name, value } => {
                let value = self.expression(value);
                let slot = self.f.slot("i64");
                self.f.line(format!("store i64 {value}, ptr {slot}"));
                self.bind(name, Binding::Scalar(slot));
            }
            Op::TryLet {
                name,
                function,
                args,
            } => self.try_binding(name, function, args),
            Op::Set { name, value } => {
                let value = self.expression(value);
                match self.binding(name) {
                    Binding::Scalar(slot) => self.f.line(format!("store i64 {value}, ptr {slot}")),
                    _ => panic!("checked assignment target is not scalar: {name}"),
                }
            }
            Op::Append {
                batch,
                producer,
                args,
                status,
            } => {
                self.append(batch, producer, args, status);
            }
            Op::AppendHandled {
                batch,
                producer,
                args,
                result,
                arms,
            } => self.append_handled(batch, producer, args, result, arms),
            Op::ConstructHandled {
                region,
                producer,
                args,
                arms,
            } => self.construct_handled(region, producer, args, arms),
            Op::Borrow {
                name, batch, index, ..
            } => {
                let origin = self.batch_origin(batch);
                let (batch, record) = self.batch(batch);
                let index = self.expression(index);
                let schema = self.record_index(&record);
                let pointer = self.f.value(format!(
                    "call ptr @holo.record_at.{schema}(ptr {batch}, i64 {index})"
                ));
                let slot = self.f.slot("ptr");
                self.f.line(format!("store ptr {pointer}, ptr {slot}"));
                self.bind_reference(name, slot, record, origin, None);
            }
            Op::Alias { name, source, .. } => {
                // An explicit alias captures its own rebindable slot. A scoped
                // source supplies its SSA address with no batch staging proof.
                let (pointer, record, origin) = match self.binding(source) {
                    Binding::Reference {
                        slot,
                        record,
                        region,
                        ..
                    } => {
                        let pointer = self.f.value(format!("load ptr, ptr {slot}"));
                        (pointer, record, region)
                    }
                    Binding::ScopedReference { pointer, record } => (pointer, record, None),
                    _ => unreachable!("checked alias requires a reference"),
                };
                let slot = self.f.slot("ptr");
                self.f.line(format!("store ptr {pointer}, ptr {slot}"));
                self.bind_reference(name, slot, record, origin, None);
            }
            Op::AssignRef { name, source } => match self.binding(name) {
                Binding::Reference { slot, .. } => {
                    let (pointer, _) = self.reference(source);
                    self.f.line(format!("store ptr {pointer}, ptr {slot}"))
                }
                _ => panic!("checked reference assignment has wrong target: {name}"),
            },
            Op::Write {
                reference,
                path,
                value,
            } => {
                let pointer = self.field_pointer(reference, path);
                let value = self.expression(value);
                storage::store_u64(&mut self.f, &pointer, &value);
            }
            Op::WriteByte {
                reference,
                path,
                index,
                value,
            } => {
                let header = self.field_pointer(reference, path);
                let index = self.expression(index);
                let extent = self.known_byte_extent(reference);
                let pointer = self.byte_pointer(&header, &index, extent);
                let value = self.expression(value);
                let byte = self.f.value(format!("trunc i64 {value} to i8"));
                self.f
                    .line(format!("store i8 {byte}, ptr {pointer}, align 1"));
            }
            Op::Reset { batch } => self.reset(batch),
            Op::Repeat { count, index, body } => self.repeat(count, index, body),
            Op::ForEach {
                batch,
                reference,
                body,
                ..
            } => self.for_each(batch, reference, body),
            Op::If {
                condition,
                then_body,
                else_body,
            } => {
                self.branch(condition, then_body, else_body);
            }
            Op::Assert(condition) => {
                let condition = self.condition(condition);
                self.f
                    .line(format!("call void @holo.assert(i1 {condition})"));
            }
            Op::Print(expression) => {
                let value = self.expression(expression);
                self.f.value(format!(
                    "call i32 (ptr, ...) @printf(ptr @holo.number_format, i64 {value})"
                ));
            }
            Op::PrintText(text) => {
                // Each checked site gets an immutable module constant. puts
                // writes its bytes and adds one newline without format parsing.
                let index = self.text_literals.len();
                self.text_literals.push(text.clone());
                self.f
                    .line(format!("call i32 @puts(ptr @holo.print_text.{index})"));
            }
            Op::Return(expression) => {
                let value = self.expression(expression);
                self.f.line(format!("ret i64 {value}"));
                // A checked ordinary body owns no region or marker cleanup. A
                // predecessor-free block lets structured siblings finish normally.
                let continuation = self.f.fresh_label();
                self.f.label(&continuation);
            }
            Op::ReturnResult(result) => self.return_result(result),
            Op::ExitZero => self.exit_zero(),
            Op::OpaqueRegion(name) => {
                let region = self.region(name);
                let base_slot = storage::member(&mut self.f, "%holo.region", &region, 0);
                let base = self.f.value(format!("load ptr, ptr {base_slot}"));
                self.f.line(format!("call void @holo_opaque(ptr {base})"));
            }
        }
    }
}
