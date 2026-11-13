//! Structured IR for checked region construction and synchronous borrowed calls.
//!
//! This compiler API does not freeze source syntax. Schemas and destination-only
//! producers and ordinary calls retain lexical roots until checking. Strings and
//! vectors allocate in the compiler only. Scalar arithmetic wraps explicitly here;
//! source-language
//! defaults remain open. Native storage arithmetic is checked independently.

/// Complete unchecked compiler input, never itself an authorization to lower.
#[derive(Clone, Debug, Default)]
pub struct Program {
    /// Named, acyclic logical schemas.
    pub records: Vec<Record>,
    /// Named, statically resolved destination constructors.
    pub producers: Vec<Producer>,
    /// Outermost helper declarations and structured entry operations in evaluation order.
    pub body: Vec<Op>,
}

/// A logical inline record; its representation metadata is not a writable field.
#[derive(Clone, Debug)]
pub struct Record {
    /// Unique schema identity.
    pub name: String,
    /// Field construction and traversal order.
    pub fields: Vec<Field>,
}

/// One named inline component.
#[derive(Clone, Debug)]
pub struct Field {
    /// Unique name within this record.
    pub name: String,
    /// Plain-data representation kind.
    pub kind: FieldKind,
}

/// Supported field kinds; references/resource handles are deliberately absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// One unsigned scalar occupying eight bytes.
    U64,
    /// Frozen length metadata followed by initialized inline payload.
    Bytes,
    /// Inline instance of the named acyclic schema.
    Record(String),
}

/// Destination constructor with typed value/borrow parameters and no external writes.
#[derive(Clone, Debug)]
pub struct Producer {
    /// Unique function identity.
    pub name: String,
    /// Result schema, built in its receiving slot.
    pub record: String,
    /// Optional nominal family for typed rejection from this destination call.
    pub error: Option<String>,
    /// Ordered explicit parameter kinds, independent of their native ABI operands.
    pub params: Vec<Parameter>,
    /// Ordered initialization and possible explicit rejection.
    pub body: Vec<Build>,
}

/// Permitted synchronous producer input kinds; neither can own escaping storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parameter {
    /// One unsigned scalar copied at argument evaluation.
    U64,
    /// A readonly borrow of initialized payload bytes with a captured logical extent.
    Bytes,
}

/// One typed actual argument, evaluated once before entering its producer.
#[derive(Clone, Debug)]
pub enum Argument {
    /// Compute a scalar value without an aggregate temporary.
    Scalar(Expr),
    /// Capture a readonly source location/extent, without copying its contents.
    Bytes(BytesSource),
    /// Lend the exact readonly prefix in a live refill data continuation.
    Received(String),
}

/// Origin of a synchronous byte borrow; raw addresses and unfinished output are absent.
#[derive(Clone, Debug)]
pub enum BytesSource {
    /// Forward a byte formal at this index in the enclosing typed parameter list.
    Param(usize),
    /// Borrow a complete byte field through a live initialized record reference.
    Field {
        /// Lexical record reference retaining its checked batch root.
        reference: String,
        /// Nonempty schema projection ending in a Bytes field.
        path: Vec<String>,
    },
}

/// Ordinary synchronous helper with no owner or reference-valued result.
#[derive(Clone, Debug)]
pub struct Function {
    /// Unique statically resolved name within the ordinary function table.
    pub name: String,
    /// Ordered explicit arguments and their lexical identities.
    pub params: Vec<FunctionParameter>,
    /// Nominal family of a scalar result, or no family for a plain `u64`.
    pub result: Option<String>,
    /// Checked operations that return the declared type on every completion path.
    pub body: Vec<Op>,
}

/// Static declaration of a closed error family whose leaves may carry one scalar.
#[derive(Clone, Debug)]
pub struct ErrorFamily {
    /// Nominal identity, qualified during source binding.
    pub name: String,
    /// Nonempty, unique leaves in declaration order, with each child's leaves
    /// stored in one contiguous block under its alias.
    pub cases: Vec<ErrorCase>,
    /// Direct nominal children admitted by this family.
    pub children: Vec<ErrorChild>,
}

impl ErrorFamily {
    /// Resolve a checked failure lift from `source` to this family. The
    /// zero-based offset maps one-based source tags to destination tags
    /// without changing the payload. Unrelated families return `None`.
    pub fn try_error_offset(&self, source: &str) -> Option<u8> {
        // Same-family propagation reuses the existing representation.
        if self.name == source {
            return Some(0);
        }
        let child = self.children.iter().find(|child| child.family == source)?;
        // Checked IR guarantees this block is complete and in source tag order.
        let first = self.cases.iter().position(|case| {
            case.name
                .split_once('.')
                .is_some_and(|(alias, _)| alias == child.alias)
        })?;
        u8::try_from(first).ok()
    }
}

/// Direct inclusion of a nominal family in an error union.
#[derive(Clone, Debug)]
pub struct ErrorChild {
    /// Unqualified path segment prepended to each child leaf.
    pub alias: String,
    /// Nominal identity of the included family.
    pub family: String,
}

/// One nominal alternative of an error family.
#[derive(Clone, Debug)]
pub struct ErrorCase {
    /// Leaf path within this family, qualified by aliases for included cases.
    pub name: String,
    /// Whether the case has one `u64` payload in the shared scalar lane.
    pub payload: bool,
}

/// Value produced by one return in a scalar-result helper.
#[derive(Clone, Debug)]
pub enum ResultValue {
    /// Evaluate the entire unsigned success range without a reserved sentinel.
    Ok(Expr),
    /// Construct a checked case of the declared family.
    Error {
        /// Nominal family identity.
        family: String,
        /// Declared case name.
        case: String,
        /// Scalar operand present exactly for cases that declare a payload.
        payload: Option<Expr>,
    },
    /// Forward the result of one ordered call with the exact same family.
    Call {
        /// Statically resolved ordinary callee identity.
        function: String,
        /// Actuals evaluated once in written order.
        args: Vec<FunctionArgument>,
    },
}

/// One arm of an immediate result handler producing an unsigned value.
#[derive(Clone, Debug)]
pub struct ResultArm {
    /// Checked case selection and any arm-local binding.
    pub pattern: ResultPattern,
    /// Evaluate only after the arm's case has been selected.
    pub value: Expr,
}

/// A payload binder, named error case, or residual error arm.
#[derive(Clone, Debug)]
pub enum ResultPattern {
    /// Bind the success scalar when named; absence discards it explicitly.
    Ok(Option<String>),
    /// Select one nominal error case and optionally bind its scalar payload.
    Error {
        /// Nominal family of the case.
        family: String,
        /// Declared case name.
        case: String,
        /// Arm-local payload name. Absence leaves the scalar lane unobserved.
        binding: Option<String>,
    },
    /// Cover remaining errors, optionally naming the typed error in this arm.
    Fallback(Option<String>),
}

/// One formal in an ordinary function's local frame.
#[derive(Clone, Debug)]
pub struct FunctionParameter {
    /// Name of a scalar or state binding, or the diagnostic name of a Bytes slot.
    pub name: String,
    /// Required access and representation category.
    pub kind: FunctionParameterKind,
}

/// Supported ordinary parameter categories with call-scoped storage authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FunctionParameterKind {
    /// Copied unsigned value, immutable within the callee.
    U64,
    /// Readonly byte borrow whose captured extent bounds every load.
    Bytes,
    /// Writable reference to an existing fixed scalar-only record schema.
    State(String),
}

/// Typed actual captured in source order before entering an ordinary helper.
#[derive(Clone, Debug)]
pub enum FunctionArgument {
    /// Evaluate one scalar value, including any nested calls or checked loads.
    Scalar(Expr),
    /// Reborrow the caller's live writable record binding for this call.
    State(String),
    /// Capture a byte location and its extent, then optionally narrow that extent.
    Bytes {
        /// Initialized published field or forwarded byte formal.
        source: BytesSource,
        /// Logical prefix evaluated once after capturing the source. Oversize traps.
        prefix: Option<Expr>,
    },
    /// Reborrow the readonly prefix produced by the active refill's Data arm.
    Received(String),
}

/// One experimental host-backed read attempt into a fixed initialized byte field.
///
/// A positive result lends the fresh prefix only to `data_body`. The caller
/// retains the slot and one admitted descriptor across attempts.
#[derive(Clone, Debug)]
pub struct Refill {
    /// Writable reference into the live storage root that owns the slot.
    pub reference: String,
    /// Path to the initialized byte field within that reference's record.
    pub path: Vec<String>,
    /// Positive request limit evaluated once after capturing the writable slot.
    pub limit: Expr,
    /// New lexical readonly prefix binding in the positive branch.
    pub data_name: String,
    /// Runs only after successful validation of this attempt's positive count.
    pub data_body: Vec<Op>,
    /// New readonly scalar holding the exact adapter result in the other branch.
    pub result_name: String,
    /// Handles EOF, interruption, would-block, or encoded native error.
    pub other_body: Vec<Op>,
}

/// Constructor-only operations; the API cannot express writes to published data.
#[derive(Clone, Debug)]
pub enum Build {
    /// Initialize the next scalar field.
    U64 {
        /// Expected next field name.
        field: String,
        /// Pure scalar initializer.
        value: Expr,
    },
    /// Initialize bytes using a scoped index; each value is explicitly narrowed
    /// to its low eight bits, independently of source-language conversion defaults.
    Bytes {
        /// Expected next byte field.
        field: String,
        /// Number of payload bytes, evaluated once.
        len: Expr,
        /// Value for each `Index` in `0..len`.
        value: Expr,
    },
    /// Construct a nested field directly through another constructor call.
    /// Typed child failures lift into the parent's family before publication.
    Record {
        /// Expected next nested record field.
        field: String,
        /// Named statically resolved callee.
        producer: String,
        /// Typed arguments evaluated left to right, retaining forwarded borrows.
        args: Vec<Argument>,
    },
    /// Delegate the complete result to a constructor with the exact same
    /// record and error family. This is the entire body, with no reservation
    /// or field initialization by the forwarding constructor.
    Forward {
        /// Named constructor that receives the original destination.
        producer: String,
        /// Actuals evaluated once in order before the delegated call.
        args: Vec<Argument>,
    },
    /// Abort the owning append with status 3 if this condition holds.
    FailIf(Condition),
    /// Reject with a case of the producer's nominal family when the condition holds.
    Reject {
        /// Evaluate before the rejection payload or any subsequent field.
        condition: Condition,
        /// Declared case in this producer's error family.
        case: String,
        /// Present exactly when the selected case declares one scalar payload.
        payload: Option<Expr>,
    },
}

/// One typed append arm. Its scalar expression observes the finalized batch.
#[derive(Clone, Debug)]
pub struct AppendArm {
    /// Outcome selection and any binding scoped to this arm.
    pub pattern: AppendPattern,
    /// Scalar computed only after this outcome is selected.
    pub value: Expr,
}

/// One lexical handler of a scoped destination construction.
#[derive(Clone, Debug)]
pub struct ConstructArm {
    /// Outcome selection and any scalar rejection payload binding.
    pub pattern: AppendPattern,
    /// Readonly reference to the completed record, allowed only on success.
    pub reference: Option<String>,
    /// Operations checked and executed only within the selected outcome scope.
    pub body: Vec<Op>,
}

/// Recoverable outcomes of the checked publication boundary.
#[derive(Clone, Debug)]
pub enum AppendPattern {
    /// A completed record has been published; success carries no value.
    Ok,
    /// The next reservation exceeded the batch's backing.
    Full,
    /// Checked size arithmetic or representation width failed.
    SizeOverflow,
    /// The destination graph returned a declared error before publication.
    Rejected {
        /// Nominal identity of the producer's error family.
        family: String,
        /// Selected case of that family.
        case: String,
        /// Optional name for the case's active scalar payload.
        binding: Option<String>,
    },
    /// Cover every otherwise unhandled error, optionally naming the typed error.
    Fallback(Option<String>),
}

/// Path access permission, never a uniqueness assertion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Read this path while another writable alias may still mutate the data.
    Readonly,
    /// Read and mutate existing fixed-size data through this path.
    Readwrite,
}

/// Scalar expressions; memory observations require checked live bindings.
#[derive(Clone, Debug)]
pub enum Expr {
    /// Exact unsigned constant.
    Const(u64),
    /// Eager scalar-helper application without a runtime closure or function object.
    /// Actuals evaluate once in the enclosing context; body Param indexes a fresh
    /// all-u64 frame and cannot observe enclosing byte inputs or initializer Index.
    Apply {
        /// Scalar actuals evaluated left to right, including unused values.
        args: Vec<Expr>,
        /// Pure scalar template evaluated with only the newly captured parameters.
        body: Box<Expr>,
    },
    /// Invoke a checked ordinary helper synchronously and obtain its scalar result.
    Call {
        /// Statically resolved ordinary callee identity.
        function: String,
        /// Ordered arguments, captured fully before beginning the next actual.
        args: Vec<FunctionArgument>,
    },
    /// Call once and evaluate the selected arm of an exhaustive result handler.
    Handle {
        /// Statically resolved scalar-result callee identity.
        function: String,
        /// Ordered actuals, evaluated before selecting an arm.
        args: Vec<FunctionArgument>,
        /// Disjoint named arms, with at most one final residual failure arm.
        arms: Vec<ResultArm>,
    },
    /// Lexical unsigned binding.
    Local(String),
    /// Zero-based U64 constructor or pure Apply formal.
    Param(usize),
    /// Captured extent of a readonly byte formal in this function's typed inputs.
    InputLen(usize),
    /// Bounds-checked read from a readonly byte formal; byte value widens to u64.
    InputByte {
        /// Index of a Bytes formal, never a numeric pointer parameter.
        param: usize,
        /// Unsigned index evaluated once before checking and loading.
        index: Box<Expr>,
    },
    /// Current byte initializer index; unavailable outside its element expression.
    Index,
    /// Explicit arithmetic with left-to-right operands.
    Binary {
        /// Arithmetic/bitwise operation.
        op: Binary,
        /// First evaluated operand.
        left: Box<Expr>,
        /// Second evaluated operand.
        right: Box<Expr>,
    },
    /// Read a scalar reached through a live reference.
    Read {
        /// Reference binding.
        reference: String,
        /// Nested field projection.
        path: Vec<String>,
    },
    /// Bounds-checked byte read from a frozen payload.
    Byte {
        /// Reference binding.
        reference: String,
        /// Nested byte field projection.
        path: Vec<String>,
        /// Unsigned element index.
        index: Box<Expr>,
    },
    /// Immutable byte payload length.
    BytesLen {
        /// Reference binding.
        reference: String,
        /// Nested byte field projection.
        path: Vec<String>,
    },
    /// Published element count of a live batch.
    BatchLen(String),
    /// Current region marker for lifecycle accounting.
    Used(String),
    /// Native address observation for identity checks; cannot create a pointer.
    /// Differential tests compare identities, not printed physical addresses.
    Address(String),
    /// Native monotonic-clock observation for measurement harnesses only.
    /// This is not a pure producer expression or a source-language clock API.
    Nanoseconds,
}

/// Explicit unsigned arithmetic; storage address/size arithmetic is not wrapping.
#[derive(Clone, Copy, Debug)]
pub enum Binary {
    /// Addition modulo 2^64.
    AddWrapping,
    /// Subtraction modulo 2^64.
    SubWrapping,
    /// Multiplication modulo 2^64.
    MulWrapping,
    /// Bitwise exclusive-or.
    Xor,
    /// Bitwise conjunction.
    And,
}

/// Unsigned scalar comparison, evaluating the left expression first.
#[derive(Clone, Debug)]
pub struct Condition {
    /// Relationship to evaluate.
    pub op: Compare,
    /// First scalar expression.
    pub left: Expr,
    /// Second scalar expression.
    pub right: Expr,
}

/// Comparison relationships for assertions, branches, and construction rejection.
#[derive(Clone, Copy, Debug)]
pub enum Compare {
    /// Equal values.
    Eq,
    /// Unequal values.
    Ne,
    /// Unsigned less-than.
    Lt,
    /// Unsigned less-than-or-equal.
    Le,
    /// Unsigned greater-than.
    Gt,
    /// Unsigned greater-than-or-equal.
    Ge,
}

/// Structured execution; arbitrary jumps cannot bypass lexical marker cleanup.
#[derive(Clone, Debug)]
pub enum Op {
    /// Declare an ordinary function at entry's outermost level, with no runtime action.
    Function(Function),
    /// Declare a nominal error family at entry's outermost level, with no runtime action.
    ErrorFamily(ErrorFamily),
    /// Take sole ownership of the TCP stream admitted by the native entry.
    TcpScope {
        /// Opaque lexical binding for the sole descriptor.
        stream: String,
        /// Structured executable body sharing the admitted stream.
        body: Vec<Op>,
    },
    /// Name the lexical stream borrowed for one synchronous hosted attempt.
    TcpIo {
        /// Live stream binding checked before the attempt.
        stream: String,
        /// A checked refill, send, or readiness wait using the existing range rules.
        operation: Box<Op>,
    },
    /// Consume the lexical stream and report the native release outcome once.
    TcpClose {
        /// The owner binding invalidated on every outcome.
        stream: String,
        /// Readonly scalar receiving zero or a tagged terminal errno.
        result: String,
    },
    /// Hosted entry synchronous external effect with a scoped positive-result borrow.
    Refill(Refill),
    /// Wait once for the sole admitted descriptor to become readable.
    WaitInput {
        /// Readonly scalar name receiving readiness, interruption, or tagged errno.
        result: String,
    },
    /// Attempt one send from an initialized byte field on the host descriptor.
    Send {
        /// Live reference to the record containing the initialized bytes.
        reference: String,
        /// Projection to the readonly send source.
        path: Vec<String>,
        /// Offset evaluated after capturing the source and before the count.
        offset: Expr,
        /// Positive count within the remaining field and the native count cap.
        count: Expr,
        /// Readonly scalar receiving the exact positive sent count or a status.
        result: String,
    },
    /// Wait once for the sole admitted stream descriptor to become writable.
    WaitOutput {
        /// Readonly scalar receiving readiness, interruption, or tagged errno.
        result: String,
    },
    /// End the current listener session and accept one next loopback client.
    NextSession {
        /// Readonly scalar receiving accepted, end, interruption, or errno.
        result: String,
    },
    /// Repeat a fresh entry scope until an explicit entry completion.
    Loop(Vec<Op>),
    /// Acquire backing once, expose it only on success, and free it once on exit.
    Region {
        /// Binding visible only in the successful body.
        name: String,
        /// Finite byte limit evaluated before acquisition.
        capacity: Expr,
        /// Compile-time test fault at the acquisition boundary; normally false.
        fail_acquire: bool,
        /// Lexical success scope.
        body: Vec<Op>,
        /// Lexical acquisition-failure scope; the region is not visible here.
        on_failure: Vec<Op>,
    },
    /// Own one region tail, restoring its marker when the batch scope ends.
    Batch {
        /// Lexical batch identity.
        name: String,
        /// Existing region binding.
        region: String,
        /// Schema of each published element.
        record: String,
        /// Scoped operations while this batch owns the tail.
        body: Vec<Op>,
    },
    /// Introduce a lexical scope for scalar/reference bindings.
    Scope(Vec<Op>),
    /// Declare and initialize a scalar binding.
    Let {
        /// New local name.
        name: String,
        /// Initial unsigned value.
        value: Expr,
    },
    /// Call a scalar result function, binding only its success. Failure exits
    /// the enclosing ordinary function in its own family, lifting a declared child.
    TryLet {
        /// Immutable success local, introduced after all actuals complete.
        name: String,
        /// Direct callee whose nominal result family matches or belongs to the caller's.
        function: String,
        /// Typed actuals evaluated once in their written order.
        args: Vec<FunctionArgument>,
    },
    /// Update an existing scalar without altering reference permissions.
    Set {
        /// Existing scalar local.
        name: String,
        /// Replacement unsigned value.
        value: Expr,
    },
    /// Attempt destination construction; publish only on success.
    Append {
        /// Receiving batch.
        batch: String,
        /// Constructor with the batch's result schema.
        producer: String,
        /// Typed scalar/byte arguments evaluated left to right.
        args: Vec<Argument>,
        /// New scalar status: 0 success, 1 capacity, 2 size overflow, 3 rejection.
        status: String,
    },
    /// Attempt typed destination construction, then evaluate one outcome arm.
    AppendHandled {
        /// Receiving batch.
        batch: String,
        /// Typed constructor with the batch's result schema.
        producer: String,
        /// Typed arguments evaluated once in written order.
        args: Vec<Argument>,
        /// New immutable scalar receiving the selected arm's value.
        result: String,
        /// Exhaustive outcome handling, after publication or rollback.
        arms: Vec<AppendArm>,
    },
    /// Construct one record in a free region tail and lend it to the success arm.
    ConstructHandled {
        /// Live bounded backing with no competing tail owner.
        region: String,
        /// Typed constructor whose schema defines the success reference.
        producer: String,
        /// Typed actuals evaluated once after the receiving tail is resolved.
        args: Vec<Argument>,
        /// Exhaustive lexical arms. Failure restores the tail before dispatch.
        arms: Vec<ConstructArm>,
    },
    /// Borrow an initialized element after a native bounds check.
    Borrow {
        /// New reference binding.
        name: String,
        /// Live owning batch.
        batch: String,
        /// Unsigned record index.
        index: Expr,
        /// Permission through this reference.
        access: Access,
    },
    /// Declare an alias without upgrading a readonly reference.
    Alias {
        /// New binding.
        name: String,
        /// Existing reference.
        source: String,
        /// Requested access permission.
        access: Access,
    },
    /// Rebind a reference only when all possible new roots outlive its binding.
    AssignRef {
        /// Existing destination reference.
        name: String,
        /// Existing source reference with compatible schema and permission.
        source: String,
    },
    /// Write a scalar field; compiler-managed shape metadata has no field path.
    Write {
        /// Writable reference binding.
        reference: String,
        /// Nested scalar projection.
        path: Vec<String>,
        /// Replacement value.
        value: Expr,
    },
    /// Change one existing payload byte without changing length or shape.
    WriteByte {
        /// Writable reference binding.
        reference: String,
        /// Nested byte field projection.
        path: Vec<String>,
        /// Bounds-checked unsigned byte index.
        index: Expr,
        /// Value explicitly narrowed to its low eight bits.
        value: Expr,
    },
    /// Reset a batch's marker/count; checker rejects any live reference to it.
    /// This compiler operation does not select a public Holo reset keyword.
    Reset {
        /// Live batch to reuse.
        batch: String,
    },
    /// Evaluate count once, then run a fresh lexical scope per iteration.
    Repeat {
        /// Unsigned iteration count.
        count: Expr,
        /// New unsigned loop-index binding in the body.
        index: String,
        /// Per-iteration scoped operations.
        body: Vec<Op>,
    },
    /// Traverse the published prefix once using frozen record-size hops.
    /// Count/end are captured on entry, so later appends are excluded from this pass.
    ForEach {
        /// Live batch whose existing records are visited in order.
        batch: String,
        /// Per-iteration checked record reference; its root remains the batch.
        reference: String,
        /// Permission through the cursor reference.
        access: Access,
        /// Fresh lexical body scope for each visited record.
        body: Vec<Op>,
    },
    /// Execute exactly one fresh branch scope.
    If {
        /// Unsigned condition with ordered operand evaluation.
        condition: Condition,
        /// Operations when the condition holds.
        then_body: Vec<Op>,
        /// Operations otherwise.
        else_body: Vec<Op>,
    },
    /// Terminate on false, independently of nonfatal property-reporting tools.
    Assert(Condition),
    /// Print one unsigned decimal value plus newline outside construction code.
    Print(Expr),
    /// Print one static UTF-8 literal plus newline from the entry function.
    /// The checker rejects interior NUL bytes before native emission.
    PrintText(String),
    /// Complete the current ordinary helper with one copied unsigned result.
    Return(Expr),
    /// Complete the current scalar-result helper with its checked outcome.
    ReturnResult(ResultValue),
    /// Complete the entry process successfully after lexical storage cleanup.
    ///
    /// This entry-only IR operation carries no returned reference or value. It
    /// does not establish a source-language rule for general function returns.
    ExitZero,
    /// Make existing backing contents opaque to optimization at a harness boundary.
    /// The native helper performs no algorithm, allocation, or actual memory mutation.
    OpaqueRegion(String),
}

impl Expr {
    /// Own two ordered children in a binary expression. Host allocation is a
    /// compiler cost, not a claim about the generated executable's allocations.
    pub fn binary(op: Binary, left: Self, right: Self) -> Self {
        Self::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }
}
