//! Opt-in local library and package binding before the shared checked frontend.
//!
//! The entry can call functions declared public in trusted leaf files, and
//! package entries can select public inline schemas in batch declarations.
//! Error unions bind child sources against the declaring file's visible names;
//! case paths retain local spelling for subject-relative matching.
//! An explicit namespace must match the import that resolved the file.
//! Compiler-only identities isolate private calls and never become a runtime
//! namespace table. Each source owns a distinct virtual span interval so the
//! same lowering and checker can report the original path and line.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::{SourceProgram, ast, functions::SCALAR_OPTION, lower_module, parse};
use crate::source::{Diagnostic, Span};

#[path = "modules/packaged.rs"]
mod packaged;

pub use packaged::{
    check_experimental_library_package, check_experimental_library_package_with_naming,
    parse_experimental_package,
};

/// Prefix outside the source identifier grammar, used only in compiler maps.
const PRIVATE_PREFIX: &str = "$holo.";
/// Distinct compiler-only prefix for resolved package-qualified functions.
const PACKAGE_PREFIX: &str = "$package.";

/// Checked-lowering input and original sources for its diagnostic spans.
pub struct SourceProject<'a> {
    /// The same source representation as a standalone checked program.
    program: SourceProgram,
    /// Ordered file intervals, each separated by one virtual byte.
    sources: Vec<SourceFile<'a>>,
}

/// One immutable source file in the project diagnostic map.
struct SourceFile<'a> {
    /// Path used when reporting syntax and semantic failures.
    path: PathBuf,
    /// Original bytes, including comments and public declarations.
    text: Cow<'a, str>,
    /// Inclusive start of this file's virtual span interval.
    offset: usize,
}

impl SourceProject<'_> {
    /// Borrow the shared lowered program. The caller still invokes its checker
    /// before generating any native code.
    pub fn program(&self) -> &SourceProgram {
        &self.program
    }

    /// Render a lowering or checker error against the original owning source.
    /// Only diagnostic formatting allocates here.
    pub fn render(&self, error: &Diagnostic) -> String {
        render_lowered(&self.sources, error)
    }

    /// Render a naming advisory against its original project source.
    /// Naming messages already use original spellings. Formatting owns its output.
    pub fn render_warning(&self, warning: &Diagnostic) -> String {
        render_in_source(&self.sources, warning, Diagnostic::render_warning)
    }
}

/// Resolve one entry and its explicit `holo.<leaf>` imports under a trusted
/// project-selected directory. The map borrows entry text and owns imported
/// text. No file is executed, and returned syntax still requires checking.
pub fn parse_experimental_project<'a>(
    entry: &Path,
    text: &'a str,
    library_root: &Path,
) -> Result<SourceProject<'a>, String> {
    let mut sources = vec![SourceFile {
        path: entry.to_owned(),
        text: Cow::Borrowed(text),
        offset: 0,
    }];
    let (mut module, imports, naming_diagnostics) =
        parse::project_entry_with_naming(&sources[0].text, true)
            .map_err(|error| render(&sources, &error))?;
    let mut seen = HashSet::new();
    // Reject duplicates before opening dependencies, even when the files are absent.
    for import in &imports {
        if !seen.insert(import.leaf.as_str()) {
            return Err(render(
                &sources,
                &Diagnostic::new(
                    import.span,
                    format!("duplicate `using` directive for `holo.{}`", import.leaf),
                ),
            ));
        }
    }
    let mut visible = Vec::new();
    let mut functions = Vec::new();
    // The parser admits one validated path component per import. Libraries
    // cannot import further files, so this loop bounds all dependency reads.
    for import in imports {
        let path = library_root.join(format!("{}.holo", import.leaf));
        let text = read_library(&path)
            .map_err(|message| render(&sources, &Diagnostic::new(import.span, message)))?;
        let offset = append_source(&mut sources, path, Cow::Owned(text), import.span)
            .map_err(|error| render(&sources, &error))?;
        let file = sources.last().expect("library source was added");
        // The explicit library root is a dependency input. Collect advisories
        // only in the selected entry, even if strict policy is enabled.
        let (library, _) = parse::library_with_naming(&file.text, offset, false)
            .map_err(|error| render(&sources, &error))?;
        // The selected import supplies the identity. A conflicting declaration
        // must fail before any of the library's names enter the checked program.
        if let Some(namespace) = &library.namespace
            && namespace.leaf != import.leaf
        {
            return Err(render(
                &sources,
                &Diagnostic::new(
                    namespace.span,
                    format!(
                        "namespace `holo.{}` does not match `using` directive for `holo.{}`",
                        namespace.leaf, import.leaf
                    ),
                ),
            ));
        }
        let (mut imported, public) =
            bind_library(&import.leaf, library.module, library.public_names)
                .map_err(|error| render(&sources, &error))?;
        let qualified = import.qualified();
        visible.push((
            import.leaf,
            Interface {
                qualified,
                public,
                public_records: RecordNames::new(),
                public_enums: RecordNames::new(),
                public_errors: ErrorNames::new(),
            },
        ));
        functions.append(&mut imported);
    }
    // Entry and imported functions keep distinct lexical environments even
    // though they share one checked program and one native compilation unit.
    let imports = visible
        .iter()
        .map(|(qualifier, interface)| (qualifier.as_str(), interface))
        .collect();
    bind_entry(&mut module, &imports, false).map_err(|error| render(&sources, &error))?;
    module.functions.append(&mut functions);
    let program = lower_module(&module, naming_diagnostics)
        .map_err(|error| render_lowered(&sources, &error))?;
    Ok(SourceProject { program, sources })
}

/// Resolve an entry file against its own imports before joining declarations.
/// Names of the entry's own helpers stay unchanged for the checked frontend.
/// Only manifest packages expose imported records as batch schema operands or parameter types.
fn bind_entry(
    module: &mut ast::Module,
    imports: &Imports<'_>,
    package_types: bool,
) -> Result<(), Diagnostic> {
    let scope = Scope::Entry {
        imports,
        package_types,
    };
    // Record names share the file with import qualifiers and must be unambiguous.
    for record in &module.records {
        scope.binding(&record.name, record.span)?;
    }
    for declaration in &module.enums {
        scope.binding(&declaration.name, declaration.span)?;
    }
    // Error names occupy the same source type position as records.
    for error in &mut module.errors {
        scope.binding(&error.name, error.span)?;
        scope.error_sources(error, None)?;
    }
    // Bodies and signatures are bound before appending any imported functions.
    for function in &mut module.functions {
        scope.binding(&function.name, function.span)?;
        scope.function(function)?;
    }
    Ok(())
}

/// Reserve a disjoint virtual span interval and append owned or borrowed text.
/// A separating byte distinguishes EOF in one file from the start of the next.
/// Failure is attributed to the requesting import without changing the map.
fn append_source<'a>(
    sources: &mut Vec<SourceFile<'a>>,
    path: PathBuf,
    text: Cow<'a, str>,
    import_span: Span,
) -> Result<usize, Diagnostic> {
    let offset = sources
        .last()
        .map_or(Some(0), |last| {
            last.offset
                .checked_add(last.text.len())
                .and_then(|end| end.checked_add(1))
        })
        .filter(|offset| offset.checked_add(text.len()).is_some())
        .ok_or_else(|| {
            Diagnostic::new(import_span, "project source spans exceed the host range")
        })?;
    sources.push(SourceFile { path, text, offset });
    Ok(offset)
}

/// Read an ordinary UTF-8 file from the trusted library tree. A direct leaf
/// may not be a symlink, directory, or special file. The root and its parent
/// paths are build inputs owned by the caller and must stay stable during
/// compilation.
fn read_library(path: &Path) -> Result<String, String> {
    let description = || format!("read local library {}", path.display());
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", description()))?;
    // Inspect the leaf itself, rather than following its type through a link.
    if !metadata.is_file() {
        return Err(format!(
            "{}: expected a regular file, excluding symbolic links",
            description()
        ));
    }
    fs::read_to_string(path).map_err(|error| format!("{}: {error}", description()))
}

/// Render an error within its original file without changing its owner.
fn render(sources: &[SourceFile<'_>], error: &Diagnostic) -> String {
    render_in_source(sources, error, Diagnostic::render)
}

/// Rebase a diagnostic into its owning file and format it with the given severity
/// renderer. Source bytes remain borrowed and the rendered excerpt is owned.
fn render_in_source(
    sources: &[SourceFile<'_>],
    diagnostic: &Diagnostic,
    render_diagnostic: impl FnOnce(&Diagnostic, &str, &str) -> String,
) -> String {
    let file = source_at(sources, diagnostic.span.start);
    let span = Span {
        start: diagnostic.span.start.saturating_sub(file.offset),
        end: diagnostic.span.end.saturating_sub(file.offset),
    };
    render_diagnostic(
        &Diagnostic::new(span, &diagnostic.message),
        &file.path.display().to_string(),
        &file.text,
    )
}

/// Borrow the original file for a virtual byte position using its sorted spans.
/// Callers provide positions produced by this map's parser or checker.
fn source_at<'s, 'a>(sources: &'s [SourceFile<'a>], position: usize) -> &'s SourceFile<'a> {
    let index = sources.partition_point(|file| file.offset <= position);
    &sources[index.saturating_sub(1)]
}

/// Render lowered diagnostics using source-facing names. Syntax and file errors
/// use `render` directly so path spellings never undergo an identity rewrite.
fn render_lowered(sources: &[SourceFile<'_>], error: &Diagnostic) -> String {
    let message = error
        .message
        .replace(PRIVATE_PREFIX, "holo.")
        .replace(PACKAGE_PREFIX, "");
    render(sources, &Diagnostic::new(error.span, message))
}

/// Map source function names to private compiler identities.
type Members = HashMap<String, String>;

/// Map bare record names to nominal schema names in the checked program.
type RecordNames = HashMap<String, String>;

/// Map bare error family names to nominal names in the checked program.
type ErrorNames = HashMap<String, String>;

/// Source interface exposed by a namespace selected by the loader.
struct Interface {
    /// Complete source-facing namespace for diagnostics.
    qualified: String,
    /// Only functions explicitly declared public enter a qualified lookup.
    public: Members,
    /// Public records visible in a package application's batch declarations.
    public_records: RecordNames,
    /// Public enum types and the nominal identities of their variants.
    public_enums: RecordNames,
    /// Public error families available to qualified patterns and union declarations.
    public_errors: ErrorNames,
}

/// A file's short qualifiers mapped to stable, already bound interfaces.
type Imports<'a> = HashMap<&'a str, &'a Interface>;

/// Check the boundary shared by legacy leaves and package namespaces. The
/// common frontend checks argument permissions and contracts after binding.
fn check_library_function(
    function: &ast::Function,
    records: Option<&RecordNames>,
    errors: Option<&ErrorNames>,
) -> Result<(), Diagnostic> {
    // A package producer's success and error types must each resolve in its
    // own namespace. A scalar option does not name a namespace-owned type.
    let (success, family) = super::functions::result_parts(&function.result)
        .map_or((function.result.as_str(), None), |(success, family)| {
            (success, Some(family))
        });
    let supported_result = (matches!(success, "u64" | SCALAR_OPTION)
        || (success == "Unit" && family.is_some())
        || (success == "bool" && family.is_none())
        || records.is_some_and(|records| records.contains_key(success)))
        && family.is_none_or(|family| errors.is_some_and(|errors| errors.contains_key(family)));
    if function.name == "main"
        || !supported_result
        || function.args.iter().any(|argument| {
            let ty = argument.ty.as_str();
            !(matches!(ty, "u64" | "bool" | "Bytes")
                || records.is_some_and(|records| {
                    super::functions::batch_record(ty).is_some()
                        || (argument.access == "readwrite"
                            && argument.passing == "reference"
                            && (records.contains_key(ty) || ty.contains('.')))
                }))
        })
    {
        let requirement = if records.is_some() {
            "a package library requires non-entry functions with u64, bool, Option<u64>, a namespace-local struct, or Result with u64, Unit, or record success and a namespace-local error family, and u64, bool, Bytes, Batch<Record>, or writable fixed record state arguments"
        } else {
            "a leaf library requires non-entry functions with u64, bool, or Option<u64> results and u64, bool, or Bytes arguments"
        };
        return Err(Diagnostic::new(function.span, requirement));
    }
    Ok(())
}

/// Bind all local calls in an imported module and return its functions and
/// public interface. Scalar results and borrowed byte arguments cross this slice.
fn bind_library(
    leaf: &str,
    mut library: ast::Module,
    public_names: Vec<String>,
) -> Result<(Vec<ast::Function>, Members), Diagnostic> {
    // The legacy leaf profile has no nominal type export table.
    if let Some(declaration) = library.errors.first() {
        return Err(Diagnostic::new(
            declaration.span,
            "error families in libraries require a manifest package",
        ));
    }
    let mut local = Members::new();
    // Index definitions first so forward calls and recursion use the same
    // checker as single-file helpers. Imported names cannot be written as
    // source identifiers.
    for function in &library.functions {
        check_library_function(function, None, None)?;
        let identity = format!("{PRIVATE_PREFIX}{leaf}.{}", function.name);
        if local.insert(function.name.clone(), identity).is_some() {
            return Err(Diagnostic::new(
                function.span,
                format!("duplicate library function `{}`", function.name),
            ));
        }
    }
    let scope = Scope::Library {
        local: &local,
        imports: None,
        records: None,
        enums: None,
        errors: None,
    };
    // Rewrite references before replacing declaration identities.
    for function in &mut library.functions {
        scope.function(function)?;
        function.name = local[&function.name].clone();
    }
    let public = public_names
        .into_iter()
        .map(|name| {
            let identity = local[&name].clone();
            (name, identity)
        })
        .collect();
    Ok((library.functions, public))
}

/// Lexical lookup boundary for one entry or library source file.
enum Scope<'a> {
    /// Imported namespaces accessible only through their visible members.
    Entry {
        /// Qualified namespaces imported by the entry file.
        imports: &'a Imports<'a>,
        /// Whether the entry can select a public record by qualifier.
        package_types: bool,
    },
    /// Local namespace definitions and file-specific qualified imports.
    Library {
        /// All functions visible by bare name inside the namespace.
        local: &'a Members,
        /// Qualified interfaces, absent for an isolated legacy leaf.
        imports: Option<&'a Imports<'a>>,
        /// Namespace-local schemas, absent for an isolated legacy leaf.
        records: Option<&'a RecordNames>,
        /// Namespace-local enum types, absent for an isolated legacy leaf.
        enums: Option<&'a RecordNames>,
        /// Namespace-local error families, absent for isolated legacy leaves.
        errors: Option<&'a ErrorNames>,
    },
}

impl Scope<'_> {
    /// Borrow this file's import scope, when its loading mode supports imports.
    fn imports(&self) -> Option<&Imports<'_>> {
        match self {
            Self::Entry { imports, .. } => Some(imports),
            Self::Library { imports, .. } => *imports,
        }
    }

    /// Reject any binding that would make an imported qualifier ambiguous.
    fn binding(&self, name: &str, span: Span) -> Result<(), Diagnostic> {
        // The check applies in each lexical scope of this file only.
        if self
            .imports()
            .is_some_and(|imports| imports.contains_key(name))
        {
            return Err(Diagnostic::new(
                span,
                format!("`{name}` shadows an imported namespace"),
            ));
        }
        Ok(())
    }

    /// Bind a function's body and record parameter schemas without altering source ranges.
    fn function(&self, function: &mut ast::Function) -> Result<(), Diagnostic> {
        // Qualifiers remain unambiguous for the entire checked entry file.
        for arg in &mut function.args {
            self.binding(&arg.name, arg.span)?;
            self.record_parameter(arg)?;
        }
        self.statements(&mut function.body, function.result == SCALAR_OPTION)
    }

    /// Give batch elements and fixed state the same identity as their backing record.
    fn record_parameter(&self, argument: &mut ast::Argument) -> Result<(), Diagnostic> {
        let batch = super::functions::batch_record(&argument.ty);
        let record = batch.unwrap_or(&argument.ty);
        if batch.is_none() && matches!(record, "u64" | "bool" | "Bytes") {
            return Ok(());
        }
        let identity = if let Some((qualifier, member)) = record.split_once('.') {
            let imports = match self {
                Self::Entry {
                    imports,
                    package_types: true,
                } => Some(*imports),
                Self::Library {
                    imports,
                    records: Some(_),
                    ..
                } => *imports,
                _ => None,
            };
            let interface = imports.and_then(|imports| imports.get(qualifier)).ok_or_else(|| {
                Diagnostic::new(
                    argument.type_span,
                    format!("no package `using` directive for namespace qualifier `{qualifier}` in this file"),
                )
            })?;
            interface.public_records.get(member).ok_or_else(|| {
                Diagnostic::new(
                    argument.type_span,
                    format!(
                        "library `{}` has no public struct `{member}`",
                        interface.qualified
                    ),
                )
            })?
        } else if let Self::Library {
            records: Some(records),
            ..
        } = self
        {
            let Some(identity) = records.get(record) else {
                let kind = if batch.is_some() {
                    "batch record"
                } else {
                    "state record"
                };
                return Err(Diagnostic::new(
                    argument.type_span,
                    format!("unknown {kind} schema `{record}`"),
                ));
            };
            identity
        } else {
            // Unqualified entry schemas are already named by their written identity.
            return Ok(());
        };
        argument.ty = if batch.is_some() {
            format!("Batch<{identity}>")
        } else {
            identity.clone()
        };
        Ok(())
    }

    /// Resolve calls throughout a block, retaining every lexical boundary.
    fn statements(
        &self,
        body: &mut [ast::Statement],
        option_return: bool,
    ) -> Result<(), Diagnostic> {
        use ast::StatementKind as Kind;
        // The existing parser bounds nesting before these recursive visits.
        for statement in body {
            match &mut statement.kind {
                Kind::Let {
                    name,
                    value,
                    otherwise,
                    ..
                } => {
                    self.binding(name, statement.span)?;
                    self.batch_schema(value)?;
                    self.expression(value)?;
                    // A recovery branch sees the same module environment.
                    if let Some(otherwise) = otherwise {
                        self.statements(otherwise, option_return)?;
                    }
                }
                Kind::Assign { target, value } => {
                    self.expression(target)?;
                    self.expression(value)?;
                }
                Kind::Expr(expr) => self.expression(expr)?,
                Kind::Return(expr) => self.return_expression(expr, option_return)?,
                // Literal output and loop transfers contain no names to resolve.
                Kind::PrintText(_) | Kind::Break | Kind::Continue => {}
                Kind::Construct { subject, arms } | Kind::HandleResult { subject, arms } => {
                    self.expression(subject)?;
                    // The constructor and case qualifiers use this file's imports.
                    // Each arm's introduced name must leave those imports unambiguous.
                    for arm in arms {
                        self.result_pattern(&mut arm.pattern, arm.span)?;
                        self.statements(&mut arm.body, option_return)?;
                    }
                }
                Kind::WithRegion {
                    capacity,
                    body,
                    on_failure,
                } => {
                    // The budget and both lexical paths use this file's imports.
                    // The private backing has no source name to bind here.
                    self.expression(capacity)?;
                    self.statements(body, option_return)?;
                    self.statements(on_failure, option_return)?;
                }
                Kind::TcpScope { stream, body } => {
                    // The lexical owner follows the same qualifier rule as
                    // the other entry bindings and only introduces a name here.
                    self.binding(stream, statement.span)?;
                    self.statements(body, option_return)?;
                }
                Kind::Scope(body) | Kind::Loop(body) => self.statements(body, option_return)?,
                Kind::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    self.expression(condition)?;
                    self.statements(then_body, option_return)?;
                    self.statements(else_body, option_return)?;
                }
                Kind::For {
                    name,
                    iterable,
                    body,
                    ..
                } => {
                    self.binding(name, statement.span)?;
                    self.expression(iterable)?;
                    self.statements(body, option_return)?;
                }
                Kind::Refill {
                    slot,
                    limit,
                    data_name,
                    data_body,
                    status_name,
                    other_body,
                    ..
                } => {
                    // Both outcomes introduce an arm-local binding. They must
                    // leave the module's qualifier unambiguous inside each arm.
                    self.binding(data_name, statement.span)?;
                    self.binding(status_name, statement.span)?;
                    self.expression(slot)?;
                    self.expression(limit)?;
                    self.statements(data_body, option_return)?;
                    self.statements(other_body, option_return)?;
                }
            }
        }
        Ok(())
    }

    /// Resolve a public type only where a package entry's batch declaration
    /// expects a schema. Other calls and malformed operands remain for the
    /// usual lowering diagnostics. The AST changes only a compile-time name.
    fn batch_schema(&self, value: &mut ast::Expr) -> Result<(), Diagnostic> {
        use ast::ExprKind as Kind;
        // The legacy loader and library bodies keep their existing operand rules.
        let Self::Entry {
            imports,
            package_types: true,
        } = self
        else {
            return Ok(());
        };
        // Recognize exactly the builtin initializer with its normal two arguments.
        let Kind::Call { callee, args } = &mut value.kind else {
            return Ok(());
        };
        if !matches!(&callee.kind, Kind::Member { base, field }
            if field == "batch" && matches!(&base.kind, Kind::Name(name) if name == "core"))
        {
            return Ok(());
        }
        let [_, schema] = args.as_mut_slice() else {
            return Ok(());
        };
        // Unqualified record names remain ordinary entry schemas.
        let Kind::Member { base, field } = &schema.kind else {
            return Ok(());
        };
        let Kind::Name(qualifier) = &base.kind else {
            return Ok(());
        };
        let interface = imports.get(qualifier.as_str()).ok_or_else(|| {
            Diagnostic::new(
                schema.span,
                format!("no `using` directive for namespace qualifier `{qualifier}` in this file"),
            )
        })?;
        let record = interface.public_records.get(field).ok_or_else(|| {
            Diagnostic::new(
                schema.span,
                format!(
                    "library `{}` has no public struct `{field}`",
                    interface.qualified
                ),
            )
        })?;
        schema.kind = Kind::Name(record.clone());
        Ok(())
    }

    /// Preserve the contextual Some constructor while resolving its payload expressions.
    /// A function called Some follows ordinary lookup outside this return position.
    fn return_expression(
        &self,
        value: &mut ast::Expr,
        option_return: bool,
    ) -> Result<(), Diagnostic> {
        use ast::ExprKind as Kind;
        if option_return
            && let Kind::Call { callee, args } = &mut value.kind
            && matches!(&callee.kind, Kind::Name(name) if name == "Some")
        {
            for argument in args {
                self.expression(argument)?;
            }
            return Ok(());
        }
        self.expression(value)
    }

    /// Resolve call targets and nested actuals without changing evaluation order.
    fn expression(&self, expression: &mut ast::Expr) -> Result<(), Diagnostic> {
        use ast::ExprKind as Kind;
        match &mut expression.kind {
            Kind::Call { callee, args } => {
                self.callee(callee)?;
                // Actual syntax retains its written evaluation order.
                for argument in args {
                    self.expression(argument)?;
                }
            }
            Kind::Member { base, .. } => {
                self.enum_root(base)?;
                self.expression(base)?;
            }
            Kind::Index { base, index } => {
                self.expression(base)?;
                self.expression(index)?;
            }
            Kind::Record { name, fields, .. } => {
                // Package literals name only their own namespace's schemas.
                // Legacy libraries cannot construct records at all.
                match self {
                    Self::Library {
                        records: Some(records),
                        ..
                    } => {
                        *name = records
                            .get(name)
                            .ok_or_else(|| {
                                Diagnostic::new(
                                    expression.span,
                                    format!("unknown namespace-local struct `{name}`"),
                                )
                            })?
                            .clone();
                    }
                    Self::Library { .. } => {
                        return Err(Diagnostic::new(
                            expression.span,
                            "leaf libraries cannot construct records",
                        ));
                    }
                    Self::Entry { .. } => {}
                }
                // Initializer order belongs to the existing destination lowering.
                for field in fields {
                    self.expression(&mut field.value)?;
                }
            }
            Kind::Binary { left, right, .. } => {
                self.expression(left)?;
                self.expression(right)?;
            }
            Kind::Not { value, .. } | Kind::Ok(value) | Kind::Try(value) => {
                self.expression(value)?;
            }
            Kind::Error { case, payload } => {
                self.error_case(case);
                // Resolve ordinary calls in the payload without changing the case identity.
                if let Some(payload) = payload {
                    self.expression(payload)?;
                }
            }
            Kind::Handle { subject, arms } => {
                self.expression(subject)?;
                // Qualify names without evaluating the subject or any arm.
                for arm in arms {
                    self.result_pattern(&mut arm.pattern, arm.span)?;
                    self.expression(&mut arm.value)?;
                }
            }
            Kind::Name(_)
            | Kind::ErrorFamilyRoot { .. }
            | Kind::EnumRoot { .. }
            | Kind::Number(_)
            | Kind::Bool(_)
            | Kind::Bytes(_) => {}
        }
        Ok(())
    }

    /// Bind a variant's type root while preserving the written spelling for shadowing.
    fn enum_root(&self, base: &mut ast::Expr) -> Result<(), Diagnostic> {
        use ast::ExprKind as Kind;
        let resolved = match &base.kind {
            Kind::Name(name) => match self {
                Self::Library {
                    enums: Some(enums), ..
                } => enums
                    .get(name)
                    .map(|nominal| (name.clone(), nominal.clone())),
                _ => None,
            },
            Kind::Member { base, field } => {
                let Kind::Name(qualifier) = &base.kind else {
                    return Ok(());
                };
                let Some(interface) = self
                    .imports()
                    .and_then(|imports| imports.get(qualifier.as_str()))
                else {
                    return Ok(());
                };
                let nominal = interface.public_enums.get(field).ok_or_else(|| {
                    Diagnostic::new(
                        base.span,
                        format!(
                            "library `{}` has no public enum `{field}`",
                            interface.qualified
                        ),
                    )
                })?;
                // Import qualifier names cannot be shadowed. The qualified
                // name is therefore distinct from any lexical identifier.
                Some((format!("{qualifier}.{field}"), nominal.clone()))
            }
            _ => None,
        };
        if let Some((name, nominal)) = resolved {
            base.kind = Kind::EnumRoot { name, nominal };
        }
        Ok(())
    }

    /// Rewrite a result case's explicit qualifier and check its optional local name.
    /// Case identity and payload compatibility are checked against the subject later.
    fn result_pattern(
        &self,
        pattern: &mut ast::ResultPattern,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let binding = match pattern {
            ast::ResultPattern::Error { case, binding }
            | ast::ResultPattern::Rejected { case, binding } => {
                self.error_case(case);
                binding
            }
            ast::ResultPattern::Ok(binding) | ast::ResultPattern::Fallback(binding) => binding,
            ast::ResultPattern::ReadwriteOk(binding) => return self.binding(binding, span),
        };
        // An introduced binding cannot shadow a namespace visible in the arm.
        if let Some(binding) = binding {
            self.binding(binding, span)?;
        }
        Ok(())
    }

    /// Rewrite this declaration's child sources in its file's scope. Exported
    /// declarations can reference only public children. Bare entry names already
    /// carry their final spelling. No case payloads or native tags are resolved here.
    fn error_sources(
        &self,
        declaration: &mut ast::ErrorDecl,
        public: Option<&ErrorNames>,
    ) -> Result<(), Diagnostic> {
        use ast::ExprKind as Kind;

        // Local cases do not need any type-name lookup during package binding.
        for source in declaration
            .cases
            .iter_mut()
            .filter_map(|case| case.source.as_mut())
        {
            let identity = match &source.kind {
                Kind::Name(name) => {
                    // Only namespace-owned declarations have canonical local names.
                    let Self::Library {
                        errors: Some(errors),
                        ..
                    } = self
                    else {
                        continue;
                    };
                    let identity = errors.get(name).ok_or_else(|| {
                        Diagnostic::new(
                            source.span,
                            format!("unknown namespace-local error family `{name}`"),
                        )
                    })?;
                    // A public parent exposes its children in callers' case paths.
                    if public.is_some_and(|visible| !visible.contains_key(name)) {
                        return Err(Diagnostic::new(
                            source.span,
                            format!(
                                "public error family `{}` cannot expose private child error family `{name}`",
                                declaration.name
                            ),
                        ));
                    }
                    identity
                }
                Kind::Member { base, field } => {
                    // Qualified source types use exactly one file-local import qualifier.
                    let Kind::Name(qualifier) = &base.kind else {
                        return Err(Diagnostic::new(
                            source.span,
                            "expected `Family` or `namespace.Family` after `from`",
                        ));
                    };
                    let interface = self.imports()
                        .and_then(|imports| imports.get(qualifier.as_str()))
                        .ok_or_else(|| Diagnostic::new(
                            base.span,
                            format!("no `using` directive for namespace qualifier `{qualifier}` in this file"),
                        ))?;
                    interface.public_errors.get(field).ok_or_else(|| {
                        Diagnostic::new(
                            source.span,
                            format!(
                                "library `{}` has no public error family `{field}`",
                                interface.qualified
                            ),
                        )
                    })?
                }
                _ => unreachable!("the parser supplies only family paths"),
            };
            source.kind = Kind::Name(identity.clone());
        }
        Ok(())
    }

    /// Rewrite an explicitly named family root to its nominal identity.
    /// Bare cases and child paths resolve against the result subject later.
    /// The parser bounds path depth before this walk and rewrites only prefixes.
    fn error_case(&self, case: &mut ast::Expr) {
        use ast::ExprKind as Kind;
        // A bare case has no namespace lookup or visibility of its own.
        let Kind::Member { base, .. } = &mut case.kind else {
            return;
        };
        // Walk to the root first so a longer path can retain its child suffix.
        self.error_case(base);
        let replacement = match &base.kind {
            // A bare family name can still be a relative child alias. Retain
            // its spelling so the checked subject can disambiguate it.
            Kind::Name(name) => match self {
                Self::Library {
                    errors: Some(errors),
                    ..
                } => errors.get(name).map(|nominal| Kind::ErrorFamilyRoot {
                    name: name.clone(),
                    nominal: nominal.clone(),
                }),
                _ => None,
            },
            // A qualifier and public family form a root together. Leave other
            // paths intact since they may use aliases from the subject's union.
            Kind::Member {
                base: qualifier,
                field,
            } => match &qualifier.kind {
                Kind::Name(qualifier) => self
                    .imports()
                    .and_then(|imports| imports.get(qualifier.as_str()))
                    .and_then(|interface| interface.public_errors.get(field))
                    .map(|nominal| Kind::Name(nominal.clone())),
                _ => None,
            },
            // A shorter path already resolved this root.
            Kind::ErrorFamilyRoot { .. } => None,
            _ => unreachable!("the parser supplies only case paths"),
        };
        // Preserve source spans and suffix components for payload diagnostics.
        if let Some(kind) = replacement {
            base.kind = kind;
        }
    }

    /// Lower an allowed module call to an ordinary named source call. Unknown
    /// entry calls keep their existing diagnostics. A library can use its own
    /// namespace by bare name and only the interfaces it imports by qualifier.
    fn callee(&self, callee: &mut ast::Expr) -> Result<(), Diagnostic> {
        use ast::ExprKind as Kind;
        let identity = match (self, &callee.kind) {
            (Self::Library { local, .. }, Kind::Name(name)) => {
                Some(local.get(name).ok_or_else(|| {
                    Diagnostic::new(
                        callee.span,
                        format!("unknown local library function `{name}`"),
                    )
                })?)
            }
            // Builtins are handled by the common checked lowering.
            (_, Kind::Member { base, .. }) if matches!(&base.kind, Kind::Name(name) if name == "core") => {
                None
            }
            // A namespace qualifier binds only a direct one-level call.
            (_, Kind::Member { base, field })
                if !matches!(self, Self::Library { imports: None, .. })
                    && matches!(&base.kind, Kind::Name(_)) =>
            {
                let Kind::Name(name) = &base.kind else {
                    unreachable!("the guard admits one-level names")
                };
                if let Some(interface) = self
                    .imports()
                    .and_then(|imports| imports.get(name.as_str()))
                {
                    Some(interface.public.get(field).ok_or_else(|| {
                        Diagnostic::new(
                            callee.span,
                            format!(
                                "library `{}` has no public function `{field}`",
                                interface.qualified
                            ),
                        )
                    })?)
                } else if matches!(self, Self::Library { .. }) {
                    return Err(Diagnostic::new(
                        callee.span,
                        format!(
                            "no `using` directive for namespace qualifier `{name}` in this file"
                        ),
                    ));
                } else {
                    None
                }
            }
            // A legacy leaf stays closed, and a package library may not use
            // dynamic callees or multi-part projections to reach another scope.
            (Self::Library { .. }, _) => {
                return Err(Diagnostic::new(
                    callee.span,
                    "leaf libraries may call local functions and core operations",
                ));
            }
            (Self::Entry { .. }, _) => None,
        };
        // Keep the original call span for any later type or effect diagnostic.
        if let Some(identity) = identity {
            callee.kind = Kind::Name(identity.clone());
        }
        Ok(())
    }
}
