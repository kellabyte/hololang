//! Ordered scalar and field-observation lowering for checked region programs.
//!
//! User arithmetic in this compiler IR is explicitly modulo 2^64; storage sizes
//! use separate checked reservation helpers. Reference observations load actual
//! memory on each evaluation and carry no immutability or uniqueness annotation.

use crate::ir::{Binary, BoolExpr, Compare, Condition, EnumExpr, Expr};

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
            Expr::Bool(_) => unreachable!("checked boolean is not an unsigned expression"),
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
            Expr::HandleSavedResult { name, arms } => self.handle_saved_result(name, arms),
            Expr::Local(name) => match self.binding(name) {
                Binding::Scalar(slot) => self.f.value(format!("load i64, ptr {slot}")),
                Binding::ScalarValue(value) => value,
                _ => panic!("checked scalar has wrong lowering kind: {name}"),
            },
            Expr::Param(index) => match &self.params[*index] {
                Input::Scalar(value) => value.clone(),
                Input::Boolean(_) | Input::Bytes { .. } | Input::State(_) | Input::Batch(_) => {
                    panic!("checked scalar formal has wrong lowering kind")
                }
            },
            Expr::InputLen(param) => self.byte_input(*param).1,
            Expr::InputByte { param, index } => {
                let (contents, len) = self.byte_input(*param);
                self.read_view_byte(&contents, &len, index)
            }
            Expr::LiteralByte { bytes, index } => {
                let (contents, len) = self.byte_literals.capture(bytes);
                self.read_view_byte(&contents, &len, index)
            }
            Expr::ViewLen(name) => match self.binding(name) {
                Binding::ByteView { len, .. } | Binding::Received { len, .. } => len,
                _ => unreachable!("checked view length has a live byte view"),
            },
            Expr::ViewByte { name, index } => {
                let (contents, len) = self.local_bytes(name);
                self.read_view_byte(&contents, &len, index)
            }
            Expr::Index => self.byte_index.clone().expect("checked byte index scope"),
            Expr::Binary { op, left, right } => {
                let left = self.expression(left);
                let right = self.expression(right);
                let opcode = match op {
                    Binary::AddChecked => return self.checked_arithmetic("uadd", &left, &right),
                    Binary::SubChecked => return self.checked_arithmetic("usub", &left, &right),
                    Binary::MulChecked => return self.checked_arithmetic("umul", &left, &right),
                    Binary::DivChecked | Binary::RemChecked => {
                        let nonzero = self.f.value(format!("icmp ne i64 {right}, 0"));
                        self.f.line(format!("call void @holo.assert(i1 {nonzero})"));
                        if matches!(op, Binary::DivChecked) {
                            "udiv"
                        } else {
                            "urem"
                        }
                    }
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
            Expr::BatchLen(name) => self.batch_count(name),
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

    /// Emit one unsigned arithmetic intrinsic and trap before exposing an overflowed result.
    fn checked_arithmetic(&mut self, intrinsic: &str, left: &str, right: &str) -> String {
        let result = self.f.value(format!(
            "call {{ i64, i1 }} @llvm.{intrinsic}.with.overflow.i64(i64 {left}, i64 {right})"
        ));
        let overflow = self
            .f
            .value(format!("extractvalue {{ i64, i1 }} {result}, 1"));
        let valid = self.f.value(format!("xor i1 {overflow}, true"));
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
        self.f
            .value(format!("extractvalue {{ i64, i1 }} {result}, 0"))
    }

    /// Read a byte through a captured descriptor after checking its logical extent.
    fn read_view_byte(&mut self, contents: &str, len: &str, index: &Expr) -> String {
        let pointer = self.view_byte_pointer(contents, len, index);
        let byte = self.f.value(format!("load i8, ptr {pointer}, align 1"));
        self.f.value(format!("zext i8 {byte} to i64"))
    }

    /// Evaluate and check an index against a captured byte view before forming its address.
    pub(super) fn view_byte_pointer(&mut self, contents: &str, len: &str, index: &Expr) -> String {
        let index = self.expression(index);
        let valid = self.f.value(format!("icmp ult i64 {index}, {len}"));
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
        storage::advance(&mut self.f, contents, &index)
    }

    /// Evaluate one checked boolean, returning an i1 constant or SSA name.
    pub(super) fn boolean(&mut self, expression: &BoolExpr) -> String {
        match expression {
            BoolExpr::Const(value) => value.to_string(),
            BoolExpr::Read { reference, path } => {
                let pointer = self.field_pointer(reference, path);
                storage::load_bool(&mut self.f, &pointer)
            }
            BoolExpr::Local(name) => match self.binding(name) {
                Binding::Boolean(value) => value,
                Binding::BooleanSlot(slot) => self.f.value(format!("load i1, ptr {slot}")),
                _ => unreachable!("checked boolean local"),
            },
            BoolExpr::Call { function, args } => self.ordinary_call(function, args),
            BoolExpr::Compare(condition) => self.condition(condition),
            BoolExpr::EnumEq { left, right } => {
                let left = self.enumeration(left);
                let right = self.enumeration(right);
                self.f.value(format!("icmp eq i8 {left}, {right}"))
            }
            BoolExpr::And(left, right) => self.short_circuit(left, right, false),
            BoolExpr::Or(left, right) => self.short_circuit(left, right, true),
            BoolExpr::Not(value) => {
                let value = self.boolean(value);
                self.f.value(format!("xor i1 {value}, true"))
            }
        }
    }

    /// Lower a checked enum value to a private i8 discriminant.
    pub(super) fn enumeration(&mut self, expression: &EnumExpr) -> String {
        match expression {
            EnumExpr::Variant { kind, variant } => self.enums[kind.as_str()]
                .variants
                .iter()
                .position(|case| case == variant)
                .expect("checked enum case")
                .to_string(),
            EnumExpr::Local(name) => match self.binding(name) {
                Binding::Enum(value) => value,
                Binding::EnumSlot(slot) => self.f.value(format!("load i8, ptr {slot}, align 1")),
                _ => unreachable!("checked enum local"),
            },
            EnumExpr::Read { reference, path } => {
                let pointer = self.field_pointer(reference, path);
                self.f.value(format!("load i8, ptr {pointer}, align 1"))
            }
        }
    }

    /// Evaluate the right operand only when the left differs from the decisive value.
    fn short_circuit(&mut self, left: &BoolExpr, right: &BoolExpr, decisive: bool) -> String {
        let left = self.boolean(left);
        let skip_from = self.f.current_label().to_owned();
        let evaluate = self.f.fresh_label();
        let done = self.f.fresh_label();
        let (if_true, if_false) = if decisive {
            (&done, &evaluate)
        } else {
            (&evaluate, &done)
        };
        self.f
            .line(format!("br i1 {left}, label %{if_true}, label %{if_false}"));
        self.f.label(&evaluate);
        let right = self.boolean(right);
        let evaluated_from = self.f.current_label().to_owned();
        self.f.line(format!("br label %{done}"));
        self.f.label(&done);
        self.f.value(format!(
            "phi i1 [ {decisive}, %{skip_from} ], [ {right}, %{evaluated_from} ]"
        ))
    }

    /// Emit a checked condition, returning an i1 constant or SSA name.
    ///
    /// The left operand is evaluated before the right, preserving observable reads.
    pub(super) fn condition(&mut self, condition: &Condition) -> String {
        if matches!(condition.op, Compare::Truth) {
            let Expr::Bool(value) = &condition.left else {
                unreachable!("checked truth condition")
            };
            return self.boolean(value);
        }
        let left = self.expression(&condition.left);
        let right = self.expression(&condition.right);
        let predicate = match condition.op {
            Compare::Eq => "eq",
            Compare::Ne => "ne",
            Compare::Lt => "ult",
            Compare::Le => "ule",
            Compare::Gt => "ugt",
            Compare::Ge => "uge",
            Compare::Truth => unreachable!("truth tested above"),
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
