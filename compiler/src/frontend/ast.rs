//! Spanned syntax for the explicitly experimental source frontend.
//!
//! These owned nodes preserve source order and diagnostic locations before
//! lowering to the existing checked construction IR. Allocation belongs to the
//! compiler, never to generated records. Parsing a node does not make it valid.

use crate::source::Span;

/// One source module, without imports or implicit dependencies.
#[derive(Clone, Debug)]
pub struct Module {
    /// Logical record declarations in source order.
    pub records: Vec<RecordDecl>,
    /// Payload-free enum declarations in source order.
    pub enums: Vec<EnumDecl>,
    /// Nominal error families in source order.
    pub errors: Vec<ErrorDecl>,
    /// Function declarations in source order, including the entry point.
    pub functions: Vec<Function>,
}

/// A payload-free enum and its variants in declaration order.
#[derive(Clone, Debug)]
pub struct EnumDecl {
    /// Type name in the declaring namespace.
    pub name: String,
    /// Variant names with their source spans.
    pub variants: Vec<(String, Span)>,
    /// Full declaration range.
    pub span: Span,
}

/// An unresolved nominal error declaration; source identities precede any tag encoding.
#[derive(Clone, Debug)]
pub struct ErrorDecl {
    /// Family name in the declaring namespace.
    pub name: String,
    /// Local cases and child aliases in written order, before tag assignment.
    pub cases: Vec<ErrorCaseDecl>,
    /// Complete declaration range, including its visibility marker when written.
    pub span: Span,
}

/// One local error case or named child family before its types are checked.
#[derive(Clone, Debug)]
pub struct ErrorCaseDecl {
    /// Nominal case name or child alias within the enclosing family.
    pub name: String,
    /// Named field and written type, when the case carries a scalar.
    pub payload: Option<FieldDecl>,
    /// Child family path for `from`; namespace binding can replace it with a canonical name.
    pub source: Option<Expr>,
    /// Source range including any payload or child declaration.
    pub span: Span,
}

/// A named logical inline record declaration.
#[derive(Clone, Debug)]
pub struct RecordDecl {
    /// Schema name.
    pub name: String,
    /// Ordered fields; lowering must not reorder initializers.
    pub fields: Vec<FieldDecl>,
    /// Full declaration byte range.
    pub span: Span,
}

/// One field declaration before its type is resolved.
#[derive(Clone, Debug)]
pub struct FieldDecl {
    /// Field identity within the record.
    pub name: String,
    /// Identifier naming u64, bool, Bytes, or another declared record.
    pub ty: String,
    /// Source range for diagnostics.
    pub span: Span,
}

/// Preferred vertical function header and executable body.
#[derive(Clone, Debug)]
pub struct Function {
    /// Function name; main is the current entry point.
    pub name: String,
    /// Explicit input modes, never inferred from omitted text.
    pub args: Vec<Argument>,
    /// Attached contracts and their precise source ranges.
    pub guarantees: Vec<(String, Span)>,
    /// Header return type, distinct from a body return expression.
    pub result: String,
    /// Source extent of the result type, including brackets and type arguments when present.
    pub result_span: Span,
    /// Statements inside the body braces.
    pub body: Vec<Statement>,
    /// Full function range.
    pub span: Span,
}

/// An explicit argument declaration; unsupported modes remain diagnosable.
#[derive(Clone, Debug)]
pub struct Argument {
    /// Local parameter name.
    pub name: String,
    /// Declared type identifier or canonical `Batch<Record>` spelling.
    pub ty: String,
    /// Complete type span, including brackets and the qualified record if present.
    pub type_span: Span,
    /// Access spelling, currently readonly or readwrite.
    pub access: String,
    /// Passing spelling, currently copy or reference.
    pub passing: String,
    /// Full argument declaration range.
    pub span: Span,
}

/// One ordered statement with a stable source range.
#[derive(Clone, Debug)]
pub struct Statement {
    /// Syntactic statement form.
    pub kind: StatementKind,
    /// Byte range of the statement.
    pub span: Span,
}

/// Structured statements; no goto or hidden capture is introduced by parsing.
#[derive(Clone, Debug)]
pub enum StatementKind {
    /// Declare a lexical binding; an optional recovery body belongs to a fallible declaration.
    Let {
        /// New name.
        name: String,
        /// Whether source reassignment is permitted.
        mutable: bool,
        /// Whether the binding explicitly requests a writable byte view.
        writable_view: bool,
        /// Initializer evaluated once.
        value: Expr,
        /// Explicit acquisition recovery, when supported by the initializer.
        otherwise: Option<Vec<Statement>>,
    },
    /// Assign a scalar/reference binding or an existing projected field/byte.
    Assign {
        /// Destination resolved before its replacement value.
        target: Expr,
        /// Replacement expression.
        value: Expr,
    },
    /// Expression used for a recognized operation or discarded scalar value.
    Expr(Expr),
    /// Print fixed UTF-8 text from a literal with no runtime value.
    PrintText(String),
    /// Construct into explicit or compiler-selected backing and run one outcome arm.
    Construct {
        /// Written core call containing the constructor and any explicit region.
        subject: Expr,
        /// Outcome arms in source order.
        arms: Vec<ResultBlockArm>,
    },
    /// Call one ordinary fallible function and execute its selected outcome block.
    HandleResult {
        /// The written call, evaluated before any arm bindings exist.
        subject: Expr,
        /// Outcome arms in source order.
        arms: Vec<ResultBlockArm>,
    },
    /// Body return; supported roles are checked by lowering.
    Return(Expr),
    /// Explicit lexical lifetime boundary.
    Scope(Vec<Statement>),
    /// Bounded backing acquisition with separate success and recovery scopes.
    WithRegion {
        /// Written byte budget, evaluated in the enclosing scope.
        capacity: Expr,
        /// Statements executed with the acquired backing live.
        body: Vec<Statement>,
        /// Statements executed if acquisition fails before entering the body.
        on_failure: Vec<Statement>,
    },
    /// Lexical owner of the TCP stream admitted by the hosted entry adapter.
    TcpScope {
        /// Name usable only within the owner body.
        stream: String,
        /// Statements executed with the admitted stream.
        body: Vec<Statement>,
    },
    /// Hosted entry loop with a fresh lexical scope per iteration.
    Loop(Vec<Statement>),
    /// Exit the innermost loop.
    Break,
    /// Advance the innermost loop to its next iteration.
    Continue,
    /// Single hosted input attempt with disjoint outcome bindings.
    Refill {
        /// Existing readwrite byte field.
        slot: Expr,
        /// Positive maximum byte count.
        limit: Expr,
        /// Optional named TCP owner and its source range. The legacy refill has no owner operand.
        stream: Option<(String, Span)>,
        /// Local name for the readonly received extent.
        data_name: String,
        /// Body that may lend the received extent to ordinary helpers or producers.
        data_body: Vec<Statement>,
        /// Local name for the copied nondata status.
        status_name: String,
        /// Body for EOF, interruption, readiness retry, or native error.
        other_body: Vec<Statement>,
    },
    /// Exactly one conditionally selected branch.
    If {
        /// Boolean/comparison condition.
        condition: Expr,
        /// Selected statements when true.
        then_body: Vec<Statement>,
        /// Selected statements otherwise, possibly empty.
        else_body: Vec<Statement>,
    },
    /// Counted-range or batch iteration, resolved by lowering.
    For {
        /// Per-iteration local/cursor name.
        name: String,
        /// Optional explicit cursor access spelling.
        access: Option<String>,
        /// Range call or batch expression.
        iterable: Expr,
        /// Fresh lexical body per iteration.
        body: Vec<Statement>,
    },
}

/// One source expression before type/effect checking.
#[derive(Clone, Debug)]
pub struct Expr {
    /// Parsed expression form.
    pub kind: ExprKind,
    /// Source range including postfix operations.
    pub span: Span,
}

/// Source expressions supported by parsing; lowering rejects unavailable semantics.
#[derive(Clone, Debug)]
pub enum ExprKind {
    /// Exact unsigned integer literal.
    Number(u64),
    /// Readonly byte string with its decoded contents.
    Bytes(std::sync::Arc<[u8]>),
    /// Boolean literal.
    Bool(bool),
    /// Boolean negation with the original prefix location for formatting.
    Not {
        /// The contextual `not` word.
        operator: Span,
        /// Operand evaluated once.
        value: Box<Expr>,
    },
    /// Lexical or declared name.
    Name(String),
    /// A namespace-local enum name that can still be shadowed by a lexical binding.
    EnumRoot {
        /// Written name, used for lexical lookup before choosing the enum.
        name: String,
        /// Checked nominal name of the type.
        nominal: String,
    },
    /// Namespace-local family spelling in a bound error case path.
    ErrorFamilyRoot {
        /// Written root, which can also name an alias of the expected family.
        name: String,
        /// Checked nominal identity when used as an explicit family qualifier.
        nominal: String,
    },
    /// Wrap the evaluated operand as the declared success payload of a return.
    Ok(Box<Expr>),
    /// Return a named case from a nominal error family.
    Error {
        /// Qualified case name, resolved against the function's error family.
        case: Box<Expr>,
        /// Optional operand evaluated on the error return path only.
        payload: Option<Box<Expr>>,
    },
    /// Early typed propagation in a binding or nested destination initializer.
    Try(Box<Expr>),
    /// Dispatch one evaluated result to one typed outcome arm.
    Handle {
        /// Result-valued expression evaluated exactly once by checked lowering.
        subject: Box<Expr>,
        /// Indented arms in written order, including any final fallback.
        arms: Vec<ResultArm>,
    },
    /// Ordinary call syntax, including experimental core operations.
    Call {
        /// Function/member expression naming the callee.
        callee: Box<Expr>,
        /// Ordered arguments.
        args: Vec<Expr>,
    },
    /// Named field projection or namespace member.
    Member {
        /// Projected expression.
        base: Box<Expr>,
        /// Field/member name.
        field: String,
    },
    /// Indexed byte projection; not an implicit unchecked pointer operation.
    Index {
        /// Payload expression.
        base: Box<Expr>,
        /// Unsigned index expression.
        index: Box<Expr>,
    },
    /// Fresh named record, constructed only in a supported destination context.
    Record {
        /// Logical record name.
        name: String,
        /// Initializers in their written order.
        fields: Vec<FieldInit>,
        /// Whether a colon opened an indentation-delimited destination initializer.
        indented: bool,
    },
    /// Explicit infix syntax, including operators whose numeric defaults remain unavailable.
    Binary {
        /// Parsed operator.
        op: BinaryOp,
        /// Written operator token span.
        operator: Span,
        /// First operand.
        left: Box<Expr>,
        /// Second operand.
        right: Box<Expr>,
    },
}

/// One `on` clause with its unresolved pattern and result expression.
#[derive(Clone, Debug)]
pub struct ResultArm {
    /// Active alternative selected by the arm.
    pub pattern: ResultPattern,
    /// Expression evaluated only when this alternative is selected.
    pub value: Expr,
    /// Source range from `on` through the arm expression.
    pub span: Span,
}

/// One statement handler outcome and its lexical block.
#[derive(Clone, Debug)]
pub struct ResultBlockArm {
    /// Active alternative selected by the arm.
    pub pattern: ResultPattern,
    /// Statements that run only on this alternative.
    pub body: Vec<Statement>,
    /// Source range from `on` through the closing brace.
    pub span: Span,
}

/// Syntactic pattern categories whose types and coverage are checked after binding.
#[derive(Clone, Debug)]
pub enum ResultPattern {
    /// Successful result, optionally binding its payload; `_` explicitly ignores it.
    Ok(Option<String>),
    /// Successful scoped construction with an explicitly writable record binding.
    ReadwriteOk(String),
    /// A named error case, with optional qualification and payload binding.
    Error {
        /// Written case path.
        case: Expr,
        /// Local payload name, or `_` for an explicit discard.
        binding: Option<String>,
    },
    /// A constructor error nested under the append result's rejection alternative.
    Rejected {
        /// Written constructor error case path.
        case: Expr,
        /// Local payload name, or `_` for an explicit discard.
        binding: Option<String>,
    },
    /// Every uncovered error, optionally binding the original typed reason.
    Fallback(Option<String>),
}

/// A named initializer retaining source order and its own diagnostic range.
#[derive(Clone, Debug)]
pub struct FieldInit {
    /// Written field name.
    pub name: String,
    /// Initializer expression.
    pub value: Expr,
    /// Full initializer range.
    pub span: Span,
}

/// Parsed infix operators; lowering selects the checked operation for each operand type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    /// Short-circuit boolean disjunction.
    LogicalOr,
    /// Short-circuit boolean conjunction.
    LogicalAnd,
    /// Equality.
    Eq,
    /// Inequality.
    Ne,
    /// Unsigned less-than.
    Lt,
    /// Unsigned less-than-or-equal.
    Le,
    /// Unsigned greater-than.
    Gt,
    /// Unsigned greater-than-or-equal.
    Ge,
    /// Addition.
    Add,
    /// Subtraction.
    Sub,
    /// Multiplication.
    Mul,
    /// Division.
    Div,
    /// Remainder.
    Rem,
    /// Bitwise conjunction.
    And,
    /// Bitwise exclusive-or.
    Xor,
}
