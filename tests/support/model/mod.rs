//! Independent executable semantics for the first region construction IR.
//!
//! This oracle owns ordinary safe Rust values and derives fixture byte use with
//! mathematical integers. It never calls the compiler checker, storage planner,
//! or LLVM emitter. A bounded step budget protects the test runner itself; hitting
//! that limit is an inconclusive model error, never an expected native outcome.

mod construction;
mod execution;
mod expression;
mod layout;
mod ordinary;
mod state;
mod value;

use holo::ir::Program;
use holo::layout::LayoutMode;
use layout::Encoding;
use state::State;
use std::fmt;

/// An unsupported/invalid oracle input or a specified execution trap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelError {
    /// Human-readable context retained with a failing generated program.
    pub message: String,
}

impl ModelError {
    /// Own a diagnostic message; this allocates only in test infrastructure.
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ModelError {
    /// Render the retained message for regression reports without altering state.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ModelError {}

/// Evaluate a complete program and return its ordered decimal-output values.
///
/// The caller passes the same IR input to the real compiler independently. The
/// oracle assumes small realizable backing allocations; injected acquisition
/// failure is modeled explicitly, while host allocator availability is not.
/// Physical address integers must not be printed as differential expected values;
/// synthetic disjoint bases are only witnesses for within-run identity checks.
pub fn run(program: &Program) -> Result<Vec<u64>, ModelError> {
    run_with_layout(program, LayoutMode::Wide)
}

/// Evaluate explicit experimental storage with independently derived schema rules.
/// This does not consume any compiler plan; the selection affects only mathematical
/// byte accounting and width-failure behavior, never owned semantic values.
pub fn run_with_layout(program: &Program, mode: LayoutMode) -> Result<Vec<u64>, ModelError> {
    // Sharing an explicit selection label does not share any compiler layout computation.
    let encoding = match mode {
        LayoutMode::Wide => Encoding::Wide,
        LayoutMode::Derived64 => Encoding::Derived64,
        LayoutMode::Compact32 => Encoding::Compact32,
    };
    let mut state = State::new(program, encoding);
    state.execute_ops(&program.body)?;
    if state.loop_transfer.is_some() {
        return Err(ModelError::new("loop transfer escaped model entry"));
    }
    Ok(state.output)
}
