//! Independent destination-construction outcomes and mathematical byte limits.
//!
//! The oracle constructs owned values separately from model backing accounting.
//! An append publishes those values only after success; partial construction is
//! discarded by the caller. This code shares no native reservation/offset helper
//! with the backend, and its allocations are not runtime performance evidence.
//! Typed byte inputs retain published source identities through nested calls;
//! constructed output bytes use separate owned model values, while inputs remain
//! borrowed and partial failed values stay unpublished.

use super::ModelError;
use super::state::{ByteInput, ByteOrigin, Input, State};
use super::value::{Field, Record, Value};
use holo::ir::{Access, Argument, Build, ByteRange, BytesSource, FieldKind, Parameter};

/// A recoverable construction failure represented without backend tag numbers.
#[derive(Debug)]
pub(super) enum Rejection {
    /// A reservation exceeds backing capacity.
    Full,
    /// Size arithmetic or representation width overflowed.
    SizeOverflow,
    /// An explicitly rejected legacy constructor, which has no named error.
    Legacy,
    /// A typed error carried independently of any unpublished storage.
    Named {
        /// Nominal family of the producer returning this rejection.
        family: String,
        /// Selected case declared by the family.
        case: String,
        /// Scalar copied only for a case with an active payload.
        payload: Option<u64>,
    },
}

/// Either a specified recoverable failure or an inconclusive oracle error.
#[derive(Debug)]
pub(super) enum BuildFailure {
    /// Semantically observable outcome after the owning append restores its marker.
    Rejected(Rejection),
    /// Malformed input or model work limit, never an expected native status.
    Model(ModelError),
}

impl From<ModelError> for BuildFailure {
    /// Preserve oracle failures distinctly from specified construction outcomes.
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

impl State<'_> {
    /// Account for initialized storage without using native-width pointer arithmetic.
    ///
    /// `amount` is a mathematical field extent. Failure leaves the marker at its
    /// previous value; the owning append later restores any earlier partial work.
    fn reserve(&mut self, region: usize, amount: u128) -> Result<(), BuildFailure> {
        let next = self.regions[region].used + amount;
        // Integer overflow is distinct from the configured backing limit and is
        // checked first, matching the specified order of size and capacity errors.
        if amount > u128::from(u64::MAX) || next > u128::from(u64::MAX) {
            return Err(BuildFailure::Rejected(Rejection::SizeOverflow));
        }
        // This bound is a finite-backing policy, not an assertion about allocator
        // capacity slack, virtual reservation, or page residency.
        if next > u128::from(self.regions[region].capacity) {
            return Err(BuildFailure::Rejected(Rejection::Full));
        }
        self.regions[region].used = next;
        Ok(())
    }

    /// Construct one complete semantic record using a named producer and pure inputs.
    ///
    /// Nested failure returns immediately to the append boundary; no partially
    /// initialized destination can be caught and resumed inside this model.
    pub fn construct(
        &mut self,
        name: &str,
        args: &[Input],
        region: usize,
    ) -> Result<Record, BuildFailure> {
        // A malformed input must not exhaust the Rust stack before the compiler
        // gets a chance to report it. These are oracle limits, not native statuses.
        if self.construction_stack.len() >= 128
            || self.construction_stack.iter().any(|active| active == name)
        {
            return Err(ModelError::new("recursive or excessive model construction depth").into());
        }
        self.construction_stack.push(name.to_owned());
        let result = self.construct_steps(name, args, region);
        self.construction_stack.pop();
        result
    }

    /// Evaluate ordered steps inside a bounded model constructor frame.
    ///
    /// The caller owns stack bookkeeping on every success/error return. Nested
    /// calls reenter `construct` so cycle checks are independent of the checker.
    fn construct_steps(
        &mut self,
        name: &str,
        args: &[Input],
        region: usize,
    ) -> Result<Record, BuildFailure> {
        self.tick()?;
        let producer = self
            .program
            .producers
            .iter()
            .find(|producer| producer.name == name)
            .cloned()
            .ok_or_else(|| ModelError::new(format!("unknown model producer {name}")))?;
        // Constructors have an explicit arity, independently checked here so an
        // invalid test input cannot accidentally read another frame's parameter.
        if args.len() != producer.params.len() {
            return Err(ModelError::new("model producer argument count mismatch").into());
        }
        // Formals and evaluated values must agree independently of the compiler
        // checker: a byte descriptor is not a scalar parameter or owned payload.
        for (formal, actual) in producer.params.iter().zip(args) {
            if !matches!(
                (formal, actual),
                (Parameter::U64, Input::Scalar(_)) | (Parameter::Bytes, Input::Bytes(_))
            ) {
                return Err(ModelError::new("model producer argument kind mismatch").into());
            }
        }
        // A sole whole-result forward delegates to the exact callee before
        // reserving any leading bytes. The callee owns the one record extent,
        // and the append observes its complete outcome without retagging.
        if let [
            Build::Forward {
                producer: child,
                args: expressions,
            },
        ] = producer.body.as_slice()
        {
            let exact = self.program.producers.iter().any(|candidate| {
                candidate.name == *child
                    && candidate.record == producer.record
                    && candidate.error == producer.error
            });
            if !exact {
                return Err(ModelError::new("model forward needs an exact result identity").into());
            }
            let args = self.arguments(expressions, args)?;
            return self.construct(child, &args, region);
        }
        // Mixed construction and forwarding has no semantic record shape.
        if producer
            .body
            .iter()
            .any(|step| matches!(step, Build::Forward { .. }))
        {
            return Err(ModelError::new("model whole-result forward must stand alone").into());
        }
        let schema = self
            .program
            .records
            .iter()
            .find(|record| record.name == producer.record)
            .cloned()
            .ok_or_else(|| ModelError::new("unknown model record schema"))?;
        let leading_bytes = self.encoding.leading_bytes(self.program, &schema.name)?;
        let length_bytes = self.encoding.length_bytes();
        self.reserve(region, u128::from(leading_bytes))?;
        let mut fields = Vec::new();
        // Source-order steps determine error precedence and initialized values;
        // records are owned model values, not aliases of the native layout.
        for step in &producer.body {
            self.tick()?;
            // An explicit rejection may occur before or after initialized fields;
            // either way the caller discards this unpublished record as a whole.
            let (field, value) = match step {
                Build::Forward { .. } => {
                    unreachable!("whole-result forwards handled before reserve")
                }
                Build::FailIf(condition) => {
                    // A true rejection aborts this producer and every enclosing
                    // producer, without resuming a partially constructed value.
                    if self.condition(condition, args, None)? {
                        return Err(BuildFailure::Rejected(Rejection::Legacy));
                    }
                    continue;
                }
                Build::Reject {
                    condition,
                    case,
                    payload,
                } => {
                    let family = producer.error.as_ref().ok_or_else(|| {
                        ModelError::new("model named rejection needs a typed producer")
                    })?;
                    // Model the selected prefix only. Its payload is evaluated
                    // once on failure and remains live after suffix rollback.
                    if self.condition(condition, args, None)? {
                        let payload = payload
                            .as_ref()
                            .map(|value| self.expression(value, args, None))
                            .transpose()?;
                        return Err(BuildFailure::Rejected(Rejection::Named {
                            family: family.clone(),
                            case: case.clone(),
                            payload,
                        }));
                    }
                    continue;
                }
                Build::U64 { field, value } => {
                    let value = self.expression(value, args, None)?;
                    self.reserve(region, 8)?;
                    (field.clone(), Value::Scalar(value))
                }
                Build::Bool { field, value } => {
                    let value = self.boolean_context(value, args, None)?;
                    self.reserve(region, 1)?;
                    (field.clone(), Value::Boolean(value))
                }
                Build::Enum { field, value } => {
                    let value = self.enumeration(value)?;
                    self.reserve(region, 1)?;
                    (field.clone(), Value::Enum(value))
                }
                Build::Bytes { field, len, value } => {
                    let len = self.expression(len, args, None)?;
                    // Explicit compact width rejects unrepresentable lengths before narrowing
                    // or reserve; an allowed maximal u32 length may still exhaust tiny backing.
                    if length_bytes == 4 && len > u64::from(u32::MAX) {
                        return Err(BuildFailure::Rejected(Rejection::SizeOverflow));
                    }
                    self.reserve(region, u128::from(length_bytes) + u128::from(len))?;
                    // Native storage may hold far more bytes than a small oracle
                    // should allocate; treat a test budget excess as inconclusive.
                    if len > self.budget as u64 {
                        return Err(ModelError::new("model payload exceeds work budget").into());
                    }
                    let mut bytes = Vec::with_capacity(len as usize);
                    // Every readable payload byte is initialized independently;
                    // value narrowing is explicitly the low eight bits in this IR.
                    for index in 0..len {
                        bytes.push(self.expression(value, args, Some(index))? as u8);
                    }
                    (field.clone(), Value::Bytes(bytes))
                }
                Build::Record {
                    field,
                    producer: child,
                    args: expressions,
                } => {
                    let args = self.arguments(expressions, args)?;
                    // The parent's `try` wraps named rejections. Storage failures
                    // and model failures bypass the family mapping.
                    let nested = match self.construct(child, &args, region) {
                        Err(BuildFailure::Rejected(Rejection::Named {
                            family,
                            case,
                            payload,
                        })) => {
                            let target = producer.error.as_deref().ok_or_else(|| {
                                ModelError::new("model nested try needs a typed producer")
                            })?;
                            let (family, case) = self.lift_error(family, case, target)?;
                            return Err(BuildFailure::Rejected(Rejection::Named {
                                family,
                                case,
                                payload,
                            }));
                        }
                        other => other?,
                    };
                    (field.clone(), Value::Record(nested))
                }
            };
            let expected = schema
                .fields
                .get(fields.len())
                .ok_or_else(|| ModelError::new("too many model field initializers"))?;
            // Field order is semantic here, not a name-based implicit reorder
            // that could conceal different initialization or failure behavior.
            if expected.name != field {
                return Err(ModelError::new("model field initialization order mismatch").into());
            }
            // Independently confirm the value kind, so a checker omission cannot
            // be hidden by the oracle accepting an incompatible initializer.
            let kind_matches = match (&expected.kind, &value, step) {
                (FieldKind::U64, Value::Scalar(_), _) => true,
                (FieldKind::Bool, Value::Boolean(_), _) => true,
                (FieldKind::Enum(kind), Value::Enum(value), _) => kind == &value.kind,
                (FieldKind::Bytes, Value::Bytes(_), _) => true,
                (FieldKind::Record(expected), Value::Record(_), Build::Record { producer, .. }) => {
                    self.program.producers.iter().any(|candidate| {
                        candidate.name == *producer && candidate.record == *expected
                    })
                }
                _ => false,
            };
            // A mismatched type is invalid model input, not a recoverable storage
            // failure code that could be mistaken for expected native behavior.
            if !kind_matches {
                return Err(ModelError::new("model field initializer kind mismatch").into());
            }
            fields.push(Field { name: field, value });
        }
        // Publication requires all fields; returning a prefix would otherwise
        // hide a checker defect behind the oracle's simpler owned representation.
        if fields.len() != schema.fields.len() {
            return Err(ModelError::new("model record is incompletely initialized").into());
        }
        Ok(Record::new(fields, leading_bytes, length_bytes))
    }

    /// Capture typed constructor arguments once in source order into a model frame.
    /// Scalar evaluation copies a value; byte capture retains only source identity,
    /// projection and extent. Wrong kinds/roots return a model error without changing
    /// source bytes or markers; model frame allocations are not native copy evidence.
    pub fn arguments(
        &mut self,
        arguments: &[Argument],
        params: &[Input],
    ) -> Result<Vec<Input>, ModelError> {
        let mut values = Vec::with_capacity(arguments.len());
        // Explicit iteration makes left-to-right observation order visible and
        // avoids sharing any compiler expression traversal helper.
        for argument in arguments {
            let value = match argument {
                Argument::Scalar(expression) => {
                    Input::Scalar(self.expression(expression, params, None)?)
                }
                Argument::Bytes(source) => {
                    let mut input = self.capture_byte_source(source, params)?;
                    input.access = Access::Readonly;
                    Input::Bytes(input)
                }
                Argument::ByteView { source, ranges } => {
                    let mut input = self.capture_byte_source(source, params)?;
                    input.access = Access::Readonly;
                    self.narrow_byte_input(&mut input, ranges, params)?;
                    Input::Bytes(input)
                }
                // The memory oracle never executes hosted input, so it cannot
                // create a fresh native prefix or infer one from backing bytes.
                Argument::Received(_) => {
                    return Err(ModelError::new(
                        "received producer input is outside the hosted-free memory model",
                    ));
                }
            };
            values.push(value);
        }
        Ok(values)
    }

    /// Capture the live root, projection, and initialized extent without reading bytes.
    pub(super) fn capture_byte_source(
        &self,
        source: &BytesSource,
        params: &[Input],
    ) -> Result<ByteInput, ModelError> {
        match source {
            BytesSource::Literal(bytes) => Ok(ByteInput {
                origin: ByteOrigin::Literal(bytes.clone()),
                offset: 0,
                len: bytes.len() as u64,
                access: Access::Readonly,
            }),
            BytesSource::Param(position) => {
                let input = self.byte_input(params, *position)?.clone();
                self.input_bytes(&input)?;
                Ok(input)
            }
            BytesSource::Local(name) => {
                if !self.construction_stack.is_empty() {
                    return Err(ModelError::new(
                        "producer byte source must forward an input parameter",
                    ));
                }
                self.byte_view(name)
            }
            BytesSource::Field { reference, path } => {
                if !self.construction_stack.is_empty() {
                    return Err(ModelError::new(
                        "producer byte source must forward an input parameter",
                    ));
                }
                let identity = self.reference(reference)?;
                let Value::Bytes(bytes) = self.projection(reference, path)?.0 else {
                    return Err(ModelError::new("byte argument projects a non-byte field"));
                };
                Ok(ByteInput {
                    origin: ByteOrigin::Field {
                        batch: identity.batch,
                        index: identity.index,
                        path: path.clone(),
                    },
                    offset: 0,
                    len: bytes.len() as u64,
                    access: identity.access,
                })
            }
            BytesSource::Received(_) => Err(ModelError::new(
                "received view is outside the hosted-free memory model",
            )),
        }
    }

    /// Narrow a captured input using mathematical sums independent of native guards.
    pub(super) fn narrow_byte_input(
        &mut self,
        input: &mut ByteInput,
        ranges: &[ByteRange],
        params: &[Input],
    ) -> Result<(), ModelError> {
        for range in ranges {
            let offset = range
                .offset
                .as_ref()
                .map(|offset| self.expression(offset, params, None))
                .transpose()?
                .unwrap_or(0);
            let length = self.expression(&range.length, params, None)?;
            if u128::from(offset) + u128::from(length) > u128::from(input.len) {
                return Err(ModelError::new("byte argument bounds trap"));
            }
            input.offset += offset;
            input.len = length;
        }
        Ok(())
    }
}
