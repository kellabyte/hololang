//! Ordered scalar and field-observation lowering for checked region programs.
//!
//! User arithmetic in this compiler IR is explicitly modulo 2^64; storage sizes
//! use separate checked reservation helpers. Reference observations load actual
//! memory on each evaluation and carry no immutability or uniqueness annotation.

use crate::ir::{Binary, Compare, Condition, Expr};

use super::{
    context::{Binding, Lowering},
    input::Input,
    storage,
};

impl Lowering<'_> {
    /// Emit one unsigned expression and return its i64 constant or SSA name.
    ///
    /// `expression` is already type/root checked. Children are emitted left first,
    /// preserving alias-visible evaluation order; no source storage is copied.
    pub(super) fn expression(&mut self, expression: &Expr) -> String {
        // Every branch returns the native unsigned representation of one IR form.
        match expression {
            Expr::Const(value) => value.to_string(),
            Expr::Apply { args, body } => {
                let mut captured = Vec::with_capacity(args.len());
                // All actuals evaluate once before replacing the enclosing frame;
                // an unused invalid byte read must still trap at this point.
                for argument in args {
                    captured.push(Input::Scalar(self.expression(argument)));
                }
                let previous = std::mem::replace(&mut self.params, captured);
                let previous_index = self.byte_index.take();
                let value = self.expression(body);
                self.params = previous;
                self.byte_index = previous_index;
                value
            }
            Expr::Call { function, args } => self.ordinary_call(function, args),
            Expr::Handle {
                function,
                args,
                arms,
            } => self.handle_result(function, args, arms),
            Expr::Local(name) => match self.binding(name) {
                Binding::Scalar(slot) => self.f.value(format!("load i64, ptr {slot}")),
                Binding::ScalarValue(value) => value,
                _ => panic!("checked scalar has wrong lowering kind: {name}"),
            },
            Expr::Param(index) => match &self.params[*index] {
                Input::Scalar(value) => value.clone(),
                Input::Bytes { .. } | Input::State(_) => {
                    panic!("checked scalar formal has wrong lowering kind")
                }
            },
            Expr::InputLen(param) => self.byte_input(*param).1,
            Expr::InputByte { param, index } => {
                let (contents, len) = self.byte_input(*param);
                let index = self.expression(index);
                let valid = self.f.value(format!("icmp ult i64 {index}, {len}"));
                // Bounds failure traps before pointer formation or any payload load.
                self.f.line(format!("call void @holo.assert(i1 {valid})"));
                let pointer = storage::advance(&mut self.f, &contents, &index);
                let byte = self.f.value(format!("load i8, ptr {pointer}, align 1"));
                self.f.value(format!("zext i8 {byte} to i64"))
            }
            Expr::Index => self.byte_index.clone().expect("checked byte index scope"),
            Expr::Binary { op, left, right } => {
                let left = self.expression(left);
                let right = self.expression(right);
                let opcode = match op {
                    Binary::AddWrapping => "add",
                    Binary::SubWrapping => "sub",
                    Binary::MulWrapping => "mul",
                    Binary::Xor => "xor",
                    Binary::And => "and",
                };
                self.f.value(format!("{opcode} i64 {left}, {right}"))
            }
            Expr::Read { reference, path } => {
                let pointer = self.field_pointer(reference, path);
                storage::load_u64(&mut self.f, &pointer)
            }
            Expr::Byte {
                reference,
                path,
                index,
            } => {
                let header = self.field_pointer(reference, path);
                let index = self.expression(index);
                let extent = self.known_byte_extent(reference);
                let pointer = self.byte_pointer(&header, &index, extent);
                let byte = self.f.value(format!("load i8, ptr {pointer}, align 1"));
                self.f.value(format!("zext i8 {byte} to i64"))
            }
            Expr::BytesLen { reference, path } => {
                // The proved cursor still refers to initialized storage, and a
                // length projection by itself has no payload access or trap.
                match self.known_byte_extent(reference) {
                    Some(extent) => extent.to_string(),
                    None => {
                        let pointer = self.field_pointer(reference, path);
                        self.byte_length(&pointer)
                    }
                }
            }
            Expr::BatchLen(name) => {
                let (batch, _) = self.batch(name);
                let slot = storage::member(&mut self.f, "%holo.batch", &batch, 3);
                self.f.value(format!("load i64, ptr {slot}"))
            }
            Expr::Used(name) => {
                let region = self.region(name);
                storage::top(&mut self.f, &region)
            }
            Expr::Address(name) => {
                let (pointer, _) = self.reference(name);
                self.f.value(format!("ptrtoint ptr {pointer} to i64"))
            }
            Expr::Nanoseconds => self.f.value("call i64 @holo_now()"),
        }
    }

    /// Emit a checked unsigned comparison, returning an i1 SSA name.
    ///
    /// The left operand is evaluated before the right, preserving observable reads.
    pub(super) fn condition(&mut self, condition: &Condition) -> String {
        let left = self.expression(&condition.left);
        let right = self.expression(&condition.right);
        let predicate = match condition.op {
            Compare::Eq => "eq",
            Compare::Ne => "ne",
            Compare::Lt => "ult",
            Compare::Le => "ule",
            Compare::Gt => "ugt",
            Compare::Ge => "uge",
        };
        self.f
            .value(format!("icmp {predicate} i64 {left}, {right}"))
    }

    /// Bounds-check an initialized payload index and return its byte address.
    ///
    /// `header` names frozen length metadata and `index` is unsigned. An exact
    /// proved extent can supply the bound instead of reading its metadata.
    /// Failure terminates before any element access; success allocates nothing.
    pub(super) fn byte_pointer(
        &mut self,
        header: &str,
        index: &str,
        extent: Option<u64>,
    ) -> String {
        // Both paths check the same initialized bound before forming the address.
        let length = match extent {
            Some(extent) => extent.to_string(),
            None => self.byte_length(header),
        };
        let valid = self.f.value(format!("icmp ult i64 {index}, {length}"));
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
        let width = self.layout.length_bytes().to_string();
        let contents = storage::advance(&mut self.f, header, &width);
        storage::advance(&mut self.f, &contents, index)
    }
}
