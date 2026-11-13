//! Lexical bindings and owned semantic storage for the independent region oracle.
//!
//! References are stable logical handles into model batches, not Rust borrows of
//! vectors and not native pointers. Model vectors may allocate freely: none of
//! this code is linked into generated Holo programs or used as timing evidence.

use super::ModelError;
use super::layout::Encoding;
use super::ordinary::Outcome;
use super::value::Record;
use holo::ir::{Access, Op, Program};
use std::collections::BTreeMap;

/// One lexical binding with its independent semantic value.
#[derive(Clone, Debug)]
pub(super) enum Binding {
    /// An unsigned scalar, including copied status/address observations.
    Scalar(u64),
    /// An arm-local named error cannot supply a scalar or a storage reference.
    Error,
    /// Index into the model's owned region table.
    Region(usize),
    /// Index into the model's owned batch table.
    Batch(usize),
    /// Rooted record identity and path access permission.
    Reference(Reference),
}

/// A checked semantic reference, independent of native backing addresses.
#[derive(Clone, Debug)]
pub(super) struct Reference {
    /// The exact batch lifetime/generation that owns the record.
    pub batch: usize,
    /// Published record index within that batch.
    pub index: usize,
    /// Writes are permitted only through a readwrite path.
    pub access: Access,
}

/// One captured byte borrow into already-published semantic model storage.
///
/// This is a stable logical identity and extent, not a clone of source bytes or
/// a Rust/native pointer. Forwarding it clones only the bounded field path.
#[derive(Clone, Debug)]
pub(super) struct ByteInput {
    /// Exact batch lifetime/generation owning the initialized source record.
    pub batch: usize,
    /// Published source record within that batch, stable across later appends.
    pub index: usize,
    /// Nested logical field path ending at the borrowed Bytes payload.
    pub path: Vec<String>,
    /// Extent captured at call-argument evaluation, never a payload snapshot.
    pub len: u64,
}

/// One evaluated typed argument in a synchronous model producer call frame.
#[derive(Clone, Debug)]
pub(super) enum Input {
    /// Copied unsigned value, independent of later alias-visible source writes.
    Scalar(u64),
    /// Readonly byte borrow retaining its original published storage identity.
    Bytes(ByteInput),
}

/// Finite model backing retained across repeated batch resets.
#[derive(Debug)]
pub(super) struct Region {
    /// Maximum usable bytes, independently of host allocator/page behavior.
    pub capacity: u64,
    /// Mathematical byte marker; reservations check it before publication.
    pub used: u128,
    /// Whether this backing owner's lexical lifetime is still active.
    pub active: bool,
}

/// A record and its stable offset in model backing.
#[derive(Debug)]
pub(super) struct Stored {
    /// Byte offset from the owning region's start, unchanged by later appends.
    pub offset: u128,
    /// Complete initialized values; failed constructions never enter this list.
    pub record: Record,
}

/// One live or ended batch generation owning a region tail.
#[derive(Debug)]
pub(super) struct Batch {
    /// The region whose tail belongs to this batch while active.
    pub region: usize,
    /// Expected result schema for appended constructors.
    pub schema: String,
    /// Marker restored on reset or lexical batch exit.
    pub start: u128,
    /// Fully published records in append order.
    pub records: Vec<Stored>,
    /// Whether a live reference may still access this batch generation.
    pub active: bool,
}

/// All test-model state, including immutable input and independent owned values.
pub(super) struct State<'a> {
    /// Shared input descriptions only; compiler checking/lowering is never called.
    pub program: &'a Program,
    /// Explicit independent model encoding; no native compiler plan is shared.
    pub encoding: Encoding,
    /// Lexical scopes searched from innermost to outermost.
    pub scopes: Vec<BTreeMap<String, Binding>>,
    /// Region identities are never recycled within one model execution.
    pub regions: Vec<Region>,
    /// Distinct batch generations remain identifiable after scope exit/reset.
    pub batches: Vec<Batch>,
    /// Deterministic values emitted by Print operations.
    pub output: Vec<u64>,
    /// Whether successful entry completion has stopped all remaining operations.
    pub exited: bool,
    /// Remaining bounded evaluation work for adversarial test inputs.
    pub budget: usize,
    /// Active constructor names, rejecting recursive/pathological oracle inputs.
    pub construction_stack: Vec<String>,
    /// Active ordinary names, bounding model stack usage and rejecting cycles.
    pub ordinary_stack: Vec<&'a str>,
    /// Declared family of the current ordinary result frame, if any.
    pub ordinary_result_family: Option<&'a str>,
    /// Immutable logical byte borrows indexed by ordinary formal position.
    pub ordinary_bytes: Vec<Option<ByteInput>>,
    /// Ordinary or typed return of the current function, if complete.
    pub returned: Option<Outcome>,
}

impl<'a> State<'a> {
    /// Begin a model execution with borrowed logical input, explicit independent
    /// encoding, one root scope and a bounded test-work budget. Allocations are
    /// model-owned; constructing this state executes no native code or plan helper.
    pub fn new(program: &'a Program, encoding: Encoding) -> Self {
        Self {
            program,
            encoding,
            scopes: vec![BTreeMap::new()],
            regions: Vec::new(),
            batches: Vec::new(),
            output: Vec::new(),
            exited: false,
            budget: 1_000_000,
            construction_stack: Vec::new(),
            ordinary_stack: Vec::new(),
            ordinary_result_family: None,
            ordinary_bytes: Vec::new(),
            returned: None,
        }
    }

    /// Preserve an exact family or wrap the source case with a direct child's alias.
    /// The result owns names for model dispatch. An undeclared conversion is a
    /// model error. This uses nominal aliases independently of native tag arithmetic.
    pub(super) fn lift_error(
        &self,
        source: String,
        case: String,
        target: &str,
    ) -> Result<(String, String), ModelError> {
        // Exact propagation preserves both names without extra model allocation.
        if source == target {
            return Ok((source, case));
        }
        let alias = self
            .program
            .body
            .iter()
            .find_map(|operation| match operation {
                Op::ErrorFamily(family) if family.name == target => family
                    .children
                    .iter()
                    .find(|child| child.family == source)
                    .map(|child| child.alias.as_str()),
                _ => None,
            })
            .ok_or_else(|| ModelError::new("model try has no declared error child"))?;
        Ok((target.to_owned(), format!("{alias}.{case}")))
    }

    /// Charge one test-model step, rejecting runaway inputs as inconclusive.
    pub fn tick(&mut self) -> Result<(), ModelError> {
        // Exhaustion describes the oracle's own execution limit, not a Holo
        // capacity error and not a successful native semantic outcome.
        if self.budget == 0 {
            return Err(ModelError::new("reference model step budget exhausted"));
        }
        self.budget -= 1;
        Ok(())
    }

    /// Clone a small binding from its nearest lexical declaration.
    pub fn binding(&self, name: &str) -> Result<Binding, ModelError> {
        // Reverse lookup preserves ordinary lexical shadowing independently of
        // how the compiler represents names or lifetime facts.
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .cloned()
            .ok_or_else(|| ModelError::new(format!("unbound model name {name}")))
    }

    /// Introduce one initialized binding in the current scope, rejecting repeats.
    pub fn declare(&mut self, name: &str, binding: Binding) -> Result<(), ModelError> {
        let scope = self
            .scopes
            .last_mut()
            .expect("model always has a root scope");
        // A duplicate in one lexical frame is malformed input rather than an
        // implicit assignment that could obscure a reference lifetime.
        if scope.contains_key(name) {
            return Err(ModelError::new(format!("duplicate model binding {name}")));
        }
        scope.insert(name.to_owned(), binding);
        Ok(())
    }

    /// Replace the nearest declared binding while preserving outer scope lookup.
    pub fn replace(&mut self, name: &str, binding: Binding) -> Result<(), ModelError> {
        // Search the same lexical order as reads; assignment cannot create a new
        // local and accidentally hide the binding intended to be updated.
        for scope in self.scopes.iter_mut().rev() {
            // Stop at the first owning frame, without changing any other scope.
            if let Some(slot) = scope.get_mut(name) {
                *slot = binding;
                return Ok(());
            }
        }
        Err(ModelError::new(format!(
            "assignment to unknown model name {name}"
        )))
    }

    /// Read an initialized unsigned lexical value or report a wrong-kind input.
    pub fn scalar(&self, name: &str) -> Result<u64, ModelError> {
        // Typed alternatives keep invalid compiler inputs from becoming model
        // pointer arithmetic or unchecked Rust casts.
        match self.binding(name)? {
            Binding::Scalar(value) => Ok(value),
            _ => Err(ModelError::new(format!("{name} is not a scalar"))),
        }
    }

    /// Resolve a currently active backing owner by lexical name.
    pub fn region_id(&self, name: &str) -> Result<usize, ModelError> {
        // An ended owner is never silently revived by a retained integer ID.
        match self.binding(name)? {
            Binding::Region(id) if self.regions[id].active => Ok(id),
            _ => Err(ModelError::new(format!("{name} is not a live region"))),
        }
    }

    /// Resolve a currently active published collection by lexical name.
    pub fn batch_id(&self, name: &str) -> Result<usize, ModelError> {
        // Batch IDs distinguish generations even if their physical backing is
        // reused; this is a model identity check rather than a runtime counter.
        match self.binding(name)? {
            Binding::Batch(id) if self.batches[id].active => Ok(id),
            _ => Err(ModelError::new(format!("{name} is not a live batch"))),
        }
    }

    /// Resolve a reference whose batch and element remain published and active.
    pub fn reference(&self, name: &str) -> Result<Reference, ModelError> {
        // Both lexical ownership and element range are checked before any model
        // projection. This does not reuse the compiler's lifetime algorithm.
        match self.binding(name)? {
            Binding::Reference(reference)
                if self.batches[reference.batch].active
                    && reference.index < self.batches[reference.batch].records.len() =>
            {
                Ok(reference)
            }
            _ => Err(ModelError::new(format!("{name} is not a live reference"))),
        }
    }
}
