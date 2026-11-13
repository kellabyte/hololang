//! Entry-operation lowering for scalars, references, and observable data access.
//!
//! Structured lifetime transitions live in `scope`, and destination commit,
//! rollback, and reset live in `publication`. This dispatcher keeps ordinary data
//! operations separate from storage ownership. Access rights and lifetime roots
//! are validated before any native instruction is emitted.

use crate::ir::{Expr, Op};

use super::{
    context::{Binding, Lowering},
    input::Input,
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

    /// Check the byte destination before evaluating and narrowing its replacement.
    fn write_view_byte(&mut self, contents: &str, len: &str, index: &Expr, value: &Expr) {
        let pointer = self.view_byte_pointer(contents, len, index);
        let value = self.expression(value);
        let byte = self.f.value(format!("trunc i64 {value} to i8"));
        self.f
            .line(format!("store i8 {byte}, ptr {pointer}, align 1"));
    }

    /// Move the captured source into an existing destination, including overlap.
    /// Validate the full destination extent before emitting any payload access.
    fn move_input_bytes(&mut self, destination: usize, source: usize) {
        let (destination, capacity) = self.byte_input(destination);
        let (source, length) = self.byte_input(source);
        let valid = self.f.value(format!("icmp ule i64 {length}, {capacity}"));
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
        let same = self.f.value(format!("icmp eq ptr {destination}, {source}"));
        let count = self
            .f
            .value(format!("select i1 {same}, i64 0, i64 {length}"));
        // Walking backward whenever the destination starts higher preserves
        // unread bytes on rightward overlap, even across separate formals.
        let backward = self
            .f
            .value(format!("icmp ugt ptr {destination}, {source}"));
        let entry = self.f.current_label().to_owned();
        let test = self.f.fresh_label();
        let copy = self.f.fresh_label();
        let done = self.f.fresh_label();
        let next = self.f.fresh_value();
        self.f.line(format!("br label %{test}"));
        self.f.label(&test);
        let index = self
            .f
            .value(format!("phi i64 [ 0, %{entry} ], [ {next}, %{copy} ]"));
        let more = self.f.value(format!("icmp ult i64 {index}, {count}"));
        self.f
            .line(format!("br i1 {more}, label %{copy}, label %{done}"));
        self.f.label(&copy);
        let remaining = self.f.value(format!("sub i64 {count}, {index}"));
        let reverse = self.f.value(format!("sub i64 {remaining}, 1"));
        let offset = self
            .f
            .value(format!("select i1 {backward}, i64 {reverse}, i64 {index}"));
        let from = storage::advance(&mut self.f, &source, &offset);
        let to = storage::advance(&mut self.f, &destination, &offset);
        let byte = self.f.value(format!("load i8, ptr {from}, align 1"));
        self.f.line(format!("store i8 {byte}, ptr {to}, align 1"));
        self.f.line(format!("{next} = add i64 {index}, 1"));
        self.f.line(format!("br label %{test}"));
        self.f.label(&done);
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
                source,
                offset,
                count,
                result,
            } => self.send(source, offset, count, result),
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
            Op::LetBool { name, value } => {
                let value = self.boolean(value);
                self.bind(name, Binding::Boolean(value));
            }
            Op::BindBytes {
                name,
                source,
                ranges,
                ..
            } => {
                let Input::Bytes { contents, len } = self.capture_byte_view(source, ranges) else {
                    unreachable!("checked view declaration has a byte source");
                };
                self.bind(name, Binding::ByteView { contents, len });
            }
            Op::TryLet {
                name,
                function,
                args,
            } => self.try_binding(name, function, args),
            Op::TrySaved { name, source } => self.try_saved_binding(name, source),
            Op::BindResult {
                name,
                function,
                args,
            } => {
                let (index, callee) = self.ordinary_definition(function);
                let family = callee.result.clone().expect("checked saved result family");
                let value = self.ordinary_call_resolved(index, callee, args);
                self.bind(name, Binding::SavedResult { value, family });
            }
            Op::HandleResult {
                function,
                args,
                arms,
            } => {
                self.handle_result_blocks(function, args, arms);
            }
            Op::HandleSavedResult { name, arms } => self.handle_saved_result_blocks(name, arms),
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
                let batch = self.binding(batch);
                let index = self.expression(index);
                let (record, pointer) = match batch {
                    Binding::Batch { slot, record, .. } => {
                        let schema = self.record_index(&record);
                        let pointer = self.f.value(format!(
                            "call ptr @holo.record_at.{schema}(ptr {slot}, i64 {index})"
                        ));
                        (record, pointer)
                    }
                    Binding::BorrowedBatch { prefix, record } => {
                        let schema = self.record_index(&record);
                        let pointer = self.f.value(format!(
                            "call ptr @holo.prefix_record_at.{schema}(ptr {}, i64 {}, i64 {index})",
                            prefix.contents, prefix.count
                        ));
                        (record, pointer)
                    }
                    _ => unreachable!("checked indexed reference requires a batch"),
                };
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
            Op::WriteInputByte {
                param,
                index,
                value,
            } => {
                let (contents, len) = self.byte_input(*param);
                self.write_view_byte(&contents, &len, index, value);
            }
            Op::WriteViewByte { name, index, value } => {
                let (contents, len) = self.local_bytes(name);
                self.write_view_byte(&contents, &len, index, value);
            }
            Op::MoveInputBytes {
                destination,
                source,
            } => {
                self.move_input_bytes(*destination, *source);
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
            Op::ReturnBool(expression) => {
                let value = self.boolean(expression);
                self.f.line(format!("ret i1 {value}"));
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
