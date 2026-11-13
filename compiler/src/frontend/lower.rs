//! Lexical statement adapter for entry and ordinary borrowed functions.
//!
//! Lexical source bindings receive distinct IR names. Compiler-only temporaries
//! capture call arguments once in written order. Region/batch declarations own
//! the remainder of their block, matching marker cleanup without runtime drops.

#[path = "lower/construction.rs"]
mod construction;
#[path = "lower/declarations.rs"]
mod declarations;
#[path = "lower/expressions.rs"]
mod expressions;
#[path = "lower/hosted.rs"]
mod hosted;
#[path = "lower/ordinary.rs"]
mod ordinary;
#[path = "lower/results.rs"]
mod results;
#[path = "lower/statements.rs"]
mod statements;

use super::{FunctionSpans, SourceProgram, ast, functions};
use crate::{
    ir,
    source::{Diagnostic, Span},
};
use std::collections::HashMap;

/// Charged expression/template expansion allowance, not all IR/capture allocations.
const NODE_BUDGET: usize = 100_000;

/// Source binding facts; these are not runtime descriptors or ownership counters.
#[derive(Clone)]
enum Kind {
    /// Copied scalar with source-level rebinding permission.
    Scalar {
        /// Whether assignment is permitted.
        mutable: bool,
    },
    /// Immutable boolean decision retained in the handler's zero-or-one lane.
    HandledBool,
    /// Boolean with a distinct checked local type.
    Bool {
        /// Whether assignment is permitted.
        mutable: bool,
    },
    /// An enum value copied by its nominal type.
    Enum {
        /// Declared enum identity.
        kind: String,
        /// Whether assignment is permitted.
        mutable: bool,
    },
    /// Arm-local failure value. The scalar slice cannot coerce it to an integer.
    Error,
    /// Immutable copied tag and payload retaining its nominal outcome identity.
    SavedResult(ir::FunctionResult),
    /// Call-local byte view carrying the captured formal extent and permission.
    InputBytes {
        /// Index in the complete ordinary parameter list.
        param: usize,
        /// Permission checked when the caller lends this view.
        access: ir::Access,
    },
    /// Lexical readonly fresh extent from a checked hosted refill.
    ReceivedBytes,
    /// Local view captured once from an initialized byte source.
    BytesView(ir::Access),
    /// Opaque lexical stream owner. Only the checked hosted I/O operations can use it.
    TcpStream,
    /// Finite backing owner; no implicit copies or reassignments are supported.
    Region,
    /// Tail owner of a statically known schema.
    Batch {
        /// Logical schema name.
        record: String,
    },
    /// Call-local access to the published prefix of a batch.
    BatchView {
        /// Nominal schema of the published elements.
        record: String,
        /// Maximum permission for reborrowing elements or forwarding the view.
        access: ir::Access,
    },
    /// Rooted reference with independent binding mutability and path permission.
    Reference {
        /// Logical schema of the target.
        record: String,
        /// Capability through this path, never uniqueness.
        access: ir::Access,
        /// Whether this binding may be rebound.
        mutable: bool,
    },
}

/// One resolved source binding and its collision-free IR identity.
#[derive(Clone)]
struct Binding {
    /// Distinct generated identity, including under lexical shadowing.
    ir_name: String,
    /// Scalar/owner/reference facts needed for source subset diagnostics.
    kind: Kind,
}

/// Ordered operations and their flattened checker-preorder source map.
#[derive(Default)]
struct Block {
    /// Actual operations passed unchanged to the existing checker.
    ops: Vec<ir::Op>,
    /// One span per operation, including structured descendants.
    spans: Vec<Span>,
}

/// Source identities for the only admitted automatic backing and its site.
#[derive(Clone, Copy)]
struct SourcePlacement {
    /// Whole span of a `with core.region` statement directly in main.
    owner: Span,
    /// Whole span of the selected one-operand constructor, if one was found.
    site: Option<Span>,
}

impl Block {
    /// Append one operation followed by child maps in checker traversal order.
    /// Child IR is already owned by op; only compiler span vectors move here.
    fn push(&mut self, op: ir::Op, span: Span, children: impl IntoIterator<Item = Vec<Span>>) {
        self.ops.push(op);
        self.spans.push(span);
        // Each child map was built in preorder; concatenation preserves that order.
        for spans in children {
            self.spans.extend(spans);
        }
    }

    /// Move another consecutive block into this one without cloning IR trees.
    fn extend(&mut self, other: Self) {
        self.ops.extend(other.ops);
        self.spans.extend(other.spans);
    }
}

/// Compiler state for lowering a module's structured execution bodies.
struct Lowerer {
    /// Validated signatures, constructors, and scalar templates.
    definitions: functions::Definitions,
    /// Innermost-last source scopes, independent of IR tail-owner scopes.
    scopes: Vec<HashMap<String, Binding>>,
    /// Monotonic identity for source bindings and argument captures.
    next_name: usize,
    /// Remaining expansion allowance across the complete entry program.
    budget: usize,
    /// Structured/tail recursion depth, bounded before recursive lowering.
    depth: usize,
    /// Whether returns and harness-operation diagnostics belong to a helper body.
    ordinary_body: bool,
    /// Whether the current ordinary body returns a boolean.
    returns_bool: bool,
    /// Checked result type of the current ordinary body, if tagged.
    result: Option<ir::FunctionResult>,
    /// Typed actual evaluation is contained in its expression until the call runs.
    deferred_actual: bool,
    /// Lexically selected owner and site, independent of explicit IR owner tails.
    placement: Option<SourcePlacement>,
}

/// Lower ordinary declarations and entry statements using validated signatures.
/// Returns unchecked IR plus precise spans, or a source-subset diagnostic.
pub(super) fn lower(
    module: &ast::Module,
    definitions: functions::Definitions,
) -> Result<SourceProgram, Diagnostic> {
    let fallback = Span { start: 0, end: 0 };
    let entries: Vec<_> = module
        .functions
        .iter()
        .filter(|function| function.name == "main")
        .collect();
    // Entry identity/header are explicit; no executable body or exit policy is guessed.
    if entries.len() != 1 {
        return Err(Diagnostic::new(
            fallback,
            "expected exactly one `func main`",
        ));
    }
    let main = entries[0];
    if !main.args.is_empty() || main.result != "u64" {
        return Err(Diagnostic::new(
            main.span,
            "experimental main takes no arguments and declares `returns u64`",
        ));
    }
    if !main.guarantees.is_empty() {
        return Err(Diagnostic::new(
            main.guarantees[0].1,
            "entry-function contracts are unsupported until process I/O and cleanup effects are modeled",
        ));
    }
    if !terminates(&main.body) {
        return Err(Diagnostic::new(
            main.span,
            "experimental main cannot fall through; end every path with `return 0` or an unconditional entry loop",
        ));
    }
    lower_bodies(module, definitions, Some(main))
}

/// Lower namespace declarations for library checking without entry operations.
/// Namespace resolution has rejected entry functions. Ordinary definitions
/// retain their source map and the same checking path used by applications.
pub(super) fn lower_library(
    module: &ast::Module,
    definitions: functions::Definitions,
) -> Result<SourceProgram, Diagnostic> {
    lower_bodies(module, definitions, None)
}

/// Lower the same ordinary definitions for either source shape. Only a real
/// application entry contributes executable entry statements or their spans.
fn lower_bodies(
    module: &ast::Module,
    definitions: functions::Definitions,
    main: Option<&ast::Function>,
) -> Result<SourceProgram, Diagnostic> {
    let mut lowerer = Lowerer {
        definitions,
        scopes: vec![HashMap::new()],
        next_name: 0,
        budget: NODE_BUDGET,
        depth: 0,
        ordinary_body: false,
        returns_bool: false,
        result: None,
        deferred_actual: false,
        placement: main
            .map(construction::source_placement)
            .transpose()?
            .flatten(),
    };
    let mut body = Block::default();
    for declaration in &module.enums {
        body.push(
            ir::Op::Enum(ir::EnumDef {
                name: declaration.name.clone(),
                variants: declaration
                    .variants
                    .iter()
                    .map(|(variant, _)| variant.clone())
                    .collect(),
            }),
            declaration.span,
            [],
        );
    }
    let mut function_spans = HashMap::new();
    // Keep case ordinals in source order. Declarations are compiler metadata only.
    for declaration in &module.errors {
        body.push(
            ir::Op::ErrorFamily(lowerer.definitions.errors[&declaration.name].clone()),
            declaration.span,
            [],
        );
    }
    if let Some(option) = lowerer.definitions.errors.get(functions::OPTION_FAMILY) {
        let declaration = module
            .functions
            .iter()
            .find(|function| function.result == functions::SCALAR_OPTION)
            .expect("optional result metadata requires an optional function");
        body.push(ir::Op::ErrorFamily(option.clone()), declaration.span, []);
    }
    // Each ordinary declaration is checked even when entry never calls it.
    // The definition counts as one entry operation for the source map.
    for function in &module.functions {
        if lowerer.definitions.ordinary.contains_key(&function.name) {
            let (function_ir, spans) = lowerer.ordinary_definition(function)?;
            function_spans.insert(
                function.name.clone(),
                FunctionSpans {
                    declaration: function.span,
                    body: spans,
                },
            );
            body.push(ir::Op::Function(function_ir), function.span, []);
        }
    }
    // Library checking has definitions only. Application bodies keep their
    // existing position after declarations and their ordinary lexical cleanup.
    if let Some(main) = main {
        body.extend(lowerer.block(&main.body)?);
    }
    let fallback = main
        .or_else(|| module.functions.first())
        .map_or(Span { start: 0, end: 0 }, |function| function.span);
    // The span map is final only after all prelude and declaration operations
    // are in place. The two complete statement spans identify the emitted ops.
    let placement = lowerer.placement.map(|placement| {
        let index = |target| {
            body.spans
                .iter()
                .position(|span| *span == target)
                .expect("placement statement emitted one mapped operation")
        };
        (
            index(placement.owner),
            index(placement.site.expect("owner lowering required a site")),
        )
    });
    Ok(SourceProgram {
        program: ir::Program {
            records: lowerer.definitions.records,
            producers: lowerer.definitions.producers,
            body: body.ops,
        },
        operation_spans: body.spans,
        function_spans,
        fallback,
        naming_diagnostics: Vec::new(),
        placement,
    })
}

/// Check whether the function has any path that falls through its final statement.
fn terminates(body: &[ast::Statement]) -> bool {
    // An invalid transfer outside any loop has no continuation. Lowering retains
    // it so the verifier can report the exact statement that needs a loop.
    block_terminates(body, false, true)
}

/// Follow normal and loop-exit continuations to check a lexical block's paths.
/// A reverse fold avoids recursion for a flat sequence of owner declarations.
fn block_terminates(body: &[ast::Statement], at_end: bool, on_break: bool) -> bool {
    let mut following = at_end;
    for statement in body.iter().rev() {
        following = match &statement.kind {
            ast::StatementKind::Let {
                value,
                otherwise: Some(recovery),
                ..
            } if core_call(value).is_some_and(|(operation, _)| operation == "region") => {
                // A failed owner bypasses its successful tail in this block.
                following && block_terminates(recovery, at_end, on_break)
            }
            ast::StatementKind::Return(_) | ast::StatementKind::Continue => true,
            ast::StatementKind::Break => on_break,
            // Reaching the loop body end starts a new iteration. Only a break
            // targeting this loop can reach the following statements.
            ast::StatementKind::Loop(body) => block_terminates(body, true, following),
            ast::StatementKind::Scope(body) | ast::StatementKind::TcpScope { body, .. } => {
                block_terminates(body, following, on_break)
            }
            ast::StatementKind::If {
                then_body,
                else_body,
                ..
            } => {
                block_terminates(then_body, following, on_break)
                    && block_terminates(else_body, following, on_break)
            }
            ast::StatementKind::Refill {
                data_body,
                other_body,
                ..
            } => {
                block_terminates(data_body, following, on_break)
                    && block_terminates(other_body, following, on_break)
            }
            ast::StatementKind::WithRegion {
                body, on_failure, ..
            } => {
                block_terminates(body, following, on_break)
                    && block_terminates(on_failure, following, on_break)
            }
            ast::StatementKind::Construct { arms, .. }
            | ast::StatementKind::HandleResult { arms, .. } => arms
                .iter()
                .all(|arm| block_terminates(&arm.body, following, on_break)),
            // Counted loops can run zero times. Nonowner declarations and ordinary
            // operations do not supply an entry exit. Missing recovery gets its
            // specific declaration diagnostic during lowering, never acceptance.
            _ => following,
        };
    }
    following
}

impl Lowerer {
    /// Allocate a compiler identity that cannot collide with lexable source names.
    fn fresh(&mut self, hint: &str) -> String {
        let name = format!("{hint}.${}", self.next_name);
        self.next_name += 1;
        name
    }

    /// Charge one admitted expression before building more compiler-owned nodes.
    fn charge(&mut self, span: Span) -> Result<(), Diagnostic> {
        // Expansion exhaustion is a diagnostic, never a semantic fallback or OOM experiment.
        if self.budget == 0 {
            return Err(Diagnostic::new(
                span,
                "experimental source expansion limit exceeded",
            ));
        }
        self.budget -= 1;
        Ok(())
    }

    /// Resolve a name using lexical source scope order, copying only small facts.
    fn lookup(&self, name: &str, span: Span) -> Result<Binding, Diagnostic> {
        // Nearest binding wins, while distinct IR names preserve all outer roots.
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.get(name) {
                return Ok(binding.clone());
            }
        }
        Err(Diagnostic::new(
            span,
            format!("unknown or out-of-scope binding `{name}`"),
        ))
    }

    /// Introduce one source name after its initializer, rejecting same-block duplicates.
    fn bind(&mut self, name: &str, kind: Kind, span: Span) -> Result<String, Diagnostic> {
        let scope = self.scopes.last().expect("entry source scope exists");
        // core is the explicit adapter namespace, so local shadowing is unavailable here.
        if name == "core" || scope.contains_key(name) {
            return Err(Diagnostic::new(
                span,
                format!("duplicate or reserved binding `{name}`"),
            ));
        }
        let ir_name = self.fresh(name);
        self.scopes
            .last_mut()
            .expect("entry source scope exists")
            .insert(
                name.to_owned(),
                Binding {
                    ir_name: ir_name.clone(),
                    kind,
                },
            );
        Ok(ir_name)
    }

    /// Lower a fresh lexical source block, discarding names on success or failure.
    fn scoped(&mut self, body: &[ast::Statement]) -> Result<Block, Diagnostic> {
        self.scopes.push(HashMap::new());
        let result = self.block(body);
        self.scopes.pop();
        result
    }

    /// Lower a lexical branch with a fresh input binding, returning its name and block.
    /// Removes the binding after success or rejection so sibling branches cannot see it.
    fn scoped_input(
        &mut self,
        name: &str,
        kind: Kind,
        body: &[ast::Statement],
        span: Span,
    ) -> Result<(String, Block), Diagnostic> {
        self.scopes.push(HashMap::new());
        let result = self
            .bind(name, kind, span)
            .and_then(|name| self.block(body).map(|body| (name, body)));
        self.scopes.pop();
        result
    }

    /// Lower a block with a fixed recursion bound, including implicit owner-tail scopes.
    fn block(&mut self, body: &[ast::Statement]) -> Result<Block, Diagnostic> {
        // Flat chains of owner declarations also nest IR cleanup; cap those before recursing.
        if self.depth >= 64 {
            return Err(Diagnostic::new(
                body.first().map_or(Span { start: 0, end: 0 }, |s| s.span),
                "experimental lowering nesting limit of 64 exceeded",
            ));
        }
        self.depth += 1;
        let result = self.statements(body);
        self.depth -= 1;
        result
    }

    /// Capture an already lowered scalar exactly once before later argument work.
    fn capture(&mut self, value: ir::Expr, span: Span, output: &mut Block) -> ir::Expr {
        let name = self.fresh("argument");
        output.push(
            ir::Op::Let {
                name: name.clone(),
                value,
            },
            span,
            [],
        );
        ir::Expr::Local(name)
    }

    /// Resolve a bare named operand instead of interpreting arbitrary owner expressions.
    fn named(&self, expr: &ast::Expr) -> Result<Binding, Diagnostic> {
        // Owners/references must be explicit names in this subset; no hidden copies/calls.
        match &expr.kind {
            ast::ExprKind::Name(name) => self.lookup(name, expr.span),
            _ => Err(Diagnostic::new(expr.span, "expected a live binding name")),
        }
    }

    /// Resolve a named owner or view with the requested element access.
    fn batch(&self, expr: &ast::Expr, access: ir::Access) -> Result<(String, String), Diagnostic> {
        let binding = self.named(expr)?;
        let record = match binding.kind {
            Kind::Batch { record } => record,
            Kind::BatchView {
                record,
                access: available,
            } if access == ir::Access::Readonly || access == available => record,
            Kind::BatchView { .. } => {
                return Err(Diagnostic::new(
                    expr.span,
                    "cannot get readwrite access through a readonly batch view",
                ));
            }
            _ => return Err(Diagnostic::new(expr.span, "expected a batch binding")),
        };
        Ok((binding.ir_name, record))
    }

    /// Require an owning batch for operations that may change publication or storage.
    fn owned_batch(&self, expr: &ast::Expr) -> Result<(String, String), Diagnostic> {
        let binding = self.named(expr)?;
        match binding.kind {
            Kind::Batch { record } => Ok((binding.ir_name, record)),
            Kind::BatchView { .. } => Err(Diagnostic::new(
                expr.span,
                "cannot change publication through a borrowed batch view",
            )),
            _ => Err(Diagnostic::new(expr.span, "expected a batch owner binding")),
        }
    }

    /// Require a bare backing owner; this does not acquire or duplicate storage.
    fn region(&self, expr: &ast::Expr) -> Result<String, Diagnostic> {
        let binding = self.named(expr)?;
        if !matches!(binding.kind, Kind::Region) {
            return Err(Diagnostic::new(expr.span, "expected a region binding"));
        }
        Ok(binding.ir_name)
    }
}

/// Recognize the explicit experimental namespace without treating it as a receiver.
fn core_call(expr: &ast::Expr) -> Option<(&str, &[ast::Expr])> {
    // Only exactly core.member(args) names a builtin; nested/member-value calls are rejected.
    if let ast::ExprKind::Call { callee, args } = &expr.kind
        && let ast::ExprKind::Member { base, field } = &callee.kind
        && matches!(&base.kind, ast::ExprKind::Name(name) if name == "core")
    {
        return Some((field, args));
    }
    None
}

/// Validate a builtin/callee arity before indexing argument slices.
fn arity(args: &[ast::Expr], expected: usize, span: Span) -> Result<(), Diagnostic> {
    if args.len() != expected {
        return Err(Diagnostic::new(
            span,
            format!("expected {expected} arguments, received {}", args.len()),
        ));
    }
    Ok(())
}
