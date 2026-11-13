//! Explicitly experimental source adapter for the existing checked storage core.
//!
//! Parsing preserves preferred headers and spans. Lowering admits only operations
//! representable by the core, without implicit allocation or aggregate copying.
//! Temporary `core` APIs remain opt-in rather than permanent language decisions.

pub mod ast;
mod functions;
mod lower;
mod modules;
pub mod parse;

pub use modules::{
    SourceProject, check_experimental_library_package,
    check_experimental_library_package_with_naming, parse_experimental_package,
    parse_experimental_project,
};

use crate::{
    check, ir,
    source::{Diagnostic, Span},
};
use std::collections::HashMap;

/// Source locations for one ordinary definition and its independently checked body.
#[derive(Debug)]
struct FunctionSpans {
    /// Declaration span used for signature or whole-body errors.
    declaration: Span,
    /// Preorder operation spans within this helper only.
    body: Vec<Span>,
}

/// Owned, lowered source input with immutable operation-to-source correspondence.
/// A successful parse is not evidence of lifetime or access correctness: call check.
#[derive(Debug)]
pub struct SourceProgram {
    /// The single unchecked IR path shared with structured clients.
    program: ir::Program,
    /// Preorder spans matching the core checker's entry-operation traversal.
    operation_spans: Vec<Span>,
    /// Separately indexed ordinary bodies and their declarations.
    function_spans: HashMap<String, FunctionSpans>,
    /// Definition/whole-program fallback when no operation is responsible.
    fallback: Span,
    /// Advisory declarations in original spelling and source order.
    naming_diagnostics: Vec<Diagnostic>,
    /// Preorder identities of the private region and construction, if present.
    placement: Option<(usize, usize)>,
}

impl SourceProgram {
    /// Borrow lowered input without permitting edits that would invalidate spans.
    /// No checking, copying, or generated execution occurs.
    pub fn program(&self) -> &ir::Program {
        &self.program
    }

    /// Borrow advisory naming diagnostics with original source spans.
    /// Call check before reporting them on valid programs. No I/O or copying occurs.
    pub fn naming_diagnostics(&self) -> &[Diagnostic] {
        &self.naming_diagnostics
    }

    /// Apply the existing schema, access, and lifetime checker, mapping failures
    /// back to source. The returned token alone permits LLVM emission.
    pub fn check(&self) -> Result<check::CheckedProgram<'_>, Diagnostic> {
        let placement = self.placement.map(|(owner, site)| check::PrivatePlacement {
            name: crate::placement::PRIVATE_REGION,
            owner: check::OperationIdentity::Preorder(owner),
            site: check::OperationIdentity::Preorder(site),
        });
        check::check_with_placement(&self.program, placement.as_ref()).map_err(|error| {
            // Helper body indexes cannot name entry spans. Definition errors
            // without a body operation use that helper's declaration.
            let span = if let Some((name, index)) = error.function_location()
                && let Some(function) = self.function_spans.get(name)
            {
                index
                    .and_then(|index| function.body.get(index).copied())
                    .unwrap_or(function.declaration)
            } else {
                error
                    .operation_index()
                    .and_then(|index| self.operation_spans.get(index).copied())
                    .unwrap_or(self.fallback)
            };
            Diagnostic::new(span, error.to_string())
        })
    }
}

/// Parse and lower the opt-in source subset without executing it or discovering LLVM.
/// Returns the first syntax/subset diagnostic. Call SourceProgram::check before
/// trusting storage semantics; compiler-owned trees/vectors allocate host memory.
pub fn parse_experimental(source: &str) -> Result<SourceProgram, Diagnostic> {
    let (module, naming_diagnostics) = parse::parse_with_naming(source, true)?;
    lower_module(&module, naming_diagnostics)
}

/// Lower resolved syntax through the single checked source adapter. The caller
/// retains the original source map and still checks the resulting program. The
/// compiler owns naming diagnostics without putting them in the checked IR.
fn lower_module(
    module: &ast::Module,
    mut naming_diagnostics: Vec<Diagnostic>,
) -> Result<SourceProgram, Diagnostic> {
    let definitions = functions::lower(module)?;
    let mut program = lower::lower(module, definitions)?;
    // The virtual span order is the original source order across all files.
    naming_diagnostics.sort_unstable_by_key(|diagnostic| diagnostic.span.start);
    program.naming_diagnostics = naming_diagnostics;
    Ok(program)
}

/// Lower namespace declarations and run the common checker. The checked
/// authorization is consumed here, so this path cannot produce an executable.
fn check_library_module(module: &ast::Module) -> Result<(), Diagnostic> {
    let definitions = functions::lower(module)?;
    lower::lower_library(module, definitions)?.check().map(drop)
}
