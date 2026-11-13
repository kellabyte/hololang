//! Independent semantics for synchronous ordinary calls over caller storage.
//!
//! Model borrows retain batch/record/path identities and checked byte windows.
//! Each call receives a fresh lexical frame while updating the same owned model
//! records as its caller. Host allocations here are never part of native timing.

use std::collections::BTreeMap;

use holo::ir::{
    Access, FunctionArgument, FunctionParameterKind, Op, ResultArm, ResultBlockArm, ResultPattern,
    ResultSuccess,
};

use super::{
    ModelError,
    state::{Binding, State},
};

/// Independent ordinary-call outcomes, distinguished without machine tags.
#[derive(Clone, Debug)]
pub(super) enum Outcome {
    /// Result of a plain scalar helper.
    Scalar(u64),
    /// Result of a plain boolean helper.
    Boolean(bool),
    /// Success of a typed helper with the entire unsigned range available.
    Success(u64),
    /// Success without an observable value.
    SuccessUnit,
    /// Typed failure represented by its declared names, independently of LLVM tags.
    Failure {
        /// Nominal error family.
        family: String,
        /// Declared case within the family.
        case: String,
        /// Optional scalar stored independently of the compiler's lane encoding.
        payload: Option<u64>,
    },
}

impl State<'_> {
    /// Invoke a scalar helper and reject observing a typed result as a scalar.
    pub(super) fn ordinary_call(
        &mut self,
        name: &str,
        arguments: &[FunctionArgument],
    ) -> Result<u64, ModelError> {
        match self.invoke(name, arguments)? {
            Outcome::Scalar(value) => Ok(value),
            _ => Err(ModelError::new(
                "model scalar call requires a scalar result",
            )),
        }
    }

    /// Invoke a boolean helper, preserving the result's boolean kind.
    pub(super) fn boolean_call(
        &mut self,
        name: &str,
        arguments: &[FunctionArgument],
    ) -> Result<bool, ModelError> {
        match self.invoke(name, arguments)? {
            Outcome::Boolean(value) => Ok(value),
            _ => Err(ModelError::new(
                "model boolean call requires a boolean result",
            )),
        }
    }

    /// Evaluate `arguments` once, then run `name` in an independent lexical frame.
    /// Logical state references mutate the caller's records on either outcome.
    pub(super) fn invoke(
        &mut self,
        name: &str,
        arguments: &[FunctionArgument],
    ) -> Result<Outcome, ModelError> {
        // Construction cannot delegate mutation of published records to a helper.
        if !self.construction_stack.is_empty() {
            return Err(ModelError::new(
                "constructor cannot call an ordinary function",
            ));
        }
        let program = self.program;
        let function = program
            .body
            .iter()
            .find_map(|operation| match operation {
                Op::Function(function) if function.name == name => Some(function),
                _ => None,
            })
            .ok_or_else(|| ModelError::new(format!("unknown ordinary function {name}")))?;
        // The model rejects unbounded host-stack work even for malformed inputs.
        if self.ordinary_stack.len() >= 64 || self.ordinary_stack.contains(&function.name.as_str())
        {
            return Err(ModelError::new(
                "recursive or excessive ordinary model call depth",
            ));
        }
        if arguments.len() != function.params.len() {
            return Err(ModelError::new("ordinary model call arity mismatch"));
        }

        let mut bindings = BTreeMap::new();
        let mut bytes = vec![None; arguments.len()];
        // Retain caller authority until every typed capture, including each
        // range check, completes in written argument order.
        for (index, (argument, parameter)) in arguments.iter().zip(&function.params).enumerate() {
            let binding = match (argument, &parameter.kind) {
                (FunctionArgument::Scalar(expression), FunctionParameterKind::U64) => {
                    Binding::Scalar(self.expression(expression, &[], None)?)
                }
                (FunctionArgument::Bool(expression), FunctionParameterKind::Bool) => {
                    Binding::Boolean(self.boolean(expression)?)
                }
                (FunctionArgument::State(name), FunctionParameterKind::State(schema)) => {
                    let reference = self.reference(name)?;
                    if reference.access != Access::Readwrite
                        || self.batches[reference.batch].schema != *schema
                    {
                        return Err(ModelError::new(
                            "ordinary state model argument has wrong schema or permission",
                        ));
                    }
                    Binding::Reference(reference)
                }
                (
                    FunctionArgument::Batch(name),
                    FunctionParameterKind::Batch { record, access },
                ) => {
                    let view = self.batch_view(name, *access)?;
                    if self.batches[view.batch].schema != *record {
                        return Err(ModelError::new(
                            "ordinary batch model argument has wrong schema",
                        ));
                    }
                    Binding::BorrowedBatch(view)
                }
                (
                    FunctionArgument::Bytes { source, ranges },
                    FunctionParameterKind::Bytes | FunctionParameterKind::WritableBytes,
                ) => {
                    let mut input = self.capture_byte_source(source, &[])?;
                    if parameter.kind == FunctionParameterKind::Bytes {
                        input.access = Access::Readonly;
                    } else if input.access != Access::Readwrite {
                        return Err(ModelError::new(
                            "ordinary byte argument has insufficient permission",
                        ));
                    }
                    self.narrow_byte_input(&mut input, ranges, &[])?;
                    bytes[index] = Some(input);
                    continue;
                }
                _ => return Err(ModelError::new("ordinary model argument has wrong kind")),
            };
            if bindings.insert(parameter.name.clone(), binding).is_some() {
                return Err(ModelError::new("duplicate ordinary model parameter"));
            }
        }

        let outer_scopes = std::mem::replace(&mut self.scopes, vec![bindings]);
        let outer_bytes = std::mem::replace(&mut self.ordinary_bytes, bytes);
        let family = function
            .result
            .as_ref()
            .map(|result| result.family.as_str());
        let outer_family = std::mem::replace(&mut self.ordinary_result_family, family);
        let outer_return = self.returned.take();
        self.ordinary_stack.push(&function.name);
        let result = self.execute_ops(&function.body);
        // Restore the suspended caller frame on either result. Published record
        // writes use shared semantic identities and remain visible after return.
        self.ordinary_stack.pop();
        self.scopes = outer_scopes;
        self.ordinary_bytes = outer_bytes;
        self.ordinary_result_family = outer_family;
        let returned = std::mem::replace(&mut self.returned, outer_return);
        let transferred = self.loop_transfer.take();
        result?;
        if transferred.is_some() {
            return Err(ModelError::new(
                "loop transfer escaped an ordinary model function",
            ));
        }
        let outcome = returned
            .ok_or_else(|| ModelError::new("ordinary model function completed without a return"))?;
        // The outcome must agree with the declared signature, including family identity.
        let success = function.result.as_ref().map(|result| result.success);
        match (function.returns_bool, success, &outcome) {
            (true, None, Outcome::Boolean(_))
            | (false, None, Outcome::Scalar(_))
            | (false, Some(ResultSuccess::U64), Outcome::Success(_))
            | (false, Some(ResultSuccess::Unit), Outcome::SuccessUnit) => Ok(outcome),
            (false, Some(_), Outcome::Failure { family: actual, case, payload }) if family == Some(actual.as_str())
                && program.body.iter().any(|operation| matches!(operation,
                    Op::ErrorFamily(declared) if declared.name == *actual
                        && declared.cases.iter().any(|item| item.name == *case && item.payload == payload.is_some()))) => Ok(outcome),
            _ => Err(ModelError::new("ordinary model return disagrees with its signature")),
        }
    }

    /// Evaluate exactly one handler arm using a fresh payload-binding scope.
    pub(super) fn handle_result(
        &mut self,
        name: &str,
        arguments: &[FunctionArgument],
        arms: &[ResultArm],
    ) -> Result<u64, ModelError> {
        let outcome = self.invoke(name, arguments)?;
        self.evaluate_result_arm(&outcome, arms)
    }

    /// Read an outcome saved in the current caller's lexical scope.
    pub(super) fn saved_result(&self, name: &str) -> Result<Outcome, ModelError> {
        let Binding::SavedResult(value) = self.binding(name)? else {
            return Err(ModelError::new(format!("{name} is not a saved result")));
        };
        Ok(value)
    }

    /// Select an expression arm from an already evaluated semantic outcome.
    pub(super) fn evaluate_result_arm(
        &mut self,
        outcome: &Outcome,
        arms: &[ResultArm],
    ) -> Result<u64, ModelError> {
        let (arm, binding) =
            select_result_arm(outcome, arms.iter().map(|arm| (&arm.pattern, arm)))?;
        self.evaluate_arm(&arm.value, binding)
    }

    /// Execute one statement arm with the original result's active local.
    pub(super) fn handle_result_blocks(
        &mut self,
        name: &str,
        arguments: &[FunctionArgument],
        arms: &[ResultBlockArm],
    ) -> Result<(), ModelError> {
        let outcome = self.invoke(name, arguments)?;
        self.execute_result_arm(&outcome, arms)
    }

    /// Execute a selected block from a semantic outcome without invoking its source.
    pub(super) fn execute_result_arm(
        &mut self,
        outcome: &Outcome,
        arms: &[ResultBlockArm],
    ) -> Result<(), ModelError> {
        let (arm, binding) =
            select_result_arm(outcome, arms.iter().map(|arm| (&arm.pattern, arm)))?;
        let mut scope = BTreeMap::new();
        if let Some((name, binding)) = binding {
            scope.insert(name.clone(), binding);
        }
        self.scopes.push(scope);
        let result = self.execute_ops(&arm.body);
        self.scopes.pop();
        result
    }
}

/// Select by the names of the active outcome rather than native ordinals.
fn select_result_arm<'a, A>(
    outcome: &Outcome,
    mut arms: impl Iterator<Item = (&'a ResultPattern, &'a A)>,
) -> Result<(&'a A, ArmBinding<'a>), ModelError> {
    arms.find_map(|(pattern, arm)| match (outcome, pattern) {
        (Outcome::SuccessUnit, ResultPattern::Ok(None)) => Some((arm, None)),
        (Outcome::Success(value), ResultPattern::Ok(name)) => Some((
            arm,
            name.as_ref().map(|name| (name, Binding::Scalar(*value))),
        )),
        (
            Outcome::Failure {
                family,
                case,
                payload,
            },
            ResultPattern::Error {
                family: wanted_family,
                case: wanted_case,
                binding,
            },
        ) if family == wanted_family && case == wanted_case => Some((
            arm,
            binding
                .as_ref()
                .zip(*payload)
                .map(|(name, value)| (name, Binding::Scalar(value))),
        )),
        (Outcome::Failure { .. }, ResultPattern::Fallback(name)) => {
            Some((arm, name.as_ref().map(|name| (name, Binding::Error))))
        }
        _ => None,
    })
    .ok_or_else(|| ModelError::new("model handler has no arm for the active outcome"))
}

/// Optional arm-local name with the active scalar or opaque failure.
type ArmBinding<'a> = Option<(&'a String, Binding)>;
