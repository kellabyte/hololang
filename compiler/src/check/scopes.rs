//! Lexical root, access, and structured-operation validation.
//!
//! A batch or scoped record owns one live region tail. References retain the
//! owning root through stable append or until the record arm exits. Reset is
//! rejected while any lexical alias remains. Changed-root
//! reference rebinding is deliberately unsupported, so branch/loop analysis cannot
//! silently forget a possible origin. These are compiler facts, not runtime scans.

use super::definitions::Definitions;
use super::expressions::{self, Context};
use super::{CheckError, HostedIo, PrivatePlacement};
use crate::ir::{
    Access, AppendPattern, Argument, BytesSource, ConstructArm, FieldKind, Function,
    FunctionParameterKind, FunctionResult, Op, Producer, Refill, ResultBlockArm, ResultPattern,
    ResultSuccess, ResultValue,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Root and permission information retained for one live reference binding.
#[derive(Clone, Copy)]
pub(super) struct Reference<'p> {
    /// Unique identity of the owner whose initialized contents are borrowed.
    root: usize,
    /// Lexical depth at which the owner was introduced.
    root_depth: usize,
    /// Schema establishing valid field projections.
    pub(super) record: &'p str,
    /// Capability through this path, without any uniqueness implication.
    access: Access,
}

/// A checked byte field's owning root and complete projection path.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ByteOrigin<'p> {
    /// Owning batch or record identity, shared through aliases and same-root rebinds.
    root: usize,
    /// Distinct checked fields occupy disjoint storage.
    path: &'p [String],
}

/// Kind and ownership facts for an initialized lexical name.
#[derive(Clone, Copy)]
pub(super) enum Binding<'p> {
    /// Unsigned local, with loop indices protected against mutation.
    Scalar {
        /// Whether `Set` may update this scalar; loop indices are readonly.
        writable: bool,
    },
    /// Boolean, distinct from unsigned scalar locals.
    Boolean {
        /// Whether `SetBool` may update this local.
        writable: bool,
    },
    /// Enum local with a fixed nominal type.
    Enum {
        /// Checked enum declaration identity.
        kind: &'p str,
        /// Whether assignment may replace this value.
        writable: bool,
    },
    /// Immutable result family, independent of the initializer's borrows.
    SavedResult(&'p FunctionResult),
    /// An arm-local error with its checked nominal family, never a scalar tag.
    Error {
        /// Family of the subject's failure result.
        family: &'p str,
    },
    /// Backing owner identified independently of shadowed source names.
    Region {
        /// Unique compiler identity, independent of lexical shadowing.
        root: usize,
    },
    /// Single tail owner, whose references share `root` and schema.
    Batch {
        /// Identity shared by all references into this batch.
        root: usize,
        /// Backing owner whose single tail this batch occupies.
        region: usize,
        /// Scope depth used to reject storing a reference into an older binding.
        depth: usize,
        /// Logical schema used to validate each field projection.
        record: &'p str,
    },
    /// Call-local authority over an existing published prefix, without tail ownership.
    BatchView(Reference<'p>),
    /// Checked reference into one live owner root.
    Reference(Reference<'p>),
    /// Non-rebindable success path to a record constructed at its final address.
    ScopedReference(Reference<'p>),
    /// Immutable Bytes formal whose observations address the typed formal index.
    Bytes,
    /// Immutable local view with authority over the captured bytes.
    BytesView {
        /// Original field, absent for a view of a caller's byte formal.
        origin: Option<ByteOrigin<'p>>,
        /// Permission retained through this view.
        access: Access,
    },
    /// The sole opaque TCP stream capability. It is never a scalar or reference.
    TcpStream,
    /// Readonly fresh prefix of one checked refill with a lexical Data scope.
    Received(ByteOrigin<'p>),
}

/// Name environments for nested lexical regions, batches, branches, and loops.
pub(super) struct Environment<'p> {
    /// Innermost-last lexical tables; names cannot be redeclared within one table.
    scopes: Vec<HashMap<&'p str, Binding<'p>>>,
    /// Immutable expression-local bindings visited by the iterative arm walker.
    arms: RefCell<Vec<(&'p str, Binding<'p>)>>,
    /// Region roots lent to active success arms, including arms without a name.
    constructed_tails: Vec<usize>,
    /// Next distinct compiler root identity; never emitted into runtime data.
    next_root: usize,
    /// Next entry-operation preorder index, including all structured child bodies.
    next_operation: usize,
    /// Number of enclosing loops available to a local break or continue.
    loop_depth: usize,
    /// Ordinary helper whose closed local frame is being checked, if any.
    function: Option<&'p Function>,
    /// Entry effects that require the host and its optional output support.
    hosted: HostedIo,
    /// True when the checked root is exactly one TCP scope and static declarations.
    allows_tcp_scope: bool,
    /// The lexical TCP owner stays live until a direct consuming close.
    tcp_live: bool,
    /// Statically resolved callees seen in this helper's checked expressions.
    /// Interior mutation records graph facts without changing lexical bindings.
    calls: RefCell<Vec<&'p str>>,
    /// Optional compiler-owned binding checked only during placement admission.
    private_placement: Option<&'p PrivatePlacement>,
}

impl<'p> Environment<'p> {
    /// Start entry checking with one empty lexical scope and no live roots.
    fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            arms: RefCell::new(Vec::new()),
            constructed_tails: Vec::new(),
            next_root: 0,
            next_operation: 0,
            loop_depth: 0,
            function: None,
            hosted: HostedIo::default(),
            allows_tcp_scope: false,
            tcp_live: false,
            calls: RefCell::new(Vec::new()),
            private_placement: None,
        }
    }

    /// Find the nearest initialized binding named `name`, or reject an escape/use.
    pub(super) fn lookup(&self, name: &str) -> Result<Binding<'p>, CheckError> {
        self.reject_private_name(name)?;
        self.lookup_bound(name)
    }

    /// Resolve a binding by lexical position without the placement name filter.
    /// Only the exact generated site may use this for the private region name.
    fn lookup_bound(&self, name: &str) -> Result<Binding<'p>, CheckError> {
        // Arm locals shadow ordinary bindings only while checking that arm.
        for &(local, binding) in self.arms.borrow().iter().rev() {
            if local == name {
                return Ok(binding);
            }
        }
        // Reverse search implements lexical shadowing without merging owner identities.
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.get(name) {
                return Ok(*binding);
            }
        }
        Err(CheckError::new(format!(
            "unknown or out-of-scope binding {name}"
        )))
    }

    /// Record the current expression depth so a failed walk can restore its bindings.
    pub(super) fn arm_depth(&self) -> usize {
        self.arms.borrow().len()
    }

    /// Add an immutable expression-local binding after checking its name.
    /// A rejected reserved name leaves the current arm stack untouched.
    pub(super) fn push_arm(&self, name: &'p str, binding: Binding<'p>) -> Result<(), CheckError> {
        self.reject_private_name(name)?;
        self.arms.borrow_mut().push((name, binding));
        Ok(())
    }

    /// Leave the innermost expression-local binding after its arm was checked.
    pub(super) fn pop_arm(&self) {
        self.arms.borrow_mut().pop().expect("active result arm");
    }

    /// Restore expression-local scopes after validation, including diagnostic exits.
    pub(super) fn restore_arms(&self, depth: usize) {
        self.arms.borrow_mut().truncate(depth);
    }

    /// Require an initialized unsigned scalar name without allocating target data.
    pub(super) fn require_scalar(&self, name: &str) -> Result<(), CheckError> {
        // Storage owners and references cannot become scalar addresses implicitly.
        match self.lookup(name)? {
            Binding::Scalar { .. } => Ok(()),
            Binding::Error { family } => Err(CheckError::new(format!(
                "{name} is an error of family {family}, not a scalar local"
            ))),
            _ => Err(CheckError::new(format!("{name} is not a scalar local"))),
        }
    }

    /// Verify that the name still exposes this entry's sole live TCP owner.
    /// A local shadow cannot recover authority and a consumed owner stays dead.
    fn require_tcp(&self, name: &str) -> Result<(), CheckError> {
        // Check nominal binding kind before reporting consumption of the owner.
        if !matches!(self.lookup(name)?, Binding::TcpStream) {
            return Err(CheckError::new(format!("{name} is not a TCP stream owner")));
        }
        if !self.tcp_live {
            return Err(CheckError::new(format!(
                "TCP stream {name} was already closed"
            )));
        }
        Ok(())
    }

    /// Copy immutable checker facts for a live reference, rejecting other bindings.
    pub(super) fn reference(&self, name: &str) -> Result<Reference<'p>, CheckError> {
        // Copying checker facts does not clone target data or upgrade access.
        match self.lookup(name)? {
            Binding::Reference(reference) | Binding::ScopedReference(reference) => Ok(reference),
            _ => Err(CheckError::new(format!("{name} is not a reference"))),
        }
    }

    /// Resolve a batch that owns its region tail for append and reset.
    fn owned_batch(&self, name: &str) -> Result<Binding<'p>, CheckError> {
        match self.lookup(name)? {
            binding @ Binding::Batch { .. } => Ok(binding),
            Binding::BatchView(_) => Err(CheckError::new(format!(
                "borrowed batch {name} does not own its storage tail"
            ))),
            _ => Err(CheckError::new(format!("{name} is not a batch"))),
        }
    }

    /// Obtain the published prefix's root and narrow its element access for this use.
    pub(super) fn batch_view(
        &self,
        name: &str,
        access: Access,
    ) -> Result<Reference<'p>, CheckError> {
        let mut view = match self.lookup(name)? {
            Binding::Batch {
                root,
                depth,
                record,
                ..
            } => Reference {
                root,
                root_depth: depth,
                record,
                access: Access::Readwrite,
            },
            Binding::BatchView(view) => view,
            _ => return Err(CheckError::new(format!("{name} is not a batch"))),
        };
        if view.access == Access::Readonly && access == Access::Readwrite {
            return Err(CheckError::new(format!("batch {name} is readonly")));
        }
        view.access = access;
        Ok(view)
    }

    /// Resolve an initialized region and prove its tail is available.
    ///
    /// Compares owner identities across scopes, including shadowed batch bindings
    /// and success arms that discarded the record. This is compiler bookkeeping.
    fn free_tail(&self, name: &str) -> Result<usize, CheckError> {
        self.require_free_tail(name, self.lookup(name)?)
    }

    /// Verify that an already resolved region root has no live tail claimant.
    /// Neither scope state nor region storage is mutated.
    fn require_free_tail(&self, name: &str, binding: Binding<'p>) -> Result<usize, CheckError> {
        let Binding::Region { root } = binding else {
            return Err(CheckError::new(format!("{name} is not a region")));
        };
        // An unnamed success still owns the tail through the end of its arm.
        if self.constructed_tails.contains(&root) {
            return Err(CheckError::new(format!(
                "region {name} already has a live scoped record owner"
            )));
        }
        // Batch identity is independent of subsequent lexical name shadowing.
        if self.scopes.iter().any(|scope| {
            scope
                .values()
                .any(|binding| matches!(binding, Binding::Batch { region, .. } if *region == root))
        }) {
            return Err(CheckError::new(format!(
                "region {name} already has a live batch owner"
            )));
        }
        Ok(root)
    }

    /// Declare a new initialized name at the current depth, rejecting duplicates.
    fn insert(&mut self, name: &'p str, binding: Binding<'p>) -> Result<(), CheckError> {
        self.reject_private_name(name)?;
        self.insert_bound(name, binding)
    }

    /// Insert a binding after the caller has verified its naming authority.
    /// The current scope gains the fact only if it was previously absent.
    fn insert_bound(&mut self, name: &'p str, binding: Binding<'p>) -> Result<(), CheckError> {
        let current = self.scopes.last_mut().expect("entry scope always exists");
        // Reject same-scope replacement so names cannot silently discard root facts.
        if name.is_empty() || current.contains_key(name) {
            return Err(CheckError::new(format!(
                "duplicate or empty local name {name}"
            )));
        }
        current.insert(name, binding);
        Ok(())
    }

    /// Keep user-supplied entry code from naming or shadowing the generated
    /// backing. This check allocates only when constructing a rejection.
    fn reject_private_name(&self, name: &str) -> Result<(), CheckError> {
        // Ordinary IR has no reserved placement binding. Admission of this
        // prototype reserves exactly one name for the generated owner/site.
        if self
            .private_placement
            .is_some_and(|placement| placement.name == name)
        {
            return Err(CheckError::new(
                "cannot use or shadow the private scoped placement region",
            ));
        }
        Ok(())
    }

    /// Reserve a fresh checker root, independent of user names and runtime pointers.
    fn root(&mut self) -> usize {
        let root = self.next_root;
        self.next_root += 1;
        root
    }

    /// Require a writable path while permitting compatible aliases of its root.
    pub(super) fn writable_reference(&self, name: &str) -> Result<Reference<'p>, CheckError> {
        let reference = self.reference(name)?;
        // Another alias may be writable, but authority never transfers implicitly here.
        if reference.access != Access::Readwrite {
            return Err(CheckError::new(format!("reference {name} is readonly")));
        }
        Ok(reference)
    }

    /// Identify a body subject to ordinary-call effect and return restrictions.
    pub(super) fn in_function(&self) -> bool {
        self.function.is_some()
    }

    /// Check a forwarded parameter's typed position without exposing a pointer value.
    pub(super) fn byte_param(&self, index: usize) -> bool {
        self.byte_param_kind(index).is_some_and(|kind| {
            matches!(
                kind,
                FunctionParameterKind::Bytes | FunctionParameterKind::WritableBytes
            )
        })
    }

    /// Require write authority on the forwarded descriptor itself.
    pub(super) fn writable_byte_param(&self, index: usize) -> bool {
        self.byte_param_kind(index) == Some(&FunctionParameterKind::WritableBytes)
    }

    /// Resolve a typed formal position within the current ordinary frame.
    fn byte_param_kind(&self, index: usize) -> Option<&FunctionParameterKind> {
        self.function
            .and_then(|function| function.params.get(index))
            .map(|parameter| &parameter.kind)
    }

    /// Require that `name` is an in-scope fresh byte prefix for a synchronous borrow.
    /// A scalar or ordinary reference cannot stand in for an adapter result.
    pub(super) fn received(&self, name: &str) -> Result<(), CheckError> {
        // A lexical shadow can hide a received prefix from use but cannot widen it.
        if !matches!(self.lookup(name)?, Binding::Received(_)) {
            return Err(CheckError::new(format!("{name} is not a received view")));
        }
        Ok(())
    }

    /// Require a compatible named view and retain its original field identity.
    pub(super) fn byte_view(
        &self,
        name: &str,
        requested: Access,
    ) -> Result<Option<ByteOrigin<'p>>, CheckError> {
        let Binding::BytesView { origin, access } = self.lookup(name)? else {
            return Err(CheckError::new(format!("{name} is not a local byte view")));
        };
        if requested == Access::Readwrite && access == Access::Readonly {
            return Err(CheckError::new(format!(
                "local byte view {name} is readonly"
            )));
        }
        Ok(origin)
    }

    /// Check a byte borrow's authority before narrowing or lending its extent.
    pub(super) fn byte_view_source(
        &self,
        source: &'p BytesSource,
        access: Access,
        definitions: &Definitions<'_>,
    ) -> Result<Option<ByteOrigin<'p>>, CheckError> {
        match source {
            BytesSource::Literal(_) => {
                if access == Access::Readwrite {
                    return Err(CheckError::new("byte literal is readonly"));
                }
                Ok(None)
            }
            BytesSource::Param(index) if self.byte_param(*index) => {
                if access == Access::Readwrite && !self.writable_byte_param(*index) {
                    return Err(CheckError::new("Bytes parameter is readonly"));
                }
                Ok(None)
            }
            BytesSource::Field { reference, path } => {
                let reference = match access {
                    Access::Readonly => self.reference(reference)?,
                    Access::Readwrite => self.writable_reference(reference)?,
                };
                expressions::require_field(definitions, reference.record, path, FieldKind::Bytes)?;
                Ok(Some(ByteOrigin {
                    root: reference.root,
                    path,
                }))
            }
            BytesSource::Received(name) => {
                let Binding::Received(origin) = self.lookup(name)? else {
                    return Err(CheckError::new(format!("{name} is not a received view")));
                };
                if access == Access::Readwrite {
                    return Err(CheckError::new("received view is readonly"));
                }
                Ok(Some(origin))
            }
            BytesSource::Local(name) => self.byte_view(name, access),
            BytesSource::Param(_) => Err(CheckError::new(
                "byte borrow requires a live Bytes parameter",
            )),
        }
    }

    /// Record a validated graph edge when this frame belongs to a helper.
    /// Entry calls cannot participate in a cycle because entry is never a callee.
    pub(super) fn note_call(&self, name: &'p str) {
        // Keeping entry graph storage empty preserves its allocation profile.
        if self.in_function() {
            self.calls.borrow_mut().push(name);
        }
    }
}

/// Validate `body` against known `definitions` from a fresh lexical environment.
///
/// Returns the first rejection or success without changing input IR. Temporary
/// scope maps allocate host memory; no environment is retained by generated code.
pub(super) fn validate<'p>(
    body: &'p [Op],
    definitions: &Definitions<'p>,
    private_placement: Option<&'p PrivatePlacement>,
) -> Result<HostedIo, CheckError> {
    let mut environment = Environment::new();
    environment.private_placement = private_placement;
    // One trusted handoff supplies the whole entry. Static declarations never
    // execute and may coexist with that root without changing close authority.
    let mut executable = body
        .iter()
        .filter(|op| !matches!(op, Op::Function(_) | Op::ErrorFamily(_) | Op::Enum(_)));
    environment.allows_tcp_scope =
        matches!(executable.next(), Some(Op::TcpScope { .. })) && executable.next().is_none();
    operations(body, definitions, &mut environment)?;
    Ok(environment.hosted)
}

/// Check one ordinary definition in a new parameter-only frame. Its checked call
/// edges are returned for cycle detection before native lowering is authorized.
pub(super) fn validate_function<'p>(
    function: &'p Function,
    definitions: &Definitions<'p>,
) -> Result<Vec<&'p str>, CheckError> {
    let mut environment = Environment::new();
    environment.function = Some(function);
    // State roots model permission and lifetime. Distinct compiler facts never
    // imply that the actual caller records are disjoint.
    for parameter in &function.params {
        let binding = match &parameter.kind {
            FunctionParameterKind::U64 => Binding::Scalar { writable: false },
            FunctionParameterKind::Bool => Binding::Boolean { writable: false },
            FunctionParameterKind::Bytes | FunctionParameterKind::WritableBytes => Binding::Bytes,
            FunctionParameterKind::State(record) => {
                definitions.require_scalar_record(record)?;
                Binding::Reference(Reference {
                    root: environment.root(),
                    root_depth: 0,
                    record,
                    access: Access::Readwrite,
                })
            }
            FunctionParameterKind::Batch { record, access } => {
                definitions.record(record)?;
                Binding::BatchView(Reference {
                    root: environment.root(),
                    root_depth: 0,
                    record,
                    access: *access,
                })
            }
        };
        environment.insert(&parameter.name, binding)?;
    }
    operations(&function.body, definitions, &mut environment)?;
    // Even a dynamically nonempty loop does not establish a structural return.
    if !returns(&function.body) {
        let result = if function.returns_bool {
            "a boolean"
        } else if function.result.is_some() {
            "a typed result"
        } else {
            "a scalar"
        };
        return Err(CheckError::new(format!(
            "ordinary function must return {result} on every path"
        )));
    }
    Ok(environment.calls.into_inner())
}

/// Establish a return on every path through an already checked body.
/// Scope inherits the fact, a conditional needs both branches, and loops may skip.
fn returns(body: &[Op]) -> bool {
    // One definitely returning instruction suffices for all paths still live there.
    body.iter().any(|operation| match operation {
        Op::Return(_) | Op::ReturnBool(_) | Op::ReturnResult(_) => true,
        Op::Scope(body) => returns(body),
        Op::HandleResult { arms, .. } | Op::HandleSavedResult { arms, .. } => {
            arms.iter().all(|arm| returns(&arm.body))
        }
        Op::If {
            then_body,
            else_body,
            ..
        } => returns(then_body) && returns(else_body),
        _ => false,
    })
}

/// Check `body` against `definitions`, mutating compiler lexical facts in `environment`.
///
/// Returns success or the first error. Current aliases cannot change roots, so
/// checking both branches and a loop body cannot lose a possible outer origin.
fn operations<'p>(
    body: &'p [Op],
    definitions: &Definitions<'p>,
    environment: &mut Environment<'p>,
) -> Result<(), CheckError> {
    // Structured order ensures a use is checked only after its declaration/initialization.
    for operation in body {
        let operation_index = environment.next_operation;
        environment.next_operation += 1;
        // Assign the parent before visiting children. A nested rejection attaches
        // its own index first, and this enclosing annotation cannot overwrite it.
        operation_check(operation, operation_index, definitions, environment, false)
            .map_err(|error| error.at_operation(operation_index))?;
    }
    Ok(())
}

/// Check one operation and its structured children using the shared environment.
///
/// Input definitions remain immutable; lexical facts and the traversal index may
/// advance. Returns the first rejection, with child positions already attached by
/// `operations`. No target storage or runtime reference bookkeeping is created.
fn operation_check<'p>(
    operation: &'p Op,
    operation_index: usize,
    definitions: &Definitions<'p>,
    environment: &mut Environment<'p>,
    named_tcp: bool,
) -> Result<(), CheckError> {
    // The initial ordinary call subset borrows fixed caller storage and has no
    // allocator, external I/O or owner capabilities. Nested calls are checked
    // against the same closed graph, including unused and unreachable definitions.
    if environment.in_function()
        && !matches!(
            operation,
            Op::Scope(_)
                | Op::Let { .. }
                | Op::LetBool { .. }
                | Op::VarBool { .. }
                | Op::LetEnum { .. }
                | Op::BindBytes { .. }
                | Op::BindResult { .. }
                | Op::TryLet { .. }
                | Op::TrySaved { .. }
                | Op::TryUnit { .. }
                | Op::TrySavedUnit { .. }
                | Op::HandleResult { .. }
                | Op::HandleSavedResult { .. }
                | Op::Set { .. }
                | Op::SetBool { .. }
                | Op::SetEnum { .. }
                | Op::Borrow { .. }
                | Op::Alias { .. }
                | Op::AssignRef { .. }
                | Op::Write { .. }
                | Op::WriteBool { .. }
                | Op::WriteEnum { .. }
                | Op::WriteByte { .. }
                | Op::WriteInputByte { .. }
                | Op::WriteViewByte { .. }
                | Op::MoveInputBytes { .. }
                | Op::Repeat { .. }
                | Op::ForEach { .. }
                | Op::Break
                | Op::Continue
                | Op::If { .. }
                | Op::Assert(_)
                | Op::Return(_)
                | Op::ReturnBool(_)
                | Op::ReturnResult(_)
        )
    {
        return Err(CheckError::new(
            "ordinary functions cannot acquire or invalidate storage owners or perform external effects",
        ));
    }
    // A TCP entry owns the descriptor. The implicit host operations would
    // bypass the binding and could continue to use its number after close.
    if environment.hosted.tcp
        && !named_tcp
        && matches!(
            operation,
            Op::Refill(_)
                | Op::Send { .. }
                | Op::WaitInput { .. }
                | Op::WaitOutput { .. }
                | Op::NextSession { .. }
        )
    {
        return Err(CheckError::new(
            "a TCP entry requires an explicit live stream and cannot advance the host session",
        ));
    }
    match operation {
        // Only outermost declarations enter the static table, with no runtime action.
        Op::Function(_) | Op::ErrorFamily(_) | Op::Enum(_) => {
            if environment.scopes.len() != 1 {
                return Err(CheckError::new(
                    "function, error-family, or enum declaration must be at the outermost level",
                ));
            }
        }
        Op::TcpScope { stream, body } => {
            // No nested, repeated, or competing owners can reach the backend.
            if !environment.allows_tcp_scope || environment.scopes.len() != 1 {
                return Err(CheckError::new(
                    "TCP stream ownership must be the only executable scope at the hosted entry",
                ));
            }
            environment.hosted.tcp = true;
            environment.tcp_live = true;
            environment.scopes.push(HashMap::new());
            environment.insert(stream, Binding::TcpStream)?;
            operations(body, definitions, environment)?;
            environment.scopes.pop();
            environment.tcp_live = false;
        }
        Op::TcpIo { stream, operation } => {
            environment.require_tcp(stream)?;
            // Restrict the wrapper before delegating to the existing range
            // checker. No generic operation can borrow or transfer authority.
            if !matches!(
                operation.as_ref(),
                Op::Refill(_) | Op::Send { .. } | Op::WaitInput { .. } | Op::WaitOutput { .. }
            ) {
                return Err(CheckError::new(
                    "invalid operation on a borrowed TCP stream",
                ));
            }
            // The underlying attempt shares its source position. Refill's
            // child bodies retain their own ordinary lexical traversal.
            operation_check(operation, operation_index, definitions, environment, true)?;
        }
        Op::TcpClose { stream, result } => {
            environment.require_tcp(stream)?;
            // Only straight-line close changes owner state. Nested branches
            // and loops may use normal entry exits, which emit cleanup.
            if environment.scopes.len() != 2 {
                return Err(CheckError::new(
                    "explicit TCP close must be directly inside its owner scope",
                ));
            }
            environment.tcp_live = false;
            environment.insert(result, Binding::Scalar { writable: false })?;
        }
        // Refill has an external effect and is admitted only at the host entry.
        Op::Refill(refill) => check_refill(refill, definitions, environment)?,
        // Waiting borrows the host descriptor and publishes only a copied status.
        Op::WaitInput { result } => {
            environment.hosted.input = true;
            environment.insert(result, Binding::Scalar { writable: false })?;
        }
        // Output borrows one initialized field or named view, with offset/count
        // checked in written order. The runtime attempt retains no reference.
        Op::Send {
            source,
            offset,
            count,
            result,
        } => {
            match source {
                BytesSource::Field { .. } | BytesSource::Local(_) | BytesSource::Literal(_) => {
                    environment.byte_view_source(source, Access::Readonly, definitions)?;
                }
                BytesSource::Param(_) | BytesSource::Received(_) => {
                    return Err(CheckError::new(
                        "send requires an initialized Bytes field, a named byte view, or a byte literal",
                    ));
                }
            }
            expressions::expression(offset, definitions, Context::Execution(environment))?;
            expressions::expression(count, definitions, Context::Execution(environment))?;
            environment.hosted.output = true;
            environment.insert(result, Binding::Scalar { writable: false })?;
        }
        // A writable readiness wait shares the host's sole admitted socket.
        Op::WaitOutput { result } => {
            environment.hosted.output = true;
            environment.insert(result, Binding::Scalar { writable: false })?;
        }
        // A receive continuation cannot outlive the client that supplied it.
        // Plain owned bytes and the store itself may survive between sessions.
        Op::NextSession { result } => {
            if environment.scopes.iter().any(|scope| {
                scope
                    .values()
                    .any(|binding| matches!(binding, Binding::Received(_)))
            }) {
                return Err(CheckError::new(
                    "cannot advance the hosted session while a received view is live",
                ));
            }
            environment.hosted.session = true;
            environment.insert(result, Binding::Scalar { writable: false })?;
        }
        // The loop cannot publish an iteration's local bindings at its back edge.
        Op::Loop(body) => loop_scope(body, None, definitions, environment)?,
        Op::Break | Op::Continue => {
            if environment.loop_depth == 0 {
                let keyword = if matches!(operation, Op::Break) {
                    "break"
                } else {
                    "continue"
                };
                return Err(CheckError::new(format!(
                    "`{keyword}` requires an enclosing loop"
                )));
            }
        }
        // Acquisition creates the owner only in the success scope.
        Op::Region {
            name,
            capacity,
            body,
            on_failure,
            ..
        } => {
            expressions::expression(capacity, definitions, Context::Execution(environment))?;
            let root = environment.root();
            environment.scopes.push(HashMap::new());
            // Only the synthesized owner may declare the reserved identity.
            if environment
                .private_placement
                .is_some_and(|placement| placement.owner.matches(operation, operation_index))
            {
                environment.insert_bound(name, Binding::Region { root })?;
            } else {
                environment.insert(name, Binding::Region { root })?;
            }
            operations(body, definitions, environment)?;
            environment.scopes.pop();
            // Acquisition failure never exposes the region binding or successful body.
            scope(on_failure, definitions, environment)?;
        }
        // Batch ownership is lexical and cannot compete for existing backing.
        Op::Batch {
            name,
            region,
            record,
            body,
        } => {
            definitions.record(record)?;
            let region_root = environment.free_tail(region)?;
            let root = environment.root();
            environment.scopes.push(HashMap::new());
            let depth = environment.scopes.len() - 1;
            environment.insert(
                name,
                Binding::Batch {
                    root,
                    region: region_root,
                    depth,
                    record,
                },
            )?;
            operations(body, definitions, environment)?;
            environment.scopes.pop();
        }
        // Leaving an explicit scope makes its reference names unusable.
        Op::Scope(body) => scope(body, definitions, environment)?,
        // Validate the initializer before exposing its name, preventing self-reads.
        Op::Let { name, value } => {
            expressions::expression(value, definitions, Context::Execution(environment))?;
            environment.insert(name, Binding::Scalar { writable: true })?;
        }
        Op::LetBool { name, value } | Op::VarBool { name, value } => {
            expressions::boolean(value, definitions, environment)?;
            environment.insert(
                name,
                Binding::Boolean {
                    writable: matches!(operation, Op::VarBool { .. }),
                },
            )?;
        }
        Op::LetEnum {
            name,
            value,
            mutable,
        } => {
            let kind =
                expressions::enumeration(value, definitions, Context::Execution(environment))?;
            environment.insert(
                name,
                Binding::Enum {
                    kind,
                    writable: *mutable,
                },
            )?;
        }
        Op::BindBytes {
            name,
            source,
            access,
            ranges,
        } => {
            let origin = environment.byte_view_source(source, *access, definitions)?;
            for range in ranges {
                let context = Context::Execution(environment);
                if let Some(offset) = &range.offset {
                    expressions::expression(offset, definitions, context)?;
                }
                expressions::expression(&range.length, definitions, context)?;
            }
            environment.insert(
                name,
                Binding::BytesView {
                    origin,
                    access: *access,
                },
            )?;
        }
        Op::BindResult {
            name,
            function,
            args,
        } => {
            let result = expressions::result_initializer(function, args, definitions, environment)?;
            environment.insert(name, Binding::SavedResult(result))?;
        }
        // Only the success continuation gains an immutable scalar. Failure
        // exits the owner-free frame with the same family or a checked child lift.
        Op::TryLet { function, args, .. } | Op::TryUnit { function, args } => {
            let result = environment
                .function
                .and_then(|function| function.result.as_ref())
                .ok_or_else(|| {
                    CheckError::new(
                        "try requires a scalar typed-result function or a Result<Unit, E> function",
                    )
                })?;
            let (name, success) = try_binding(operation);
            expressions::forward_result(
                function,
                args,
                result,
                expressions::ResultForwarding::Try(success),
                definitions,
                environment,
            )?;
            if let Some(name) = name {
                environment.insert(name, Binding::Scalar { writable: false })?;
            }
        }
        Op::TrySaved { source, .. } | Op::TrySavedUnit { source } => {
            let result = environment
                .function
                .and_then(|function| function.result.as_ref())
                .ok_or_else(|| {
                    CheckError::new(
                        "try requires a scalar typed-result function or a Result<Unit, E> function",
                    )
                })?;
            let Binding::SavedResult(actual) = environment.lookup(source)? else {
                return Err(CheckError::new(format!(
                    "{source} is not a saved result local"
                )));
            };
            let (name, success) = try_binding(operation);
            expressions::check_try_type(source, Some(actual), result, success, definitions)?;
            if let Some(name) = name {
                environment.insert(name, Binding::Scalar { writable: false })?;
            }
        }
        // Scalar mutation cannot replace a backing owner, reference, or loop index.
        Op::Set { name, value } => {
            expressions::expression(value, definitions, Context::Execution(environment))?;
            if !matches!(
                environment.lookup(name)?,
                Binding::Scalar { writable: true }
            ) {
                return Err(CheckError::new(format!(
                    "{name} is not a writable scalar local"
                )));
            }
        }
        Op::SetBool { name, value } => {
            expressions::boolean(value, definitions, environment)?;
            if !matches!(
                environment.lookup(name)?,
                Binding::Boolean { writable: true }
            ) {
                return Err(CheckError::new(format!(
                    "{name} is not a writable boolean local"
                )));
            }
        }
        Op::SetEnum { name, value } => {
            let actual =
                expressions::enumeration(value, definitions, Context::Execution(environment))?;
            let Binding::Enum {
                kind,
                writable: true,
            } = environment.lookup(name)?
            else {
                return Err(CheckError::new(format!(
                    "{name} is not a writable enum local"
                )));
            };
            if kind != actual {
                return Err(CheckError::new(format!(
                    "cannot assign enum {actual} to {kind}"
                )));
            }
        }
        // Append requires a destination-compatible constructor, not uniqueness.
        Op::Append {
            batch,
            producer,
            args,
            status,
        } => {
            let producer = append_producer(batch, producer, args, definitions, environment)?;
            // Numeric status remains confined to the legacy calling convention.
            if producer.error.is_some() {
                return Err(CheckError::new("typed append requires an outcome handler"));
            }
            environment.insert(status, Binding::Scalar { writable: true })?;
        }
        Op::AppendHandled {
            batch,
            producer,
            args,
            result,
            arms,
        } => {
            let producer = append_producer(batch, producer, args, definitions, environment)?;
            let family = producer.error.as_deref().ok_or_else(|| {
                CheckError::new("append outcome handler requires a typed producer")
            })?;
            check_destination_arms(
                arms.iter().map(|arm| (&arm.pattern, arm)),
                Some(family),
                "append",
                definitions,
                |arm, local| {
                    // Expression arms see their active scalar without creating
                    // an entry-operation scope or changing outer bindings.
                    if let Some((name, binding)) = local {
                        environment.push_arm(name, binding)?;
                    }
                    let checked = expressions::expression(
                        &arm.value,
                        definitions,
                        Context::Execution(environment),
                    );
                    if local.is_some() {
                        environment.pop_arm();
                    }
                    checked
                },
            )?;
            // Arms execute after append commits or rolls back. They see existing
            // bindings, and only the completed handler adds this immutable scalar.
            environment.insert(result, Binding::Scalar { writable: false })?;
        }
        Op::HandleResult {
            function,
            args,
            arms,
        } => {
            let family = expressions::result_block_subject(
                function,
                args,
                arms.iter().map(|arm| &arm.pattern),
                definitions,
                environment,
            )?;
            result_blocks(arms, family, definitions, environment)?;
        }
        Op::HandleSavedResult { name, arms } => {
            let family = expressions::saved_result_subject(
                name,
                arms.iter().map(|arm| &arm.pattern),
                definitions,
                environment,
            )?;
            result_blocks(arms, family, definitions, environment)?;
        }
        // Only the success handler acquires a live tail and exposes the record.
        Op::ConstructHandled {
            region,
            producer,
            args,
            arms,
        } => {
            // The exception is for this destination operand alone. Arguments
            // and arms still use ordinary protected lexical lookup.
            let region_root = if environment
                .private_placement
                .is_some_and(|placement| placement.site.matches(operation, operation_index))
            {
                environment.require_free_tail(region, environment.lookup_bound(region)?)?
            } else {
                environment.free_tail(region)?
            };
            check_construct(region_root, producer, args, arms, definitions, environment)?;
        }
        // Runtime bounds checking chooses a record; its compile-time root stays fixed.
        Op::Borrow {
            name,
            batch,
            index,
            access,
        } => {
            let reference = environment.batch_view(batch, *access)?;
            expressions::expression(index, definitions, Context::Execution(environment))?;
            environment.insert(name, Binding::Reference(reference))?;
        }
        // Aliasing may reduce permission but never manufacture write authority.
        Op::Alias {
            name,
            source,
            access,
        } => {
            let mut reference = environment.reference(source)?;
            if reference.access == Access::Readonly && *access == Access::Readwrite {
                return Err(CheckError::new("cannot upgrade a readonly reference alias"));
            }
            reference.access = *access;
            environment.insert(name, Binding::Reference(reference))?;
        }
        // Rebinding first checks lifetime, then type/access, then the subset limit.
        Op::AssignRef { name, source } => {
            // The direct success binding has no mutable pointer slot. An alias
            // may have its own slot but still carries the original root.
            if matches!(environment.lookup(name)?, Binding::ScopedReference(_)) {
                return Err(CheckError::new(
                    "cannot rebind a scoped record's success reference",
                ));
            }
            let destination = environment.reference(name)?;
            let source = environment.reference(source)?;
            let destination_depth = environment
                .scopes
                .iter()
                .rposition(|scope| scope.contains_key(name.as_str()))
                .expect("reference lookup succeeded");
            if source.root_depth > destination_depth {
                return Err(CheckError::new(format!(
                    "reference stored in {name} would outlive its owner"
                )));
            }
            if destination.record != source.record
                || (destination.access == Access::Readwrite && source.access == Access::Readonly)
            {
                return Err(CheckError::new(
                    "reference assignment has incompatible schema or readonly permission",
                ));
            }
            // Keeping the root fixed makes branch/loop origin loss impossible in this subset.
            if destination.root != source.root {
                return Err(CheckError::new(
                    "cross-root reference rebinding is unsupported in this first checker",
                ));
            }
        }
        // A scalar store is valid only through a writable path of the same kind.
        Op::Write {
            reference,
            path,
            value,
        } => {
            let reference = environment.writable_reference(reference)?;
            expressions::require_field(definitions, reference.record, path, FieldKind::U64)?;
            expressions::expression(value, definitions, Context::Execution(environment))?;
        }
        Op::WriteBool {
            reference,
            path,
            value,
        } => {
            let reference = environment.writable_reference(reference)?;
            expressions::require_field(definitions, reference.record, path, FieldKind::Bool)?;
            expressions::boolean(value, definitions, environment)?;
        }
        Op::WriteEnum {
            reference,
            path,
            value,
        } => {
            let reference = environment.writable_reference(reference)?;
            let actual =
                expressions::enumeration(value, definitions, Context::Execution(environment))?;
            if !matches!(expressions::field(definitions, reference.record, path)?, FieldKind::Enum(kind) if kind == actual)
            {
                return Err(CheckError::new(format!(
                    "field path {} cannot store enum {actual}",
                    path.join(".")
                )));
            }
        }
        // Byte stores can replace a byte but cannot mutate its payload length/header.
        Op::WriteByte {
            reference,
            path,
            index,
            value,
        } => {
            let reference = environment.writable_reference(reference)?;
            expressions::require_field(definitions, reference.record, path, FieldKind::Bytes)?;
            expressions::expression(index, definitions, Context::Execution(environment))?;
            expressions::expression(value, definitions, Context::Execution(environment))?;
        }
        Op::WriteInputByte {
            param,
            index,
            value,
        } => {
            if !environment.writable_byte_param(*param) {
                return Err(CheckError::new(
                    "byte store requires a writable Bytes parameter",
                ));
            }
            expressions::expression(index, definitions, Context::Execution(environment))?;
            expressions::expression(value, definitions, Context::Execution(environment))?;
        }
        Op::WriteViewByte { name, index, value } => {
            environment.byte_view(name, Access::Readwrite)?;
            expressions::expression(index, definitions, Context::Execution(environment))?;
            expressions::expression(value, definitions, Context::Execution(environment))?;
        }
        Op::MoveInputBytes {
            destination,
            source,
        } => {
            if !environment.writable_byte_param(*destination) || !environment.byte_param(*source) {
                return Err(CheckError::new(
                    "byte move requires a writable destination and a readable source Bytes parameter",
                ));
            }
        }
        // Reset must preserve every still-usable reference's storage lifetime.
        Op::Reset { batch } => {
            let Binding::Batch { root, .. } = environment.owned_batch(batch)? else {
                unreachable!()
            };
            // Every currently usable lexical alias blocks reset, including outer scopes.
            for bindings in &environment.scopes {
                if bindings.values().any(|binding| match binding {
                    Binding::Reference(reference) => reference.root == root,
                    Binding::Received(source) => source.root == root,
                    Binding::BytesView {
                        origin: Some(source),
                        ..
                    } => source.root == root,
                    _ => false,
                }) {
                    return Err(CheckError::new(format!(
                        "cannot reset batch {batch} while a reference is live"
                    )));
                }
            }
        }
        // The count is evaluated outside a fresh body scope with a readonly index.
        Op::Repeat { count, index, body } => {
            expressions::expression(count, definitions, Context::Execution(environment))?;
            loop_scope(
                body,
                Some((index, Binding::Scalar { writable: false })),
                definitions,
                environment,
            )?;
        }
        // The cursor views the snapshot prefix; extending the owner remains sound.
        Op::ForEach {
            batch,
            reference,
            access,
            body,
        } => {
            let view = environment.batch_view(batch, *access)?;
            // The cursor is a normal rooted path: append stays valid, reset cannot pass it.
            loop_scope(
                body,
                Some((reference, Binding::Reference(view))),
                definitions,
                environment,
            )?;
        }
        // Both branches must be well-typed even when a condition is currently constant.
        Op::If {
            condition,
            then_body,
            else_body,
        } => {
            expressions::condition(condition, definitions, Context::Execution(environment))?;
            // Both branches are checked; same-root rebinding preserves all outer root facts.
            scope(then_body, definitions, environment)?;
            scope(else_body, definitions, environment)?;
        }
        // Assertions and printing observe only valid scalar expressions.
        Op::Assert(condition) => {
            expressions::condition(condition, definitions, Context::Execution(environment))?
        }
        Op::Print(value) => {
            expressions::expression(value, definitions, Context::Execution(environment))?
        }
        Op::PrintText(text) => {
            // Native text output reads through the added terminator. Reject an
            // embedded terminator so printing always consumes the whole literal.
            if text.as_bytes().contains(&0) {
                return Err(CheckError::new("printed text cannot contain a NUL byte"));
            }
        }
        // Scalar and typed results have distinct checked calling conventions.
        Op::Return(value) => {
            if !environment.in_function() {
                return Err(CheckError::new(
                    "scalar return requires an ordinary function",
                ));
            }
            if environment
                .function
                .and_then(|function| function.result.as_ref())
                .is_some()
            {
                return Err(CheckError::new(
                    "typed-result function cannot return a bare scalar",
                ));
            }
            if environment
                .function
                .is_some_and(|function| function.returns_bool)
            {
                return Err(CheckError::new(
                    "boolean function cannot return an unsigned scalar",
                ));
            }
            expressions::expression(value, definitions, Context::Execution(environment))?;
        }
        Op::ReturnBool(value) => {
            if !environment
                .function
                .is_some_and(|function| function.returns_bool)
            {
                return Err(CheckError::new("boolean return requires a bool function"));
            }
            expressions::boolean(value, definitions, environment)?;
        }
        Op::ReturnResult(value) => {
            let result = environment
                .function
                .and_then(|function| function.result.as_ref())
                .ok_or_else(|| {
                    CheckError::new("typed result return requires a typed-result function")
                })?;
            let family = result.family.as_str();
            match value {
                ResultValue::Ok(value) if result.success == ResultSuccess::U64 => {
                    expressions::expression(value, definitions, Context::Execution(environment))?
                }
                ResultValue::OkUnit if result.success == ResultSuccess::Unit => {}
                ResultValue::Ok(_) | ResultValue::OkUnit => {
                    return Err(CheckError::new(
                        "return success must match the declared Result success type",
                    ));
                }
                ResultValue::Error {
                    family: actual,
                    case,
                    payload,
                } => {
                    let declared = definitions.error_case(family, actual, case)?;
                    // An error carries one scalar exactly when its declaration says so.
                    if declared.payload != payload.is_some() {
                        return Err(CheckError::new(format!(
                            "wrong payload shape for error {family}.{case}"
                        )));
                    }
                    if let Some(value) = payload {
                        expressions::expression(
                            value,
                            definitions,
                            Context::Execution(environment),
                        )?;
                    }
                }
                ResultValue::Call { function, args } => expressions::forward_result(
                    function,
                    args,
                    result,
                    expressions::ResultForwarding::Exact,
                    definitions,
                    environment,
                )?,
                ResultValue::Saved(name) => {
                    let Binding::SavedResult(actual) = environment.lookup(name)? else {
                        return Err(CheckError::new(format!(
                            "{name} is not a saved result local"
                        )));
                    };
                    if actual.family != family {
                        return Err(CheckError::new(format!(
                            "saved result {name} must have the exact return family {family}"
                        )));
                    }
                    if actual != result {
                        return Err(CheckError::new(format!(
                            "saved result {name} must have the exact success type"
                        )));
                    }
                }
            }
        }
        // Entry completion carries no expression, reference, or escape value.
        // Later operations remain structurally checked even when unreachable.
        Op::ExitZero => {}
        Op::OpaqueRegion(name) => {
            // A measurement barrier may observe backing, never reinterpret another kind.
            if !matches!(environment.lookup(name)?, Binding::Region { .. }) {
                return Err(CheckError::new(format!("{name} is not a region")));
            }
        }
    }
    Ok(())
}

/// Resolve whether a checked try form introduces a scalar success binding.
fn try_binding(operation: &Op) -> (Option<&str>, ResultSuccess) {
    match operation {
        Op::TryLet { name, .. } | Op::TrySaved { name, .. } => (Some(name), ResultSuccess::U64),
        Op::TryUnit { .. } | Op::TrySavedUnit { .. } => (None, ResultSuccess::Unit),
        _ => unreachable!("only try operations carry this expectation"),
    }
}

/// Validate each statement arm under a fresh scope with only its active payload.
fn result_blocks<'p>(
    arms: &'p [ResultBlockArm],
    family: &'p str,
    definitions: &Definitions<'p>,
    environment: &mut Environment<'p>,
) -> Result<(), CheckError> {
    for arm in arms {
        environment.scopes.push(HashMap::new());
        let local = match &arm.pattern {
            ResultPattern::Ok(Some(name))
            | ResultPattern::Error {
                binding: Some(name),
                ..
            } => Some((name, Binding::Scalar { writable: false })),
            ResultPattern::Fallback(Some(name)) => Some((name, Binding::Error { family })),
            _ => None,
        };
        if let Some((name, binding)) = local {
            environment.insert(name, binding)?;
        }
        operations(&arm.body, definitions, environment)?;
        environment.scopes.pop();
    }
    Ok(())
}

/// Resolve an append's receiving schema and verify its once-evaluated actuals.
/// The constructor can read a live input, including this batch's initialized
/// prefix, but it can write only the unpublished suffix supplied by the caller.
fn append_producer<'p>(
    batch: &str,
    producer: &str,
    args: &'p [Argument],
    definitions: &Definitions<'p>,
    environment: &Environment<'p>,
) -> Result<&'p Producer, CheckError> {
    let Binding::Batch { record, .. } = environment.owned_batch(batch)? else {
        unreachable!("batch lookup checks the binding kind")
    };
    let definition = definitions.producer(producer)?;
    // Schema and arity establish a compatible receiving slot and argument list.
    if record != definition.record || args.len() != definition.params.len() {
        return Err(CheckError::new(format!(
            "append producer {producer} has incompatible result or arguments"
        )));
    }
    expressions::arguments(
        args,
        &definition.params,
        definitions,
        Context::Execution(environment),
    )?;
    Ok(definition)
}

/// Check a direct scoped constructor and its exhaustive lexical handlers.
///
/// A successful arm owns the destination tail through its entire body, even
/// when it discards the record. The failure arms can reuse that region. Only
/// the success reference's checked root can name the new initialized bytes.
fn check_construct<'p>(
    region_root: usize,
    producer: &str,
    args: &'p [Argument],
    arms: &'p [ConstructArm],
    definitions: &Definitions<'p>,
    environment: &mut Environment<'p>,
) -> Result<(), CheckError> {
    let definition = definitions.producer(producer)?;
    let family = definition.error.as_deref();
    expressions::arguments(
        args,
        &definition.params,
        definitions,
        Context::Execution(environment),
    )?;
    check_destination_arms(
        arms.iter().map(|arm| (&arm.pattern, arm)),
        family,
        "construct",
        definitions,
        |arm, local| {
            let success = matches!(arm.pattern, AppendPattern::Ok);
            // A programmatic IR client cannot bind storage on a failure edge.
            if !success && arm.reference.is_some() {
                return Err(CheckError::new(
                    "only a construct ok arm may bind the record",
                ));
            }
            // The first block-handler form admits only an unbound catch-all.
            if matches!(arm.pattern, AppendPattern::Fallback(Some(_))) {
                return Err(CheckError::new(
                    "construct error fallback cannot bind a value",
                ));
            }
            environment.scopes.push(HashMap::new());
            // Bind the direct record root at this scope's depth. The existing
            // alias/rebind rules keep any projection tied to this lifetime.
            if success {
                environment.constructed_tails.push(region_root);
                if let Some(binding) = &arm.reference {
                    let reference = Reference {
                        root: environment.root(),
                        root_depth: environment.scopes.len() - 1,
                        record: &definition.record,
                        access: binding.access,
                    };
                    environment.insert(&binding.name, Binding::ScopedReference(reference))?;
                }
            }
            // Failure exposes only an active rejection payload, if requested.
            if let Some((name, binding)) = local {
                environment.insert(name, binding)?;
            }
            operations(&arm.body, definitions, environment)?;
            if success {
                environment.constructed_tails.pop();
            }
            environment.scopes.pop();
            Ok(())
        },
    )
}

/// Verify the destination outcome partition and visit each active arm.
///
/// The callback receives only the binding justified by this pattern. It checks
/// each arm's own body and may advance the lexical traversal; no target data
/// or result storage is created by partition checking.
fn check_destination_arms<'p, A>(
    arms: impl ExactSizeIterator<Item = (&'p AppendPattern, A)>,
    family: Option<&'p str>,
    destination: &str,
    definitions: &Definitions<'p>,
    mut check: impl FnMut(A, Option<(&'p str, Binding<'p>)>) -> Result<(), CheckError>,
) -> Result<(), CheckError> {
    let declared = family
        .map(|family| definitions.error_family(family))
        .transpose()?;
    let arm_count = arms.len();
    let mut success = false;
    let mut full = false;
    let mut overflow = false;
    let mut fallback = false;
    let mut cases = HashSet::new();
    // The partition covers success and the disjoint storage and domain
    // failures. A final fallback can fill gaps only on the failure side.
    for (index, (pattern, arm)) in arms.enumerate() {
        let local = match pattern {
            AppendPattern::Ok | AppendPattern::Full | AppendPattern::SizeOverflow => {
                // These three outcomes carry no binding. Record which were
                // covered even when the compiler can prove one is unreachable.
                let (covered, name) = match pattern {
                    AppendPattern::Ok => (&mut success, "ok"),
                    AppendPattern::Full => (&mut full, "Full"),
                    AppendPattern::SizeOverflow => (&mut overflow, "SizeOverflow"),
                    _ => unreachable!("only the three payload-free outcomes enter here"),
                };
                if std::mem::replace(covered, true) {
                    return Err(CheckError::new(format!(
                        "duplicate {destination} {name} arm"
                    )));
                }
                None
            }
            AppendPattern::Rejected {
                family: actual,
                case,
                binding,
            } => {
                let family = family.ok_or_else(|| {
                    CheckError::new(format!(
                        "named {destination} rejection requires a typed producer"
                    ))
                })?;
                let case = definitions.error_case(family, actual, case)?;
                // Distinct nominal cases cannot overlap and inactive payload
                // lanes cannot be exposed through a scalar binding.
                if !cases.insert(case.name.as_str()) {
                    return Err(CheckError::new(format!(
                        "duplicate {destination} rejection arm {family}.{}",
                        case.name
                    )));
                }
                if binding.is_some() && !case.payload {
                    return Err(CheckError::new(format!(
                        "{destination} rejection arm {family}.{} has no payload to bind",
                        case.name
                    )));
                }
                binding
                    .as_deref()
                    .map(|name| (name, Binding::Scalar { writable: false }))
            }
            AppendPattern::Fallback(binding) => {
                // Enforcing the last position also prohibits duplicate fallbacks.
                if index + 1 != arm_count {
                    return Err(CheckError::new(format!(
                        "{destination} error fallback must be the last arm"
                    )));
                }
                fallback = true;
                match (binding.as_deref(), family) {
                    (Some(name), Some(family)) => Some((name, Binding::Error { family })),
                    (Some(_), None) => {
                        return Err(CheckError::new(format!(
                            "{destination} error fallback cannot bind a value"
                        )));
                    }
                    (None, _) => None,
                }
            }
        };
        // A binding introduced by a public IR client must have a usable name.
        if local.is_some_and(|(name, _)| name.is_empty()) {
            return Err(CheckError::new(format!(
                "{destination} arm binding cannot be empty"
            )));
        }
        check(arm, local)?;
    }
    // Success always needs a spelled-out arm. Without fallback, check
    // storage outcomes and the complete declared family before lowering.
    if !success {
        return Err(CheckError::new(format!("missing {destination} ok arm")));
    }
    if !fallback {
        let (Some(family), Some(declared)) = (family, declared) else {
            return Err(CheckError::new(format!(
                "untyped {destination} requires an error fallback arm"
            )));
        };
        if !full || !overflow {
            let missing = if !full { "Full" } else { "SizeOverflow" };
            return Err(CheckError::new(format!(
                "missing {destination} {missing} arm"
            )));
        }
        if let Some(case) = declared
            .cases
            .iter()
            .find(|case| !cases.contains(case.name.as_str()))
        {
            return Err(CheckError::new(format!(
                "missing {destination} rejection arm {family}.{}",
                case.name
            )));
        }
    }
    Ok(())
}

/// Validate one refill and its disjoint Data and nondata branches.
///
/// The destination's owning batch and field path survive aliases, rebindings,
/// and further narrowing. Ranges within the same field are treated as possibly
/// overlapping because the checker does not prove their bounds are disjoint.
fn check_refill<'p>(
    refill: &'p Refill,
    definitions: &Definitions<'p>,
    environment: &mut Environment<'p>,
) -> Result<(), CheckError> {
    // All attempts use the descriptor admitted once by the executable host.
    environment.hosted.input = true;
    if !matches!(
        refill.destination,
        BytesSource::Field { .. } | BytesSource::Local(_)
    ) {
        return Err(CheckError::new(
            "refill requires a writable Bytes field or a named writable byte view",
        ));
    }
    let target = environment
        .byte_view_source(&refill.destination, Access::Readwrite, definitions)?
        .ok_or_else(|| CheckError::new("refill destination requires a live byte field"))?;
    expressions::expression(&refill.limit, definitions, Context::Execution(environment))?;
    // Inspect all active scopes, even if an inner name hides a view binding.
    // Same-root paths can reach the same record through independent borrows.
    if environment.scopes.iter().any(|scope| {
        scope
            .values()
            .any(|binding| matches!(binding, Binding::Received(origin) if *origin == target))
    }) {
        return Err(CheckError::new(
            "cannot refill a byte field while its received view is live",
        ));
    }
    environment.scopes.push(HashMap::new());
    environment.insert(&refill.data_name, Binding::Received(target))?;
    operations(&refill.data_body, definitions, environment)?;
    environment.scopes.pop();
    // Nondata outcomes contain a readonly status only. Discard Data authority
    // before checking this sibling and discard its scalar at the merge.
    environment.scopes.push(HashMap::new());
    environment.insert(&refill.result_name, Binding::Scalar { writable: false })?;
    operations(&refill.other_body, definitions, environment)?;
    environment.scopes.pop();
    Ok(())
}

/// Validate operations in a temporary lexical scope and discard its local names.
///
/// Parent root identities remain unchanged because cross-root rebinding is not
/// implemented. Every error aborts validation; no partially checked state escapes.
fn scope<'p>(
    body: &'p [Op],
    definitions: &Definitions<'p>,
    environment: &mut Environment<'p>,
) -> Result<(), CheckError> {
    environment.scopes.push(HashMap::new());
    operations(body, definitions, environment)?;
    environment.scopes.pop();
    Ok(())
}

/// Check a loop body with a fresh scope and its optional index or element binding.
fn loop_scope<'p>(
    body: &'p [Op],
    local: Option<(&'p str, Binding<'p>)>,
    definitions: &Definitions<'p>,
    environment: &mut Environment<'p>,
) -> Result<(), CheckError> {
    environment.scopes.push(HashMap::new());
    if let Some((name, binding)) = local {
        environment.insert(name, binding)?;
    }
    environment.loop_depth += 1;
    operations(body, definitions, environment)?;
    environment.loop_depth -= 1;
    environment.scopes.pop();
    Ok(())
}
