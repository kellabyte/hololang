//! Structured operation evaluation for the independent region oracle.
//!
//! Lexical scopes control model bindings, while stable logical record handles
//! preserve alias identity through vector growth. Cleanup may scan/drop model
//! values; this is deliberately separate from the generated marker-reset path.

use super::ModelError;
use super::construction::{BuildFailure, Rejection};
use super::ordinary::Outcome;
use super::state::{Batch, Binding, Reference, Region, State, Stored};
use super::value::{Record, Value};
use holo::ir::{Access, AppendArm, AppendPattern, Argument, ConstructArm, Expr, Op, ResultValue};
use std::collections::BTreeMap;

/// Match a semantic outcome and supply its optional binding without native tags.
///
/// The outer option means the pattern matched. The inner option names an
/// active payload or opaque error. Legacy rejection is outside typed handling.
fn match_outcome<'a, T>(
    pattern: &'a AppendPattern,
    outcome: &Result<T, Rejection>,
) -> Option<Option<(&'a String, Binding)>> {
    // Match by meaning and nominal family, independently of emitted ordinals.
    match (pattern, outcome) {
        (AppendPattern::Ok, Ok(_))
        | (AppendPattern::Full, Err(Rejection::Full))
        | (AppendPattern::SizeOverflow, Err(Rejection::SizeOverflow)) => Some(None),
        (
            AppendPattern::Rejected {
                family,
                case,
                binding,
            },
            Err(Rejection::Named {
                family: actual_family,
                case: actual_case,
                payload,
            }),
        ) if family == actual_family && case == actual_case => Some(
            binding
                .as_ref()
                .zip(*payload)
                .map(|(name, value)| (name, Binding::Scalar(value))),
        ),
        (
            AppendPattern::Fallback(name),
            Err(Rejection::Full | Rejection::SizeOverflow | Rejection::Named { .. }),
        ) => Some(name.as_ref().map(|name| (name, Binding::Error))),
        _ => None,
    }
}

impl State<'_> {
    /// Execute structured operations in order, retaining deterministic outputs.
    pub fn execute_ops(&mut self, operations: &[Op]) -> Result<(), ModelError> {
        // Sequential evaluation preserves alias-visible writes and failure
        // publication boundaries independently of backend control-flow lowering.
        for operation in operations {
            // Completion propagates out of structured bodies while their callers
            // still restore lexical markers and end their model owner lifetimes.
            if self.exited || self.returned.is_some() {
                break;
            }
            self.execute(operation)?;
        }
        Ok(())
    }

    /// Run operations in a fresh lexical scope, removing bindings on every exit.
    fn scoped(&mut self, operations: &[Op]) -> Result<(), ModelError> {
        self.scopes.push(BTreeMap::new());
        let result = self.execute_ops(operations);
        self.scopes.pop();
        result
    }

    /// Construct one private suffix and either publish a complete record or
    /// restore its original marker before returning a semantic rejection.
    /// Invalid or unsupported model input reports an outer error.
    fn append(
        &mut self,
        batch: &str,
        producer: &str,
        args: &[Argument],
        typed: bool,
    ) -> Result<Result<(), Rejection>, ModelError> {
        let batch = self.batch_id(batch)?;
        let definition = self
            .program
            .producers
            .iter()
            .find(|candidate| candidate.name == producer)
            .ok_or_else(|| ModelError::new("unknown append producer"))?;
        // Check the logical receiving schema and result convention independently.
        if definition.record != self.batches[batch].schema || definition.error.is_some() != typed {
            return Err(ModelError::new(
                "model append result schema or convention mismatch",
            ));
        }
        let region = self.batches[batch].region;
        let before = self.regions[region].used;
        let args = self.arguments(args, &[])?;
        match self.construct(producer, &args, region) {
            Ok(record) => {
                // Completed semantic values derive size independently of the
                // oracle's incremental reservation accounting.
                if self.regions[region].used - before != record.encoded_len() {
                    return Err(ModelError::new(
                        "model construction byte accounting mismatch",
                    ));
                }
                self.batches[batch].records.push(Stored {
                    offset: before,
                    record,
                });
                Ok(Ok(()))
            }
            Err(BuildFailure::Rejected(failure)) => {
                self.regions[region].used = before;
                Ok(Err(failure))
            }
            Err(BuildFailure::Model(error)) => Err(error),
        }
    }

    /// Select an arm by semantic outcome after the append has committed or
    /// restored its marker. Only an active named payload can be read as a scalar.
    fn handle_append(
        &mut self,
        outcome: &Result<(), Rejection>,
        arms: &[AppendArm],
    ) -> Result<u64, ModelError> {
        let (arm, binding) = arms
            .iter()
            .find_map(|arm| match_outcome(&arm.pattern, outcome).map(|binding| (arm, binding)))
            .ok_or_else(|| {
                ModelError::new("model append handler has no arm for the active outcome")
            })?;
        self.evaluate_arm(&arm.value, binding)
    }

    /// Attempt one temporary record and execute its selected lexical arm.
    ///
    /// A model-only single-record generation reuses stable logical handles for
    /// projections and borrowed calls. It is never bound as a batch. Failure
    /// restores the marker before dispatch and success restores it after the
    /// arm, even when that arm completes entry or reports a model error.
    fn scoped_construct(
        &mut self,
        region: &str,
        producer: &str,
        args: &[Argument],
        arms: &[ConstructArm],
    ) -> Result<(), ModelError> {
        let region = self.region_id(region)?;
        // Both collection and single-record generations occupy the backing tail.
        if self
            .batches
            .iter()
            .any(|owner| owner.active && owner.region == region)
        {
            return Err(ModelError::new(
                "model construction requires a free region tail",
            ));
        }
        let definition = self
            .program
            .producers
            .iter()
            .find(|candidate| candidate.name == producer && candidate.error.is_some())
            .ok_or_else(|| ModelError::new("model scoped construction needs a typed producer"))?;
        let start = self.regions[region].used;
        let actuals = self.arguments(args, &[])?;
        let outcome = match self.construct(producer, &actuals, region) {
            Ok(record) => Ok(record),
            Err(BuildFailure::Rejected(reason)) => {
                self.regions[region].used = start;
                Err(reason)
            }
            Err(BuildFailure::Model(error)) => return Err(error),
        };
        let (arm, binding) = arms
            .iter()
            .find_map(|arm| match_outcome(&arm.pattern, &outcome).map(|binding| (arm, binding)))
            .ok_or_else(|| ModelError::new("model construct has no arm for the active outcome"))?;
        let mut locals = BTreeMap::new();
        // Failure contributes at most one copied scalar and keeps no record.
        if let Some((name, value)) = binding {
            locals.insert(name.clone(), value);
        }
        let owner = match outcome {
            Ok(record) => {
                // Compare independent extent accounting before exposing the record.
                if self.regions[region].used - start != record.encoded_len() {
                    return Err(ModelError::new(
                        "model scoped record byte accounting mismatch",
                    ));
                }
                let id = self.batches.len();
                self.batches.push(Batch {
                    region,
                    schema: definition.record.clone(),
                    start,
                    records: vec![Stored {
                        offset: start,
                        record,
                    }],
                    active: true,
                });
                // Discarding the name does not relinquish the tail early.
                if let Some(name) = &arm.reference {
                    locals.insert(
                        name.clone(),
                        Binding::Reference(Reference {
                            batch: id,
                            index: 0,
                            access: Access::Readonly,
                        }),
                    );
                }
                Some(id)
            }
            Err(_) => None,
        };
        self.scopes.push(locals);
        let result = self.execute_ops(&arm.body);
        self.scopes.pop();
        // Once the lexical reference is gone, invalidate its logical generation.
        if let Some(id) = owner {
            self.batches[id].active = false;
            self.regions[region].used = start;
        }
        result
    }

    /// Evaluate one selected arm's scalar expression and restore the caller's
    /// lexical environment even when the oracle detects a trap or work limit.
    pub(super) fn evaluate_arm(
        &mut self,
        expression: &Expr,
        binding: Option<(&String, Binding)>,
    ) -> Result<u64, ModelError> {
        let mut locals = BTreeMap::new();
        // Only a named payload or opaque fallback error introduces a local.
        if let Some((name, value)) = binding {
            locals.insert(name.clone(), value);
        }
        self.scopes.push(locals);
        let value = self.expression(expression, &[], None);
        self.scopes.pop();
        value
    }

    /// Execute one operation using owned model values and explicit lifetimes.
    fn execute(&mut self, operation: &Op) -> Result<(), ModelError> {
        self.tick()?;
        // Each operation owns only its documented mutation; a failed constructor
        // never turns into a whole-function transaction or an implicit relocation.
        match operation {
            // Ordinary definitions have no executable effect in the entry sequence.
            Op::Function(_) | Op::ErrorFamily(_) => Ok(()),
            // The bounded memory oracle owns no descriptors or live readiness.
            Op::TcpScope { .. }
            | Op::TcpIo { .. }
            | Op::TcpClose { .. }
            | Op::Refill(_)
            | Op::WaitInput { .. }
            | Op::Send { .. }
            | Op::WaitOutput { .. }
            | Op::NextSession { .. }
            | Op::Loop(_) => Err(ModelError::new(
                "hosted input and unbounded entry loops are outside the memory model",
            )),
            Op::Region {
                name,
                capacity,
                fail_acquire,
                body,
                on_failure,
            } => {
                let capacity = self.expression(capacity, &[], None)?;
                // Acquisition failure exposes no partial owner. Natural allocator
                // exhaustion is not predicted by this small safe-Rust model.
                if *fail_acquire {
                    return self.scoped(on_failure);
                }
                let id = self.regions.len();
                self.regions.push(Region {
                    capacity,
                    used: 0,
                    active: true,
                });
                self.scopes.push(BTreeMap::new());
                self.declare(name, Binding::Region(id))?;
                let result = self.execute_ops(body);
                self.scopes.pop();
                self.regions[id].active = false;
                result
            }
            Op::Batch {
                name,
                region,
                record,
                body,
            } => {
                let region = self.region_id(region)?;
                // Two independently growing owners cannot silently share one
                // construction tail even in the oracle's pointer-free storage.
                if self
                    .batches
                    .iter()
                    .any(|batch| batch.active && batch.region == region)
                {
                    return Err(ModelError::new("two live batches own the same region tail"));
                }
                let start = self.regions[region].used;
                let id = self.batches.len();
                self.batches.push(Batch {
                    region,
                    schema: record.clone(),
                    start,
                    records: Vec::new(),
                    active: true,
                });
                self.scopes.push(BTreeMap::new());
                self.declare(name, Binding::Batch(id))?;
                let result = self.execute_ops(body);
                self.scopes.pop();
                self.regions[region].used = start;
                self.batches[id].active = false;
                result
            }
            Op::Scope(body) => self.scoped(body),
            Op::Let { name, value } => {
                let value = self.expression(value, &[], None)?;
                self.declare(name, Binding::Scalar(value))
            }
            Op::TryLet {
                name,
                function,
                args,
            } => {
                // A try operation requires a typed destination in the enclosing
                // frame. Invoke first so operand and callee effects stay ordered.
                let target = self
                    .ordinary_result_family
                    .ok_or_else(|| ModelError::new("model try outside typed ordinary function"))?;
                match self.invoke(function, args)? {
                    Outcome::Success(value) => self.declare(name, Binding::Scalar(value)),
                    Outcome::Failure {
                        family,
                        case,
                        payload,
                    } => {
                        let (family, case) = self.lift_error(family, case, target)?;
                        self.returned = Some(Outcome::Failure {
                            family,
                            case,
                            payload,
                        });
                        Ok(())
                    }
                    Outcome::Scalar(_) => Err(ModelError::new("model try needs a typed result")),
                }
            }
            Op::Set { name, value } => {
                self.scalar(name)?;
                let value = self.expression(value, &[], None)?;
                self.replace(name, Binding::Scalar(value))
            }
            Op::Append {
                batch,
                producer,
                args,
                status,
            } => {
                let code = match self.append(batch, producer, args, false)? {
                    Ok(()) => 0,
                    Err(Rejection::Full) => 1,
                    Err(Rejection::SizeOverflow) => 2,
                    Err(Rejection::Legacy) => 3,
                    Err(Rejection::Named { .. }) => {
                        return Err(ModelError::new(
                            "numeric model append returned a named error",
                        ));
                    }
                };
                self.declare(status, Binding::Scalar(code))
            }
            Op::AppendHandled {
                batch,
                producer,
                args,
                result,
                arms,
            } => {
                let outcome = self.append(batch, producer, args, true)?;
                let value = self.handle_append(&outcome, arms)?;
                self.declare(result, Binding::Scalar(value))
            }
            Op::ConstructHandled {
                region,
                producer,
                args,
                arms,
            } => self.scoped_construct(region, producer, args, arms),
            Op::Borrow {
                name,
                batch,
                index,
                access,
            } => {
                let batch = self.batch_id(batch)?;
                let index = self.expression(index, &[], None)?;
                let index = usize::try_from(index)
                    .map_err(|_| ModelError::new("record index conversion trap"))?;
                // A reference can name only a fully published element, never an
                // unfinished suffix or the record just beyond the batch length.
                if index >= self.batches[batch].records.len() {
                    return Err(ModelError::new("record borrow bounds trap"));
                }
                self.declare(
                    name,
                    Binding::Reference(Reference {
                        batch,
                        index,
                        access: *access,
                    }),
                )
            }
            Op::Alias {
                name,
                source,
                access,
            } => {
                let mut reference = self.reference(source)?;
                // A new alias may restrict permission, but cannot recover write
                // access from a readonly path merely by changing its binding.
                if reference.access == Access::Readonly && *access == Access::Readwrite {
                    return Err(ModelError::new("readonly alias upgrade"));
                }
                reference.access = *access;
                self.declare(name, Binding::Reference(reference))
            }
            Op::AssignRef { name, source } => {
                let destination = self.reference(name)?;
                let mut reference = self.reference(source)?;
                // Keep the destination's declared access permission rather than
                // upgrading it as a side effect of copying a writable handle.
                if destination.access == Access::Readwrite && reference.access == Access::Readonly {
                    return Err(ModelError::new(
                        "readonly reference assigned to writable binding",
                    ));
                }
                reference.access = destination.access;
                self.replace(name, Binding::Reference(reference))
            }
            Op::Write {
                reference,
                path,
                value,
            } => {
                let handle = self.reference(reference)?;
                self.require_write(&handle)?;
                let value = self.expression(value, &[], None)?;
                let record = &mut self.batches[handle.batch].records[handle.index].record;
                write_scalar(record, path, value)
            }
            Op::WriteByte {
                reference,
                path,
                index,
                value,
            } => {
                let handle = self.reference(reference)?;
                self.require_write(&handle)?;
                let index = self.expression(index, &[], None)?;
                // Resolve and validate the destination before observing the RHS;
                // even pure reads on the RHS may otherwise trap in another order.
                match self.projection(reference, path)?.0 {
                    Value::Bytes(bytes) if index < bytes.len() as u64 => {}
                    Value::Bytes(_) => return Err(ModelError::new("byte write bounds trap")),
                    _ => return Err(ModelError::new("non-byte write projection")),
                }
                let value = self.expression(value, &[], None)? as u8;
                let record = &mut self.batches[handle.batch].records[handle.index].record;
                write_byte(record, path, index, value)
            }
            Op::Reset { batch } => {
                let batch = self.batch_id(batch)?;
                // The model may scan its scopes for diagnostics; the native
                // compiler must establish this statically without runtime scans.
                if self.scopes.iter().flat_map(|scope| scope.values())
                    .any(|binding| matches!(binding, Binding::Reference(reference) if reference.batch == batch)) {
                    return Err(ModelError::new("reset while a reference remains usable"));
                }
                let region = self.batches[batch].region;
                self.regions[region].used = self.batches[batch].start;
                self.batches[batch].records.clear();
                Ok(())
            }
            Op::Repeat { count, index, body } => {
                let count = self.expression(count, &[], None)?;
                // Count is captured once. Every iteration has its own lexical
                // bindings, while assignments to outer scalars remain visible.
                for value in 0..count {
                    // Entry completion ends the loop rather than spending model
                    // budget on unreachable iterations after a nested exit.
                    if self.exited || self.returned.is_some() {
                        break;
                    }
                    self.tick()?;
                    self.scopes.push(BTreeMap::new());
                    self.declare(index, Binding::Scalar(value))?;
                    let result = self.execute_ops(body);
                    self.scopes.pop();
                    result?;
                }
                Ok(())
            }
            Op::ForEach {
                batch,
                reference,
                access,
                body,
            } => {
                let batch = self.batch_id(batch)?;
                let count = self.batches[batch].records.len();
                // Capture the initial published prefix. Appended records are not
                // visited, and rebinding the body cursor cannot change progression.
                for index in 0..count {
                    // A completed process cannot visit another published record.
                    if self.exited || self.returned.is_some() {
                        break;
                    }
                    self.tick()?;
                    self.scopes.push(BTreeMap::new());
                    self.declare(
                        reference,
                        Binding::Reference(Reference {
                            batch,
                            index,
                            access: *access,
                        }),
                    )?;
                    let result = self.execute_ops(body);
                    self.scopes.pop();
                    result?;
                }
                Ok(())
            }
            Op::If {
                condition,
                then_body,
                else_body,
            } => {
                // Only the selected branch observes/mutates state; both branches
                // are independently checked by the compiler before native codegen.
                if self.condition(condition, &[], None)? {
                    self.scoped(then_body)
                } else {
                    self.scoped(else_body)
                }
            }
            Op::Assert(condition) => {
                // A failed language assertion terminates the tested execution;
                // it is never treated as a nonfatal external property report.
                if !self.condition(condition, &[], None)? {
                    return Err(ModelError::new(format!(
                        "model assertion trap: {condition:?}"
                    )));
                }
                Ok(())
            }
            Op::Print(value) => {
                let value = self.expression(value, &[], None)?;
                self.output.push(value);
                Ok(())
            }
            // The numeric oracle exposes only scalar observations. Native CLI
            // tests compare exact byte output for programs that print text.
            Op::PrintText(_) => Err(ModelError::new("model supports only numeric output")),
            Op::Return(value) => {
                // A helper's scalar return stops its frame, not the entry process.
                if self.ordinary_stack.is_empty() {
                    return Err(ModelError::new("scalar return outside ordinary function"));
                }
                self.returned = Some(Outcome::Scalar(self.expression(value, &[], None)?));
                Ok(())
            }
            Op::ReturnResult(value) => {
                // Typed completion is also local to the current ordinary frame.
                if self.ordinary_stack.is_empty() {
                    return Err(ModelError::new(
                        "typed result return outside ordinary function",
                    ));
                }
                self.returned = Some(match value {
                    ResultValue::Ok(value) => {
                        Outcome::Success(self.expression(value, &[], None)?)
                    }
                    ResultValue::Error {
                        family,
                        case,
                        payload,
                    } => Outcome::Failure {
                        family: family.clone(),
                        case: case.clone(),
                        payload: payload
                            .as_ref()
                            .map(|value| self.expression(value, &[], None))
                            .transpose()?,
                    },
                    ResultValue::Call { function, args } => self.invoke(function, args)?,
                });
                Ok(())
            }
            Op::ExitZero => {
                // Entry cleanup has no meaning in a synchronous borrowed call.
                if !self.ordinary_stack.is_empty() {
                    return Err(ModelError::new("entry completion inside ordinary function"));
                }
                self.exited = true;
                Ok(())
            }
            Op::OpaqueRegion(name) => {
                // An optimizer barrier has no semantic record mutation. Resolve
                // the owner without pretending to predict hardware timing effects.
                self.region_id(name)?;
                Ok(())
            }
        }
    }

    /// Establish writable path permission before mutating semantic record data.
    fn require_write(&self, reference: &Reference) -> Result<(), ModelError> {
        // Aliasing is permitted; only this path's access capability determines
        // whether the requested existing-field write is legal.
        if reference.access != Access::Readwrite {
            return Err(ModelError::new("write through readonly model reference"));
        }
        Ok(())
    }
}

/// Replace a scalar reached through nested semantic records, preserving shape.
fn write_scalar(record: &mut Record, path: &[String], replacement: u64) -> Result<(), ModelError> {
    let value = project_mut(record, path)?;
    // Shape metadata is never represented as an ordinary scalar field here.
    match value {
        Value::Scalar(value) => {
            *value = replacement;
            Ok(())
        }
        _ => Err(ModelError::new("non-scalar write projection")),
    }
}

/// Replace one existing payload byte after a semantic bounds check.
fn write_byte(
    record: &mut Record,
    path: &[String],
    index: u64,
    replacement: u8,
) -> Result<(), ModelError> {
    let value = project_mut(record, path)?;
    // Byte mutation cannot resize the vector or rewrite its length metadata.
    match value {
        Value::Bytes(bytes) => {
            let slot = usize::try_from(index)
                .ok()
                .and_then(|index| bytes.get_mut(index))
                .ok_or_else(|| ModelError::new("byte write bounds trap"))?;
            *slot = replacement;
            Ok(())
        }
        _ => Err(ModelError::new("non-byte write projection")),
    }
}

/// Traverse only initialized semantic record fields to obtain one mutable value.
fn project_mut<'a>(record: &'a mut Record, path: &[String]) -> Result<&'a mut Value, ModelError> {
    let (first, rest) = path
        .split_first()
        .ok_or_else(|| ModelError::new("empty mutable field path"))?;
    let value = record
        .field_mut(first)
        .ok_or_else(|| ModelError::new(format!("unknown mutable model field {first}")))?;
    // Stop at the chosen field; further components are valid only inside an
    // inline record, never inside scalar bits or payload shape metadata.
    if rest.is_empty() {
        return Ok(value);
    }
    match value {
        Value::Record(inner) => project_mut(inner, rest),
        _ => Err(ModelError::new(
            "mutable path descends through a non-record",
        )),
    }
}
