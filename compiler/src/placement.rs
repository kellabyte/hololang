//! One-site automatic backing selection for programmatic checked IR experiments.
//!
//! Elaboration inserts an ordinary finite region and an ordinary scoped result
//! before verification. Ownership, failure dispatch, and native storage retain
//! their existing implementations. The compiler moves IR vectors and allocates
//! a private binding name. The selection itself adds no runtime state.

use crate::check::{self, CheckError, OperationIdentity, PrivatePlacement};
use crate::ir::{Argument, ConstructArm, Expr, Op, Program};
use std::fmt;

/// Name outside the source identifier grammar, reserved during prototype admission.
pub(crate) const PRIVATE_REGION: &str = "$holo.scoped";

/// Profile-owned acquisition boundary and one constructor with no region operand.
#[derive(Debug)]
pub struct ScopedPlacement {
    /// First entry operation in the scope that will own the finite backing.
    /// The earlier prefix, including outermost definitions, stays outside.
    pub acquire_at: usize,
    /// Logical capacity evaluated in the entry prefix's lexical environment.
    pub capacity: Expr,
    /// Test-only acquisition fault. It takes the existing recovery edge.
    pub fail_acquire: bool,
    /// Operations executed when backing acquisition fails.
    pub on_failure: Vec<Op>,
    /// Operation indexes relative to the acquired suffix. Intermediate indexes
    /// select a scope, counted or batch iteration body, batch, successful
    /// region, or refill data body. The final index can equal the body length.
    pub site: Vec<usize>,
    /// Typed destination constructor to call at the selected site.
    pub producer: String,
    /// Actuals evaluated once, in order, after resolving the final slot.
    pub args: Vec<Argument>,
    /// Exhaustive lexical arms for the selected construction result.
    pub arms: Vec<ConstructArm>,
}

/// An invalid owner/site selection or an elaborated program rejected by checking.
#[derive(Debug)]
pub enum PlacementError {
    /// The acquisition index is past the entry body's end.
    AcquisitionOutOfBounds,
    /// No insertion position was provided.
    EmptySite,
    /// An enclosing operation or the insertion position is out of bounds.
    SiteOutOfBounds,
    /// An intermediate index selects an operation without an eligible body.
    UnsupportedSiteScope,
    /// The common checker rejected the resulting IR or its private binding use.
    Check(CheckError),
}

impl fmt::Display for PlacementError {
    /// Render the scope selection or existing checker diagnostic without allocating.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AcquisitionOutOfBounds => {
                formatter.write_str("placement acquisition index is out of bounds")
            }
            Self::EmptySite => formatter.write_str("placement site requires an insertion index"),
            Self::SiteOutOfBounds => formatter.write_str("placement site index is out of bounds"),
            Self::UnsupportedSiteScope => {
                formatter.write_str("placement site must descend through an eligible body")
            }
            Self::Check(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PlacementError {
    /// Expose the underlying checker failure when selection reached verification.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // Structural selection errors originate here and have no nested cause.
        match self {
            Self::Check(error) => Some(error),
            _ => None,
        }
    }
}

/// Insert one private backing scope and its sole automatic construction site.
///
/// Consumes the IR and request. The prefix stays outside the acquisition
/// boundary, and the original suffix moves into its success arm. A structural
/// or checker failure returns a diagnostic without publishing partial output.
/// Success returns ordinary IR. Call `check::check` to obtain the borrowed
/// authorization required for native lowering. No runtime allocation is
/// added beyond the explicit region that this experiment models.
pub fn place_scoped_aggregate(
    mut program: Program,
    request: ScopedPlacement,
) -> Result<Program, PlacementError> {
    // Splitting at the end admits a site in an otherwise empty owner body.
    if request.acquire_at > program.body.len() {
        return Err(PlacementError::AcquisitionOutOfBounds);
    }
    let [ancestors @ .., index] = request.site.as_slice() else {
        return Err(PlacementError::EmptySite);
    };
    let mut body = program.body.split_off(request.acquire_at);
    let siblings = site_body(&mut body, ancestors)?;
    // The final index is an insertion position, unlike an enclosing index.
    if *index > siblings.len() {
        return Err(PlacementError::SiteOutOfBounds);
    }
    siblings.insert(
        *index,
        Op::ConstructHandled {
            region: PRIVATE_REGION.to_owned(),
            producer: request.producer,
            args: request.args,
            arms: request.arms,
        },
    );
    // The selected operation lives in a Vec allocation inside body. Moving
    // the Vec into the new owner preserves that identity until verification.
    let site = std::ptr::from_ref(&siblings[*index]);
    program.body.push(Op::Region {
        name: PRIVATE_REGION.to_owned(),
        capacity: request.capacity,
        fail_acquire: request.fail_acquire,
        body,
        on_failure: request.on_failure,
    });
    let owner = std::ptr::from_ref(&program.body[request.acquire_at]);
    check::validate_private_placement(
        &program,
        &PrivatePlacement {
            name: PRIVATE_REGION,
            owner: OperationIdentity::Address(owner),
            site: OperationIdentity::Address(site),
        },
    )
    .map_err(PlacementError::Check)?;
    Ok(program)
}

/// Borrow the site siblings through the indicated lexical success bodies.
/// Following a path never edits an existing operation or enters a recovery arm.
fn site_body<'p>(
    mut body: &'p mut Vec<Op>,
    ancestors: &[usize],
) -> Result<&'p mut Vec<Op>, PlacementError> {
    // Every index descends one level, preserving the dominating owner scope.
    for &index in ancestors {
        let scope = body.get_mut(index).ok_or(PlacementError::SiteOutOfBounds)?;
        // Single bodies and the positive side of a resource operation have
        // unambiguous placement. Other branch selection needs a future API.
        body = match scope {
            Op::Scope(body)
            | Op::Repeat { body, .. }
            | Op::ForEach { body, .. }
            | Op::Batch { body, .. }
            | Op::Region { body, .. } => body,
            Op::Refill(refill) => &mut refill.data_body,
            _ => return Err(PlacementError::UnsupportedSiteScope),
        };
    }
    Ok(body)
}
