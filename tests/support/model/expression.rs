//! Ordered scalar evaluation and safe projections for the independent oracle.
//!
//! These routines inspect semantic records, never generated native pointers.
//! Address observations use disjoint synthetic bases solely for identity checks;
//! comparing printed physical addresses or allocator reuse across regions is not
//! part of the model/native differential fixture domain. Readonly byte inputs
//! retain logical identities and extents; observations never clone their payloads.
//! Scalar applications capture actual values once before evaluating a closed
//! scalar body, so unused potentially trapping byte reads cannot disappear.

use super::ModelError;
use super::state::{Binding, ByteInput, ByteOrigin, Input, State};
use super::value::{Record, Value};
use holo::ir::{Access, Binary, BoolExpr, Compare, Condition, Expr};

impl State<'_> {
    /// Evaluate a scalar expression in source order using explicit pure inputs.
    ///
    /// `params` and `index` are constructor-only inputs; invalid projections or
    /// out-of-range byte reads return a model trap instead of panicking.
    pub fn expression(
        &mut self,
        expr: &Expr,
        params: &[Input],
        index: Option<u64>,
    ) -> Result<u64, ModelError> {
        self.expression_context(expr, params, index, false)
    }

    /// Evaluate under either ordinary caller observations or a closed scalar frame.
    /// `scalar_only` rejects caller locals/byte inputs/indices and published memory
    /// from application bodies; arguments retain their caller context. Captured
    /// values allocate only in this test oracle, never in generated native code.
    fn expression_context(
        &mut self,
        expr: &Expr,
        params: &[Input],
        index: Option<u64>,
        scalar_only: bool,
    ) -> Result<u64, ModelError> {
        self.tick()?;
        // A scalar helper can consume captured u64 values but cannot gain access
        // to its caller's byte borrow, byte index, or lexical storage by syntax.
        if scalar_only
            && !matches!(
                expr,
                Expr::Const(_)
                    | Expr::Param(_)
                    | Expr::Binary { .. }
                    | Expr::Apply { .. }
                    | Expr::LiteralByte { .. }
            )
        {
            return Err(ModelError::new(
                "scalar application body observes caller state",
            ));
        }
        // Dispatch on semantic operations, deliberately independent of LLVM
        // lowering and of any compiler expression type/effect helper.
        match expr {
            Expr::Const(value) => Ok(*value),
            Expr::Bool(_) => Err(ModelError::new("boolean used as a model scalar")),
            Expr::Local(name) => self.scalar(name),
            Expr::Param(position) => match params.get(*position) {
                Some(Input::Scalar(value)) => Ok(*value),
                _ => Err(ModelError::new(
                    "scalar constructor parameter unavailable or wrong kind",
                )),
            },
            Expr::InputLen(position) => Ok(self.byte_input(params, *position)?.len),
            Expr::Apply { args, body } => {
                let mut captured = Vec::with_capacity(args.len());
                // Evaluate every actual exactly once in written order, including
                // arguments unused by the helper or referenced repeatedly in its body.
                for argument in args {
                    captured.push(Input::Scalar(self.expression_context(
                        argument,
                        params,
                        index,
                        scalar_only,
                    )?));
                }
                self.expression_context(body, &captured, None, true)
            }
            Expr::Call { function, args } => self.ordinary_call(function, args),
            Expr::Handle {
                function,
                args,
                arms,
            } => self.handle_result(function, args, arms),
            Expr::HandleSavedResult { name, arms } => {
                let outcome = self.saved_result(name)?;
                self.evaluate_result_arm(&outcome, arms)
            }
            Expr::InputByte { param, index: at } => {
                let at = self.expression_context(at, params, index, scalar_only)?;
                let input = self.byte_input(params, *param)?;
                self.read_input_byte(input, at)
            }
            Expr::LiteralByte { bytes, index: at } => {
                let at = self.expression_context(at, params, index, scalar_only)?;
                usize::try_from(at)
                    .ok()
                    .and_then(|at| bytes.get(at))
                    .map(|byte| u64::from(*byte))
                    .ok_or_else(|| ModelError::new("byte literal read bounds trap"))
            }
            Expr::ViewLen(name) => Ok(self.byte_view(name)?.len),
            Expr::ViewByte { name, index: at } => {
                let input = self.byte_view(name)?;
                let at = self.expression_context(at, params, index, scalar_only)?;
                self.read_input_byte(&input, at)
            }
            Expr::Index => index.ok_or_else(|| ModelError::new("byte index outside initializer")),
            Expr::Binary { op, left, right } => {
                let a = self.expression_context(left, params, index, scalar_only)?;
                let b = self.expression_context(right, params, index, scalar_only)?;
                // All user arithmetic in this IR is explicitly wrapping; this
                // says nothing about address/size reservations, checked elsewhere.
                Ok(match op {
                    Binary::AddWrapping => a.wrapping_add(b),
                    Binary::SubWrapping => a.wrapping_sub(b),
                    Binary::MulWrapping => a.wrapping_mul(b),
                    Binary::Xor => a ^ b,
                    Binary::And => a & b,
                })
            }
            Expr::Read { reference, path } => {
                // A scalar projection cannot reinterpret payload/header bytes as
                // another type, even though native storage is a packed byte image.
                match self.projection(reference, path)?.0 {
                    Value::Scalar(value) => Ok(*value),
                    _ => Err(ModelError::new("non-scalar read projection")),
                }
            }
            Expr::Byte {
                reference,
                path,
                index: byte_index,
            } => {
                let at = self.expression_context(byte_index, params, index, scalar_only)?;
                // Convert only after a semantic bounds test through get(); this
                // oracle uses the host vector rather than native offset arithmetic.
                match self.projection(reference, path)?.0 {
                    Value::Bytes(bytes) => usize::try_from(at)
                        .ok()
                        .and_then(|at| bytes.get(at))
                        .map(|value| u64::from(*value))
                        .ok_or_else(|| ModelError::new("byte read bounds trap")),
                    _ => Err(ModelError::new("non-byte read projection")),
                }
            }
            Expr::BytesLen { reference, path } => {
                // Semantic length comes from the owned payload; it cannot be
                // changed by writing compiler-managed shape metadata.
                match self.projection(reference, path)?.0 {
                    Value::Bytes(bytes) => Ok(bytes.len() as u64),
                    _ => Err(ModelError::new("non-byte length projection")),
                }
            }
            Expr::BatchLen(name) => Ok(self.batch_view(name, Access::Readonly)?.count as u64),
            Expr::Used(name) => u64::try_from(self.regions[self.region_id(name)?].used)
                .map_err(|_| ModelError::new("model marker exceeds native integer range")),
            Expr::Address(name) => {
                let reference = self.reference(name)?;
                let batch = &self.batches[reference.batch];
                let offset = u64::try_from(batch.records[reference.index].offset)
                    .map_err(|_| ModelError::new("model address offset overflow"))?;
                // Synthetic bases preserve same-region identity and offsets for
                // small test allocations, without pretending to predict malloc.
                Ok(((batch.region as u64 + 1) << 40).wrapping_add(offset))
            }
            Expr::Nanoseconds => Err(ModelError::new(
                "nondeterministic measurement is outside the value oracle",
            )),
        }
    }

    /// Evaluate a boolean in the current execution frame.
    pub fn boolean(&mut self, value: &BoolExpr) -> Result<bool, ModelError> {
        self.boolean_context(value, &[], None)
    }

    /// Evaluate boolean operators using the scalar inputs of their enclosing frame.
    fn boolean_context(
        &mut self,
        value: &BoolExpr,
        params: &[Input],
        index: Option<u64>,
    ) -> Result<bool, ModelError> {
        self.tick()?;
        match value {
            BoolExpr::Const(value) => Ok(*value),
            BoolExpr::Local(name) => {
                if !self.construction_stack.is_empty() {
                    return Err(ModelError::new("boolean local used in model construction"));
                }
                match self.binding(name)? {
                    Binding::Boolean(value) => Ok(value),
                    _ => Err(ModelError::new(format!("{name} is not a boolean"))),
                }
            }
            BoolExpr::Call { function, args } => self.boolean_call(function, args),
            BoolExpr::Compare(condition) => self.scalar_comparison(condition, params, index),
            BoolExpr::And(left, right) => Ok(self.boolean_context(left, params, index)?
                && self.boolean_context(right, params, index)?),
            BoolExpr::Or(left, right) => Ok(self.boolean_context(left, params, index)?
                || self.boolean_context(right, params, index)?),
            BoolExpr::Not(value) => Ok(!self.boolean_context(value, params, index)?),
        }
    }

    /// Evaluate a boolean test or an ordered pair of scalar operands.
    pub fn condition(
        &mut self,
        condition: &Condition,
        params: &[Input],
        index: Option<u64>,
    ) -> Result<bool, ModelError> {
        if matches!(condition.op, Compare::Truth) {
            if let (Expr::Bool(value), Expr::Const(0)) = (&condition.left, &condition.right) {
                return self.boolean_context(value, params, index);
            }
            return Err(ModelError::new("malformed model boolean condition"));
        }
        self.scalar_comparison(condition, params, index)
    }

    /// Compare two expressions after evaluating the left one before the right.
    fn scalar_comparison(
        &mut self,
        condition: &Condition,
        params: &[Input],
        index: Option<u64>,
    ) -> Result<bool, ModelError> {
        if matches!(condition.op, Compare::Truth) {
            return Err(ModelError::new(
                "model scalar comparison needs a comparison operator",
            ));
        }
        let left = self.expression(&condition.left, params, index)?;
        let right = self.expression(&condition.right, params, index)?;
        // Native and model comparisons are unsigned, with no floating-point or
        // pointer-ordering claims hidden inside this fixture operation set.
        Ok(match condition.op {
            Compare::Eq => left == right,
            Compare::Ne => left != right,
            Compare::Lt => left < right,
            Compare::Le => left <= right,
            Compare::Gt => left > right,
            Compare::Ge => left >= right,
            Compare::Truth => unreachable!("boolean tests are handled before comparisons"),
        })
    }

    /// Project an initialized named field and its relative fixture offset.
    pub fn projection(&self, name: &str, path: &[String]) -> Result<(&Value, u128), ModelError> {
        let reference = self.reference(name)?;
        let stored = &self.batches[reference.batch].records[reference.index];
        let (value, inner) = project(&stored.record, path)?;
        Ok((value, stored.offset + inner))
    }

    /// Resolve a typed byte argument without borrowing source contents into a snapshot.
    /// `params` is the current call frame; wrong-kind/out-of-range slots are model errors.
    pub fn byte_input<'a>(
        &'a self,
        params: &'a [Input],
        at: usize,
    ) -> Result<&'a ByteInput, ModelError> {
        // Ordinary operations run with a local frame; constructors explicitly
        // supply their typed inputs and cannot see the ordinary frame.
        if self.construction_stack.is_empty()
            && !self.ordinary_stack.is_empty()
            && params.is_empty()
        {
            return self
                .ordinary_bytes
                .get(at)
                .and_then(Option::as_ref)
                .ok_or_else(|| {
                    ModelError::new("ordinary byte parameter unavailable or wrong kind")
                });
        }
        // Numeric argument bits can never be reinterpreted as a model storage identity.
        match params.get(at) {
            Some(Input::Bytes(input)) => Ok(input),
            _ => Err(ModelError::new(
                "byte constructor parameter unavailable or wrong kind",
            )),
        }
    }

    /// Read one byte after validating the captured logical bound.
    fn read_input_byte(&self, input: &ByteInput, at: u64) -> Result<u64, ModelError> {
        if at >= input.len {
            return Err(ModelError::new("input byte read bounds trap"));
        }
        let bytes = self.input_bytes(input)?;
        let at = usize::try_from(at).map_err(|_| ModelError::new("input byte read bounds trap"))?;
        bytes
            .get(at)
            .map(|byte| u64::from(*byte))
            .ok_or_else(|| ModelError::new("input byte read storage unavailable"))
    }

    /// Read current initialized bytes using a captured logical record/path identity.
    /// Checks lifetime, publication, field kind and frozen captured extent; returns
    /// a borrowed host slice with no payload allocation or native address assumption.
    pub fn input_bytes<'a>(&'a self, input: &'a ByteInput) -> Result<&'a [u8], ModelError> {
        let bytes: &[u8] = match &input.origin {
            ByteOrigin::Literal(bytes) => bytes,
            ByteOrigin::Field { batch, index, path } => {
                let batch = self
                    .batches
                    .get(*batch)
                    .filter(|batch| batch.active)
                    .ok_or_else(|| ModelError::new("input byte root is no longer live"))?;
                let stored = batch
                    .records
                    .get(*index)
                    .ok_or_else(|| ModelError::new("input byte record is no longer published"))?;
                let Value::Bytes(bytes) = project(&stored.record, path)?.0 else {
                    return Err(ModelError::new("input byte projection changed"));
                };
                bytes
            }
        };
        // Retain the original relative window even through several forwards.
        if u128::from(input.offset) + u128::from(input.len) <= bytes.len() as u128 {
            return Ok(&bytes[input.offset as usize..(input.offset + input.len) as usize]);
        }
        Err(ModelError::new(
            "input byte projection or captured extent changed",
        ))
    }
}

/// Traverse an ordered semantic field path, deriving offsets without compiler code.
///
/// Returns the final value and offset from the outer record start, or rejects a
/// missing field / attempt to descend through a scalar or byte payload.
fn project<'a>(record: &'a Record, path: &[String]) -> Result<(&'a Value, u128), ModelError> {
    let (first, rest) = path
        .split_first()
        .ok_or_else(|| ModelError::new("empty field path"))?;
    let (value, offset) = record
        .field(first)
        .ok_or_else(|| ModelError::new(format!("unknown model field {first}")))?;
    // The final field is already initialized and may be returned without a
    // recursive lookup; only nested records admit further path components.
    if rest.is_empty() {
        return Ok((value, offset));
    }
    // This semantic descent cannot enter payload length metadata or scalar bits.
    match value {
        Value::Record(inner) => {
            let (value, inner_offset) = project(inner, rest)?;
            Ok((value, offset + inner_offset))
        }
        _ => Err(ModelError::new("field path descends through a non-record")),
    }
}
