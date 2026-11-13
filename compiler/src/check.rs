//! Validation boundary between unchecked construction IR and native lowering.
//!
//! Definitions, constructor initialization, lexical roots, and access rights are
//! checked before a backend can obtain a `CheckedProgram`. This validates only
//! the documented structured IR subset, not the future source language. Compiler
//! maps and temporary work lists allocate host memory; no runtime ownership or
//! aliasing promise is inferred from the spelling of an access permission.

#[path = "check/definitions.rs"]
mod definitions;
#[path = "check/expressions.rs"]
mod expressions;
#[path = "check/scopes.rs"]
mod scopes;

use crate::ir::Program;
use std::fmt;

/// Unchecked IR rejected before lowering, with a human-readable reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckError {
    /// Contextual explanation; IR has no source spans until a frontend supplies them.
    message: String,
    /// Zero-based preorder position of the innermost rejected entry operation.
    /// In an ordinary function error this instead indexes that function body.
    operation_index: Option<usize>,
    /// Name of the ordinary definition enclosing a function-local error.
    function: Option<String>,
}

impl CheckError {
    /// Own an error explanation without mutating the rejected input program.
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            operation_index: None,
            function: None,
        }
    }

    /// Borrow the validation explanation without allocation or ownership transfer.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Return the rejected entry operation's stable preorder index, if available.
    ///
    /// Region success precedes failure, conditional then precedes else, and
    /// refill data precedes nondata.
    /// Definition validation has no associated entry operation and returns `None`.
    pub fn operation_index(&self) -> Option<usize> {
        // A helper's local numbering is independent of entry's operation order.
        if self.function.is_some() {
            None
        } else {
            self.operation_index
        }
    }

    /// Locate a rejected ordinary definition and optionally an operation in its body.
    /// The local preorder index uses the same branch ordering as entry diagnostics.
    pub fn function_location(&self) -> Option<(&str, Option<usize>)> {
        self.function
            .as_deref()
            .map(|name| (name, self.operation_index))
    }

    /// Attach `index` to an unlocated error, preserving a nested operation's index.
    ///
    /// This consumes only the diagnostic; it does not mutate or wrap input IR.
    fn at_operation(mut self, index: usize) -> Self {
        // The first attachment belongs to the innermost failing operation, while
        // propagation through enclosing scopes must retain that precise location.
        if self.operation_index.is_none() {
            self.operation_index = Some(index);
        }
        self
    }

    /// Associate the existing cause and local operation position with its helper.
    fn in_function(mut self, name: &str) -> Self {
        self.message = format!("ordinary function {name}: {}", self.message);
        self.function = Some(name.to_owned());
        self
    }
}

impl fmt::Display for CheckError {
    /// Write the stored explanation to `formatter`, returning its I/O result.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CheckError {}

/// Borrowed input whose structured-IR obligations passed the current checker.
///
/// Construction is private: a backend cannot accidentally accept an unchecked
/// `Program` through this API. The proof is scoped to the borrowed immutable input
/// and does not claim that LLVM lowering or native execution is itself proved.
#[derive(Debug)]
pub struct CheckedProgram<'a> {
    /// Immutable input whose definitions and entry operations were validated.
    program: &'a Program,
    /// Which native descriptor capabilities the checked entry uses.
    hosted: HostedIo,
}

/// Compile-time selection of runtime support for the sole host descriptor.
#[derive(Debug, Default)]
pub(crate) struct HostedIo {
    /// Refill or an explicit wait for readable data occurs in checked entry.
    pub(crate) input: bool,
    /// Send or an explicit wait for writable data occurs in checked entry.
    pub(crate) output: bool,
    /// Entry explicitly advances an optional host-owned listener session.
    pub(crate) session: bool,
    /// Entry takes sole close authority for one admitted connected TCP stream.
    pub(crate) tcp: bool,
}

impl<'a> CheckedProgram<'a> {
    /// Borrow validated input for lowering or independent model execution.
    ///
    /// This performs no copying or allocation and cannot mutate the checked input.
    pub fn program(&self) -> &'a Program {
        self.program
    }

    /// Return whether the checked entry requires the descriptor-owning host.
    /// Lowering uses this compiler fact without mutating or allocating storage.
    pub(crate) fn hosted_input(&self) -> bool {
        self.hosted.input || self.hosted.output || self.hosted.session || self.hosted.tcp
    }

    /// Return whether the checked entry requires socket output support.
    /// Lowering leaves input-only modules and their runtime linkage unchanged.
    pub(crate) fn hosted_output(&self) -> bool {
        self.hosted.output
    }

    /// Return whether the entry requires the optional loopback listener host.
    /// Other modules keep their inherited input ABI and linkage.
    pub(crate) fn hosted_session(&self) -> bool {
        self.hosted.session
    }

    /// Return whether the entry receives a single owned TCP stream from its host.
    /// Lowering uses a dedicated ABI and emits lexical descriptor cleanup.
    pub(crate) fn hosted_tcp(&self) -> bool {
        self.hosted.tcp
    }
}

/// Validate definitions, destination producers, roots, and operations in `program`.
///
/// Returns a borrowed authorization for lowering, or the first contextual error.
/// This uses compiler-side lookup maps and lexical environments; it adds no
/// per-reference counters or reference enumeration to the generated executable.
pub fn check(program: &Program) -> Result<CheckedProgram<'_>, CheckError> {
    check_with_placement(program, None)
}

/// Validate an ordinary program or one with a single private placement pair.
/// Placement restricts lookup during checking. The returned token authorizes
/// the same immutable IR and backend as an explicit region.
pub(crate) fn check_with_placement<'p>(
    program: &'p Program,
    placement: Option<&PrivatePlacement>,
) -> Result<CheckedProgram<'p>, CheckError> {
    let definitions = definitions::validate(program)?;
    let hosted = scopes::validate(&program.body, &definitions, placement)?;
    Ok(CheckedProgram { program, hosted })
}

/// Identify the generated owner and its single authorized use during placement.
/// Site identities exist only during validation and never reach the backend.
pub(crate) struct PrivatePlacement {
    /// Internal binding excluded from ordinary entry operands and declarations.
    pub(crate) name: &'static str,
    /// Exact generated region operation allowed to declare the binding.
    pub(crate) owner: OperationIdentity,
    /// Exact generated construction operation allowed to resolve the binding.
    pub(crate) site: OperationIdentity,
}

/// Identify one exact operation through either stable vector addresses or the
/// same preorder numbering used by source diagnostics.
pub(crate) enum OperationIdentity {
    /// Programmatic placement has the exact allocation address after insertion.
    /// Checking compares the pointer without dereferencing it.
    Address(*const crate::ir::Op),
    /// Source lowering records the operation's final index with its source span.
    Preorder(usize),
}

impl OperationIdentity {
    /// Match this identity against the current borrowed operation and index.
    fn matches(&self, operation: &crate::ir::Op, index: usize) -> bool {
        match self {
            Self::Address(address) => std::ptr::eq(*address, operation),
            Self::Preorder(expected) => *expected == index,
        }
    }
}

/// Verify an elaborated candidate and prove that its private owner cannot be
/// captured or used by other entry operations. Both operation identities belong
/// to the borrowed input. This pass publishes no IR or native authorization.
pub(crate) fn validate_private_placement(
    program: &Program,
    placement: &PrivatePlacement,
) -> Result<(), CheckError> {
    check_with_placement(program, Some(placement)).map(drop)
}
