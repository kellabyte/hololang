//! Source lowering for named and compiler-selected scoped aggregate destinations.
//!
//! Construction schedules the actuals behind destination capture and introduces
//! a readonly record reference only in its success block. The checked IR owns
//! tail exclusivity, rollback and lifetime validation. The frontend allocates
//! only compiler data for the ordered operands, arms and source mappings.

use super::{Block, Kind, Lowerer, SourcePlacement, arity, ast, core_call};
use crate::{
    ir,
    placement::PRIVATE_REGION,
    source::{Diagnostic, Span},
};

/// Identify the owner in main's lexical body before region declarations wrap IR tails.
/// The result borrows no syntax, so later lowering may freely mutate its scopes.
pub(super) fn source_placement(
    main: &ast::Function,
) -> Result<Option<SourcePlacement>, Diagnostic> {
    let mut placement = None;
    // Only direct children of main can acquire automatic backing.
    for statement in &main.body {
        if let ast::StatementKind::WithRegion { body, .. } = &statement.kind {
            if placement.is_some() {
                return Err(Diagnostic::new(
                    statement.span,
                    "only one with core.region is supported in experimental main",
                ));
            }
            let mut site = None;
            select_site(body, &mut site)?;
            placement = Some(SourcePlacement {
                owner: statement.span,
                site,
            });
        }
    }
    Ok(placement)
}

/// Find the unique constructor on an admitted success path, retaining its full span.
/// Excluded paths are checked later by ordinary lowering for precise diagnostics.
fn select_site(body: &[ast::Statement], site: &mut Option<Span>) -> Result<(), Diagnostic> {
    // The parser bounds nested blocks. Each admitted child is visited once.
    for statement in body {
        match &statement.kind {
            // Explicit region and batch declarations own the following
            // statements in this slice. Their recovery is never traversed.
            ast::StatementKind::Scope(body)
            | ast::StatementKind::For { body, .. }
            | ast::StatementKind::Refill {
                data_body: body, ..
            } => select_site(body, site)?,
            ast::StatementKind::Construct { subject, .. }
                if core_call(subject).is_some_and(|(_, args)| args.len() == 1) =>
            {
                // Diagnose the second site even if the first visit can be skipped.
                if site.replace(statement.span).is_some() {
                    return Err(Diagnostic::new(
                        statement.span,
                        "only one automatic core.construct is supported in the success body",
                    ));
                }
            }
            // Conditional/loop recovery and constructor handlers are excluded paths.
            _ => {}
        }
    }
    Ok(())
}

impl Lowerer {
    /// Lower a source-selected budget to the ordinary checked region shape.
    /// The private name is absent from source scope, including both child bodies.
    pub(super) fn with_region(
        &mut self,
        capacity: &ast::Expr,
        body: &[ast::Statement],
        on_failure: &[ast::Statement],
        span: Span,
        output: &mut Block,
    ) -> Result<(), Diagnostic> {
        let Some(placement) = self.placement.filter(|placement| placement.owner == span) else {
            return Err(Diagnostic::new(
                span,
                "with core.region is available only directly in experimental main",
            ));
        };
        let capacity = self.scalar(capacity, output)?;
        let failure = self.scoped(on_failure)?;
        let body = self.scoped(body)?;
        // Visit both bodies first so a misplaced site points at its own call.
        if placement.site.is_none() {
            return Err(Diagnostic::new(
                span,
                "with core.region requires one automatic core.construct in its success body",
            ));
        }
        output.push(
            ir::Op::Region {
                name: PRIVATE_REGION.to_owned(),
                capacity,
                fail_acquire: false,
                body: body.ops,
                on_failure: failure.ops,
            },
            span,
            [body.spans, failure.spans],
        );
        Ok(())
    }

    /// Lower one typed constructor into the selected region and its scoped arms.
    ///
    /// The written operands and arm bodies are borrowed syntax. The output gains
    /// one structured operation followed by arm span maps in verifier order.
    /// An unsupported context or operand returns its source diagnostic.
    pub(super) fn construct(
        &mut self,
        subject: &ast::Expr,
        arms: &[ast::ConstructArm],
        span: Span,
        output: &mut Block,
    ) -> Result<(), Diagnostic> {
        // Only entry code can hold this region capability in the current subset.
        if self.ordinary_body {
            return Err(Diagnostic::new(
                span,
                "core.construct is available only in experimental main",
            ));
        }
        let Some(("construct", operands)) = core_call(subject) else {
            unreachable!("construction statement contains its parsed core call");
        };
        let (region, call) = if let [call] = operands {
            // Only the preselected source statement can name the private backing.
            if self.placement.and_then(|placement| placement.site) != Some(span) {
                return Err(Diagnostic::new(
                    span,
                    "automatic core.construct requires an eligible path inside with core.region",
                ));
            }
            (PRIVATE_REGION.to_owned(), call)
        } else {
            // Preserve the checked explicit region form and its diagnostics.
            arity(operands, 2, subject.span)?;
            (self.region(&operands[0])?, &operands[1])
        };
        // The aggregate must be constructed once in its final region slot.
        // An existing value, scalar expression or dynamic callee has no slot ABI.
        let ast::ExprKind::Call { callee, args } = &call.kind else {
            return Err(Diagnostic::new(
                call.span,
                "core.construct requires a named destination-constructor call",
            ));
        };
        let ast::ExprKind::Name(producer) = &callee.kind else {
            return Err(Diagnostic::new(
                callee.span,
                "core.construct requires a statically named destination constructor",
            ));
        };
        let definition = self
            .definitions
            .producers
            .iter()
            .find(|definition| definition.name == *producer)
            .ok_or_else(|| {
                Diagnostic::new(
                    callee.span,
                    format!("unknown destination constructor `{producer}`"),
                )
            })?;
        let family = definition.error.clone().ok_or_else(|| {
            Diagnostic::new(
                call.span,
                "core.construct requires a typed destination constructor",
            )
        })?;
        let record = definition.record.clone();
        arity(args, definition.params.len(), call.span)?;
        let parameters = definition.params.clone();

        // Keep all runtime actuals inside the operation so the backend can
        // resolve the destination first. Restore normal block evaluation before
        // lowering arm statements, including failure handlers that may retry.
        let saved = std::mem::replace(&mut self.deferred_actual, true);
        let mut prelude = Block::default();
        let args = self.producer_arguments(args, &parameters, &mut prelude);
        self.deferred_actual = saved;
        let args = args?;
        assert!(
            prelude.ops.is_empty(),
            "scoped constructor actuals must remain expressions"
        );

        let mut lowered = Vec::with_capacity(arms.len());
        let mut children = Vec::with_capacity(arms.len());
        // The checker visits arm bodies in written order, independent of their
        // runtime dispatch labels. Keep each arm's lexical names private.
        for arm in arms {
            self.charge(arm.span)?;
            let (lowered_arm, spans) = self.construct_arm(arm, &record, &family)?;
            lowered.push(lowered_arm);
            children.push(spans);
        }
        output.push(
            ir::Op::ConstructHandled {
                region,
                producer: producer.clone(),
                args,
                arms: lowered,
            },
            span,
            children,
        );
        Ok(())
    }

    /// Lower one independent outcome block and return its checker span map.
    /// The schema and error family come from the selected typed constructor.
    /// The block scope is restored on success and when lowering diagnoses an error.
    fn construct_arm(
        &mut self,
        arm: &ast::ConstructArm,
        record: &str,
        family: &str,
    ) -> Result<(ir::ConstructArm, Vec<Span>), Diagnostic> {
        self.scopes.push(Default::default());
        let item = (|| {
            let mut reference = None;
            // Only success gains the destination reference. The failure side
            // either exposes one declared scalar payload or introduces no name.
            let pattern = match &arm.pattern {
                ast::ResultPattern::Ok(name) => {
                    if let Some(name) = name {
                        reference = self.result_binding(
                            name,
                            Kind::Reference {
                                record: record.to_owned(),
                                access: ir::Access::Readonly,
                                mutable: false,
                            },
                            arm.span,
                        )?;
                    }
                    ir::AppendPattern::Ok
                }
                ast::ResultPattern::Error {
                    case,
                    binding: None,
                } if matches!(&case.kind, ast::ExprKind::Name(name) if name == "Full") => {
                    ir::AppendPattern::Full
                }
                ast::ResultPattern::Error {
                    case,
                    binding: None,
                } if matches!(&case.kind, ast::ExprKind::Name(name) if name == "SizeOverflow") => {
                    ir::AppendPattern::SizeOverflow
                }
                ast::ResultPattern::Error { .. } => {
                    return Err(Diagnostic::new(
                        arm.span,
                        "use `on Rejected Case` for a constructor error, or a bare `on Full` or `on SizeOverflow` for storage errors",
                    ));
                }
                ast::ResultPattern::Rejected { case, binding } => {
                    let (family, case, has_payload) = self.error_case(case, family, false)?;
                    let binding = self.payload_binding(
                        &family,
                        &case,
                        has_payload,
                        binding.as_deref(),
                        arm.span,
                    )?;
                    ir::AppendPattern::Rejected {
                        family,
                        case,
                        binding,
                    }
                }
                ast::ResultPattern::Fallback(binding) => {
                    // The fallback covers both storage and domain errors and
                    // cannot carry the constructor's narrower nominal family.
                    if binding.as_deref().is_some_and(|name| name != "_") {
                        return Err(Diagnostic::new(
                            arm.span,
                            "core.construct `on error` does not bind a value",
                        ));
                    }
                    ir::AppendPattern::Fallback(None)
                }
            };
            let body = self.block(&arm.body)?;
            Ok((
                ir::ConstructArm {
                    pattern,
                    reference,
                    body: body.ops,
                },
                body.spans,
            ))
        })();
        self.scopes.pop();
        item
    }
}
