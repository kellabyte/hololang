//! Independent semantics for synchronous ordinary calls over caller storage.
//!
//! Model borrows retain batch/record/path identities and checked prefix lengths.
//! Each call receives a fresh lexical frame while updating the same owned model
//! records as its caller. Host allocations here are never part of native timing.

use std::collections::BTreeMap;

use holo::ir::{
    Access, BytesSource, FunctionArgument, FunctionParameterKind, Op, ResultArm, ResultPattern,
};

use super::{
    ModelError,
    state::{Binding, ByteInput, State},
    value::Value,
};

/// Independent ordinary-call outcomes, distinguished without machine tags.
#[derive(Debug)]
pub(super) enum Outcome {
    /// Result of a plain scalar helper.
    Scalar(u64),
    /// Success of a typed helper with the entire unsigned range available.
    Success(u64),
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
                "typed model call requires handling or forwarding",
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
        // prefix check, completes in written argument order.
        for (index, (argument, parameter)) in arguments.iter().zip(&function.params).enumerate() {
            let binding = match (argument, &parameter.kind) {
                (FunctionArgument::Scalar(expression), FunctionParameterKind::U64) => {
                    Binding::Scalar(self.expression(expression, &[], None)?)
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
                (FunctionArgument::Bytes { source, prefix }, FunctionParameterKind::Bytes) => {
                    let mut input = match source {
                        BytesSource::Param(index) => self.byte_input(&[], *index)?.clone(),
                        BytesSource::Field { reference, path } => {
                            let identity = self.reference(reference)?;
                            let Value::Bytes(payload) = self.projection(reference, path)?.0 else {
                                return Err(ModelError::new(
                                    "ordinary byte argument projects a non-byte field",
                                ));
                            };
                            ByteInput {
                                batch: identity.batch,
                                index: identity.index,
                                path: path.clone(),
                                len: payload.len() as u64,
                            }
                        }
                    };
                    // The captured logical length, which may already be a
                    // prefix, is the limit for a nested narrower borrow.
                    if let Some(prefix) = prefix {
                        let length = self.expression(prefix, &[], None)?;
                        if length > input.len {
                            return Err(ModelError::new("byte prefix bounds trap"));
                        }
                        input.len = length;
                    }
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
        let outer_family =
            std::mem::replace(&mut self.ordinary_result_family, function.result.as_deref());
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
        result?;
        let outcome = returned
            .ok_or_else(|| ModelError::new("ordinary model function completed without a return"))?;
        // The outcome must agree with the declared signature, including family identity.
        match (&function.result, &outcome) {
            (None, Outcome::Scalar(_)) | (Some(_), Outcome::Success(_)) => Ok(outcome),
            (Some(expected), Outcome::Failure { family, case, payload }) if expected == family
                && program.body.iter().any(|operation| matches!(operation,
                    Op::ErrorFamily(declared) if declared.name == *family
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
        // Match the names of the active outcome; the model never uses native ordinals.
        let (arm, binding) = arms
            .iter()
            .find_map(|arm| match (&outcome, &arm.pattern) {
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
            .ok_or_else(|| ModelError::new("model handler has no arm for the active outcome"))?;
        self.evaluate_arm(&arm.value, binding)
    }
}
