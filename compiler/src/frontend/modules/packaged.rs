//! Resolve local package namespaces before lowering.
//!
//! Manifest loading owns immutable source snapshots. This pass borrows those
//! bytes, joins records, error families, and functions across each namespace,
//! and binds qualified names per file. Traversal uses an explicit stack, so
//! depth does not grow the host call stack. Applications start at their entry's
//! imports. Library checks start at every root source and pass declarations to
//! the same checker.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{
    ErrorNames, Imports, Interface, Members, PACKAGE_PREFIX, RecordNames, Scope, SourceFile,
    SourceProject, append_source, bind_entry, check_library_function, render, render_in_source,
    render_lowered, source_at,
};
use crate::frontend::{
    ast, check_library_module,
    functions::result_parts,
    lower_module,
    parse::{self, LeafPath, Library},
};
use crate::package::{Package, PackageGraph, PackageSource};
use crate::source::{Diagnostic, Span};

/// One discovered namespace in deterministic first-visit order.
struct Namespace {
    /// Qualified spelling and the public declarations importers can name.
    interface: Interface,
    /// Separate type and call lookups for all namespace contributors.
    local: LocalBindings,
    /// Contributing files in source-path order, each with separate imports.
    files: Vec<Library>,
}

/// Names visible across all contributing files of one namespace.
#[derive(Default)]
struct LocalBindings {
    /// Every function by bare source name and its private compiler identity.
    functions: Members,
    /// Every struct by bare source name and its canonical nominal schema name.
    records: RecordNames,
    /// Every error family, with the same nominal identity for local and public uses.
    errors: ErrorNames,
}

/// Traversal result. Namespace indexes remain stable until binding completes.
struct Namespaces {
    /// Full name to its discovery index and subsequently to its interface.
    index: HashMap<String, usize>,
    /// Each reachable namespace exactly once, independent of repeated imports.
    ordered: Vec<Namespace>,
}

/// One scheduled edge or completion marker for iterative depth-first traversal.
enum Visit {
    /// Check the edge's destination or open its namespace on a first visit.
    Enter(LeafPath),
    /// Mark the destination finished once all its outgoing edges complete.
    Leave(usize),
}

/// Original root library paths and their already allocated source offsets.
type RegisteredSources<'a> = HashMap<&'a Path, usize>;

/// Every namespace selected by the root library's source layout.
struct LibraryRoots<'a> {
    /// First occurrence of each namespace in source-path order.
    namespaces: Vec<LeafPath>,
    /// Root files already in the source map, including other contributors.
    offsets: RegisteredSources<'a>,
}

/// Resolve and lower one package entry using only the supplied source snapshots.
///
/// Local and qualified names are fully bound before the ordinary checked frontend
/// sees the program. The returned diagnostic map borrows the graph's source bytes.
/// Build-time tables own names and syntax only. No I/O or generated execution is
/// performed here, and the resulting program must still pass the checker.
pub fn parse_experimental_package(graph: &PackageGraph) -> Result<SourceProject<'_>, String> {
    let root = root_package(graph)?;
    let entry = root
        .entry
        .as_ref()
        .ok_or_else(|| format!("package `{}` has no entry source", graph.root))?;
    let entry = root
        .sources
        .get(entry)
        .ok_or_else(|| format!("package `{}` entry source is missing", graph.root))?;
    let mut sources = vec![SourceFile {
        path: entry.path.clone(),
        text: Cow::Borrowed(&entry.text),
        offset: 0,
    }];
    let (module, imports, mut naming_diagnostics) =
        parse::package_entry_with_naming(&entry.text, true)
            .map_err(|error| render(&sources, &error))?;
    validate_imports(graph, &graph.root, &imports).map_err(|error| render(&sources, &error))?;
    let namespaces = discover(
        graph,
        &imports,
        &mut sources,
        None,
        Some(&mut naming_diagnostics),
    )
    .map_err(|error| render(&sources, &error))?;
    let module =
        bind_package(module, &imports, namespaces).map_err(|error| render(&sources, &error))?;
    let program = lower_module(&module, naming_diagnostics)
        .map_err(|error| render_lowered(&sources, &error))?;
    Ok(SourceProject { program, sources })
}

/// Check a package that declares no entry, using every root Holo source.
///
/// Root namespaces and their reachable imports undergo the same name, body,
/// and effect checks as application dependencies. Snapshots are borrowed for
/// diagnostics. This performs no file I/O, invokes no native tool, and returns
/// only success or a diagnostic, with no executable entry or code-generation
/// authorization.
pub fn check_experimental_library_package(graph: &PackageGraph) -> Result<(), String> {
    check_library_package(graph, None).map(drop)
}

/// Check an entryless package and render selected root declarations' advisories.
/// A semantic failure returns its diagnostic before any advisories are rendered.
/// Strict callers receive error-formatted strings and decide their exit status.
/// The compiler borrows source snapshots and returns owned diagnostic text.
pub fn check_experimental_library_package_with_naming(
    graph: &PackageGraph,
    strict: bool,
) -> Result<Vec<String>, String> {
    let mut naming_diagnostics = Vec::new();
    let sources = check_library_package(graph, Some(&mut naming_diagnostics))?;
    // Library traversal follows imports, while registered root offsets follow
    // path order. Report in source order even when an import changes traversal.
    naming_diagnostics.sort_unstable_by_key(|diagnostic| diagnostic.span.start);
    let render_diagnostic = if strict {
        Diagnostic::render
    } else {
        Diagnostic::render_warning
    };
    Ok(naming_diagnostics
        .iter()
        .map(|diagnostic| render_in_source(&sources, diagnostic, render_diagnostic))
        .collect())
}

/// Check a library graph and return its borrowed diagnostic source map.
/// The optional caller-owned sink receives only selected root naming diagnostics.
/// On failure, callers discard partial advisories and receive the fatal message.
fn check_library_package<'a>(
    graph: &'a PackageGraph,
    naming_diagnostics: Option<&mut Vec<Diagnostic>>,
) -> Result<Vec<SourceFile<'a>>, String> {
    let root = root_package(graph)?;
    // An application has its own entry and reachability rules.
    if root.entry.is_some() {
        return Err(format!("package `{}` declares an entry source", graph.root));
    }
    // A root with no source has no namespaces or bodies to validate.
    if root.sources.is_empty() {
        return Ok(Vec::new());
    }
    let mut sources = Vec::new();
    let roots = register_library_sources(&graph.root, root, &mut sources)?;
    let namespaces = discover(
        graph,
        &roots.namespaces,
        &mut sources,
        Some(&roots.offsets),
        naming_diagnostics,
    )
    .map_err(|error| render(&sources, &error))?;
    let module = bind_package(
        ast::Module {
            records: Vec::new(),
            functions: Vec::new(),
            errors: Vec::new(),
        },
        &[],
        namespaces,
    )
    .map_err(|error| render(&sources, &error))?;
    check_library_module(&module).map_err(|error| render_lowered(&sources, &error))?;
    Ok(sources)
}

/// Borrow the graph's selected package without reopening any path.
fn root_package(graph: &PackageGraph) -> Result<&Package, String> {
    graph
        .packages
        .get(&graph.root)
        .ok_or_else(|| format!("missing root package `{}`", graph.root))
}

/// Register each root file once and seed its namespace from its physical path.
/// The first contributor's real source span anchors namespace-wide shape errors.
fn register_library_sources<'a>(
    owner: &str,
    root: &'a Package,
    sources: &mut Vec<SourceFile<'a>>,
) -> Result<LibraryRoots<'a>, String> {
    let mut roots = LibraryRoots {
        namespaces: Vec::new(),
        offsets: RegisteredSources::with_capacity(root.sources.len()),
    };
    let mut seen = HashSet::new();
    // Source keys are sorted and normalized by the manifest snapshot.
    for (relative, source) in &root.sources {
        let start = Span { start: 0, end: 0 };
        let offset = append_source(
            sources,
            source.path.clone(),
            Cow::Borrowed(&source.text),
            start,
        )
        .map_err(|error| error.render(&source.path.display().to_string(), &source.text))?;
        roots.offsets.insert(source.path.as_path(), offset);
        let span = Span {
            start: offset,
            end: offset,
        };
        let namespace =
            library_namespace(owner, relative, span).map_err(|error| render(sources, &error))?;
        // Several files may share one namespace, but it is discovered once.
        if seen.insert(namespace.qualified()) {
            roots.namespaces.push(namespace);
        }
    }
    Ok(roots)
}

/// Derive a source's namespace from its relative snapshot path. A file
/// directly under the source root uses its stem. Elsewhere the containing
/// directory provides all namespace components, including its final one.
fn library_namespace(owner: &str, relative: &Path, span: Span) -> Result<LeafPath, Diagnostic> {
    let directory = relative
        .parent()
        .expect("source snapshot paths are relative");
    // A nested file always contributes to its containing namespace, so its
    // filename cannot double as a child namespace identity.
    let path = if directory.as_os_str().is_empty() {
        Cow::Owned(relative.with_extension(""))
    } else {
        Cow::Borrowed(directory)
    };
    let invalid_path = || {
        Diagnostic::new(
            span,
            format!("invalid library namespace path `{}`", relative.display()),
        )
    };
    let mut parents = Vec::new();
    // Validate each component before storing it as a graph lookup identity.
    for component in path.components() {
        let segment = component
            .as_os_str()
            .to_str()
            .filter(|segment| parse::valid_leaf_name(segment))
            .ok_or_else(invalid_path)?;
        parents.push(segment.to_owned());
    }
    let leaf = parents.pop().ok_or_else(invalid_path)?;
    Ok(LeafPath {
        package: owner.to_owned(),
        parents,
        leaf,
        span,
    })
}

/// Bind an application entry, when present, and all selected declarations in
/// source order. Imports and visibility remain per-file through binding.
fn bind_package(
    mut module: ast::Module,
    imports: &[LeafPath],
    namespaces: Namespaces,
) -> Result<ast::Module, Diagnostic> {
    // Separate the borrowed interfaces from the bodies we will move and rewrite.
    let (interfaces, bodies): (Vec<_>, Vec<_>) = namespaces
        .ordered
        .into_iter()
        .map(|namespace| (namespace.interface, (namespace.local, namespace.files)))
        .unzip();
    bind_entry(
        &mut module,
        &file_scope(imports, &namespaces.index, &interfaces),
        true,
    )?;
    bind_namespaces(&mut module, bodies, &namespaces.index, &interfaces)?;
    Ok(module)
}

/// Check a file's dependency authority and qualifier uniqueness before following
/// any of its edges. A transitive package requires its own declared direct edge.
fn validate_imports(
    graph: &PackageGraph,
    owner: &str,
    imports: &[LeafPath],
) -> Result<(), Diagnostic> {
    let mut seen = HashMap::<&str, &LeafPath>::new();
    // File-local order yields deterministic errors and permits the same import
    // to be written independently in different files of one namespace.
    for import in imports {
        if import.package != owner && !graph.packages[owner].dependencies.contains(&import.package)
        {
            return Err(Diagnostic::new(
                import.span,
                format!(
                    "package `{}` is not a direct dependency of `{owner}`",
                    import.package
                ),
            ));
        }
        if let Some(first) = seen.insert(&import.leaf, import) {
            // Distinguish a repeated edge from two modules competing for its qualifier.
            let message = if first.package == import.package && first.parents == import.parents {
                format!("duplicate `using` directive for `{}`", import.qualified())
            } else {
                format!(
                    "namespace qualifier `{}` conflicts between `{}` and `{}`",
                    import.leaf,
                    first.qualified(),
                    import.qualified()
                )
            };
            return Err(Diagnostic::new(import.span, message));
        }
    }
    Ok(())
}

/// Traverse reachable namespaces in written import order and sorted file order.
/// Each source is parsed once. Active indexes keep edge classification constant
/// time, while a separate route can show the full cycle when an edge repeats.
/// Optionally collect root-package naming diagnostics as sources are parsed.
fn discover<'a>(
    graph: &'a PackageGraph,
    root_imports: &[LeafPath],
    sources: &mut Vec<SourceFile<'a>>,
    registered: Option<&RegisteredSources<'a>>,
    mut naming_diagnostics: Option<&mut Vec<Diagnostic>>,
) -> Result<Namespaces, Diagnostic> {
    let mut pending: Vec<_> = root_imports
        .iter()
        .rev()
        .cloned()
        .map(Visit::Enter)
        .collect();
    let mut route = Vec::<usize>::new();
    let mut active = Vec::<Option<usize>>::new();
    let mut namespaces = Namespaces {
        index: HashMap::new(),
        ordered: Vec::new(),
    };

    // Enter tasks precede their own Leave marker. Completed names can be shared
    // by any number of paths without reparsing or exposing another file's scope.
    while let Some(visit) = pending.pop() {
        let import = match visit {
            Visit::Enter(import) => import,
            Visit::Leave(index) => {
                active[index] = None;
                debug_assert_eq!(route.last(), Some(&index));
                route.pop();
                continue;
            }
        };
        let qualified = import.qualified();
        if let Some(&index) = namespaces.index.get(&qualified) {
            if let Some(start) = active[index] {
                let cycle = route[start..]
                    .iter()
                    .map(|&step| namespaces.ordered[step].interface.qualified.as_str())
                    .chain(std::iter::once(qualified.as_str()))
                    .collect::<Vec<_>>()
                    .join(" -> ");
                return Err(Diagnostic::new(
                    import.span,
                    format!("namespace dependency cycle: {cycle}"),
                ));
            }
            continue;
        }

        // Dependencies are still checked, but the selected package alone owns
        // the naming policy for this invocation.
        let diagnostics = if import.package == graph.root {
            naming_diagnostics.as_deref_mut()
        } else {
            None
        };
        let namespace = read_namespace(
            graph,
            sources,
            registered,
            &import,
            qualified.clone(),
            diagnostics,
        )?;
        let index = namespaces.ordered.len();
        namespaces.index.insert(qualified, index);
        active.push(Some(route.len()));
        route.push(index);
        pending.push(Visit::Leave(index));
        // Reverse scheduling preserves forward source and import order on pop.
        for file in namespace.files.iter().rev() {
            pending.extend(file.imports.iter().rev().cloned().map(Visit::Enter));
        }
        namespaces.ordered.push(namespace);
    }
    Ok(namespaces)
}

/// Select a root-level source or the immediate files of a namespace directory.
/// A range scan skips unrelated snapshot paths. Files in child directories
/// belong to their own namespace and do not contribute to or conflict with
/// this one.
fn namespace_files<'a>(
    package: &'a Package,
    import: &LeafPath,
    qualified: &str,
) -> Result<Vec<&'a PackageSource>, Diagnostic> {
    let directory: PathBuf = import
        .parents
        .iter()
        .chain(std::iter::once(&import.leaf))
        .collect();
    // Preserve the root-level single-file profile. Below the root, using a
    // file stem would make it ambiguous with a parent namespace contributor.
    let single = import
        .parents
        .is_empty()
        .then(|| package.sources.get(&directory.with_extension("holo")))
        .flatten();
    let mut members = package
        .sources
        .range(directory.clone()..)
        .take_while(|(path, _)| path.starts_with(&directory))
        .filter_map(|(path, source)| (path.parent() == Some(&directory)).then_some(source))
        .peekable();
    // A namespace has one source shape; selecting one over the other would
    // silently change its API as files are moved around the package.
    if let Some(single) = single {
        if members.peek().is_some() {
            return Err(Diagnostic::new(
                import.span,
                format!("namespace `{qualified}` has both a file and a directory"),
            ));
        }
        return Ok(vec![single]);
    }
    let files: Vec<_> = members.collect();
    // A directory with only child namespaces does not declare this namespace.
    if files.is_empty() {
        return Err(Diagnostic::new(
            import.span,
            format!("no source files for namespace `{qualified}`"),
        ));
    }
    Ok(files)
}

/// Parse and index every namespace file before binding any declaration.
/// Duplicate bare names fail within each lookup across all contributors. The
/// caller's source map gains the original borrowed files for all diagnostics.
/// When requested, parsed naming diagnostics are moved into the caller's sink.
fn read_namespace<'a>(
    graph: &'a PackageGraph,
    sources: &mut Vec<SourceFile<'a>>,
    registered: Option<&RegisteredSources<'a>>,
    import: &LeafPath,
    qualified: String,
    mut naming_diagnostics: Option<&mut Vec<Diagnostic>>,
) -> Result<Namespace, Diagnostic> {
    let package = graph.packages.get(&import.package).ok_or_else(|| {
        Diagnostic::new(
            import.span,
            format!(
                "package `{}` is missing from the resolved graph",
                import.package
            ),
        )
    })?;
    let files = namespace_files(package, import, &qualified)?;
    let mut namespace = Namespace {
        interface: Interface {
            qualified,
            public: Members::new(),
            public_records: RecordNames::new(),
            public_errors: ErrorNames::new(),
        },
        local: LocalBindings::default(),
        files: Vec::with_capacity(files.len()),
    };
    // Files are already in snapshot order, so the first duplicate location
    // and allocation of virtual source spans are stable between builds.
    for source in files {
        // Root library contributors were registered to locate path diagnostics.
        // Dependencies retain independent spans even if source roots overlap.
        let root_offset = registered
            .filter(|_| import.package == graph.root)
            .and_then(|offsets| offsets.get(source.path.as_path()).copied());
        let offset = match root_offset {
            Some(offset) => offset,
            None => append_source(
                sources,
                source.path.clone(),
                Cow::Borrowed(&source.text),
                import.span,
            )?,
        };
        let (file, diagnostics) =
            parse::package_library_with_naming(&source.text, offset, naming_diagnostics.is_some())?;
        validate_namespace_header(
            &file,
            import,
            &namespace.interface.qualified,
            offset,
            root_offset.is_some(),
        )?;
        validate_imports(graph, &import.package, &file.imports)?;
        // Commit advisories to the sink after this file's header is accepted.
        if let Some(sink) = &mut naming_diagnostics {
            sink.extend(diagnostics);
        }
        namespace.files.push(file);
    }
    // Fields and signatures can name records declared in later contributors.
    index_records(&mut namespace, sources)?;
    check_record_fields(&namespace)?;
    index_errors(&mut namespace, sources)?;
    index_functions(&mut namespace, sources)?;
    Ok(namespace)
}

/// Index families across all source contributors. Error names share the type
/// namespace with records, and only explicit public declarations enter imports.
fn index_errors(namespace: &mut Namespace, sources: &[SourceFile<'_>]) -> Result<(), Diagnostic> {
    let mut declared = HashMap::<&str, Span>::new();
    // Record lookup is already complete, including files ordered after this one.
    for file in &namespace.files {
        for declaration in &file.module.errors {
            let name = &declaration.name;
            if matches!(name.as_str(), "u64" | "Bytes" | "Result")
                || namespace.local.records.contains_key(name)
            {
                return Err(Diagnostic::new(
                    declaration.span,
                    format!("duplicate or reserved error type `{name}`"),
                ));
            }
            if let Some(first) = declared.insert(name, declaration.span) {
                let first = source_location(sources, first.start);
                return Err(Diagnostic::new(
                    declaration.span,
                    format!("duplicate library error family `{name}`; first declared at {first}"),
                ));
            }
            namespace.local.errors.insert(
                name.clone(),
                format!("{}.{name}", namespace.interface.qualified),
            );
        }
        // A canonical identity is assigned regardless of whether the family is exported.
        for name in &file.public_errors {
            namespace
                .interface
                .public_errors
                .insert(name.clone(), namespace.local.errors[name].clone());
        }
    }
    Ok(())
}

/// Collect every record in source order and expose only its public names.
/// Each schema identity is nominal. The graph selects one provider per package,
/// so its full namespace and type name are unique without a runtime descriptor.
fn index_records(namespace: &mut Namespace, sources: &[SourceFile<'_>]) -> Result<(), Diagnostic> {
    let mut declarations = HashMap::<&str, Span>::new();
    // All contributors share a record scope, including private declarations.
    for file in &namespace.files {
        for record in &file.module.records {
            // The two builtin types keep their ordinary meanings in every namespace.
            if matches!(record.name.as_str(), "u64" | "Bytes") {
                return Err(Diagnostic::new(
                    record.span,
                    "a struct cannot redefine the builtin `u64` or `Bytes` type",
                ));
            }
            if let Some(first) = declarations.insert(&record.name, record.span) {
                let first = source_location(sources, first.start);
                return Err(Diagnostic::new(
                    record.span,
                    format!(
                        "duplicate library struct `{}`; first declared at {first}",
                        record.name
                    ),
                ));
            }
            namespace.local.records.insert(
                record.name.clone(),
                format!("{}.{}", namespace.interface.qualified, record.name),
            );
        }
        // Only public declarations enter the import interface. Their names
        // already have canonical identities in the namespace's record map.
        for name in &file.public_records {
            namespace
                .interface
                .public_records
                .insert(name.clone(), namespace.local.records[name].clone());
        }
    }
    Ok(())
}

/// Check every declared field against the completed namespace type tables.
///
/// Builtins retain their ordinary owned storage semantics. Inline named types
/// must resolve locally, and every exposed field must name a public type.
/// Checking each edge also rejects transitive leaks through public records.
/// The ordinary schema resolver still checks acyclicity, initialization and
/// layout. This pass visits each field once and adds no runtime metadata.
fn check_record_fields(namespace: &Namespace) -> Result<(), Diagnostic> {
    // Check all contributors, even if no reachable constructor uses a record.
    for file in &namespace.files {
        for record in &file.module.records {
            let public = namespace
                .interface
                .public_records
                .contains_key(&record.name);
            for field in &record.fields {
                // Builtins have no declaration whose visibility can restrict them.
                if matches!(field.ty.as_str(), "u64" | "Bytes") {
                    continue;
                }
                // Reject unresolved names before merging namespaces with distinct type tables.
                if !namespace.local.records.contains_key(&field.ty) {
                    return Err(Diagnostic::new(
                        field.span,
                        format!(
                            "package field `{}` requires a namespace-local struct, `u64`, or `Bytes`; found `{}`",
                            field.name, field.ty
                        ),
                    ));
                }
                // All fields of public structs are exposed through live projections.
                if public && !namespace.interface.public_records.contains_key(&field.ty) {
                    return Err(Diagnostic::new(
                        field.span,
                        format!(
                            "public struct `{}` cannot expose private struct `{}` in field `{}`",
                            record.name, field.ty, field.name
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Collect namespace functions after all local schemas are known. Validate the
/// exported constructor boundary for every declaration, including unused ones.
fn index_functions(
    namespace: &mut Namespace,
    sources: &[SourceFile<'_>],
) -> Result<(), Diagnostic> {
    let mut declarations = HashMap::<&str, Span>::new();
    // File order gives repeatable duplicate diagnostics and interface binding.
    for file in &namespace.files {
        let public: HashSet<&str> = file.public_names.iter().map(String::as_str).collect();
        for function in &file.module.functions {
            check_library_function(
                function,
                Some(&namespace.local.records),
                Some(&namespace.local.errors),
            )?;
            // Exposing a result grants callers access to both component
            // declarations, so each nominal component must already be public.
            let exported = public.contains(function.name.as_str());
            if exported {
                let (success, family) = result_parts(&function.result)
                    .map_or((function.result.as_str(), None), |(success, family)| {
                        (success, Some(family))
                    });
                if let Some(family) = family
                    && !namespace.interface.public_errors.contains_key(family)
                {
                    return Err(Diagnostic::new(
                        function.span,
                        format!(
                            "public function `{}` cannot return private error family `{family}`",
                            function.name
                        ),
                    ));
                }
                if success != "u64" && !namespace.interface.public_records.contains_key(success) {
                    return Err(Diagnostic::new(
                        function.span,
                        format!(
                            "public constructor `{}` cannot return private struct `{success}`",
                            function.name
                        ),
                    ));
                }
            }
            // Bare-name lookup spans contributors, so every declaration must
            // have a distinct identity even if no other namespace can call it.
            if let Some(first) = declarations.insert(&function.name, function.span) {
                let first = source_location(sources, first.start);
                return Err(Diagnostic::new(
                    function.span,
                    format!(
                        "duplicate library function `{}`; first declared at {first}",
                        function.name
                    ),
                ));
            }
            let identity = format!(
                "{PACKAGE_PREFIX}{}.{}",
                namespace.interface.qualified, function.name
            );
            // Private calls and public calls to a declaration share its checked identity.
            if exported {
                namespace
                    .interface
                    .public
                    .insert(function.name.clone(), identity.clone());
            }
            namespace
                .local
                .functions
                .insert(function.name.clone(), identity);
        }
    }
    Ok(())
}

/// Check a package library's required namespace declaration against its path.
/// Failure points to the declaration, or the beginning of the file if absent.
fn validate_namespace_header(
    file: &Library,
    import: &LeafPath,
    qualified: &str,
    offset: usize,
    checked_root_source: bool,
) -> Result<(), Diagnostic> {
    let namespace = file.namespace.as_ref().ok_or_else(|| {
        Diagnostic::new(
            Span {
                start: offset,
                end: offset,
            },
            format!("expected `namespace {qualified}` in package library file"),
        )
    })?;
    // The source path or import supplies the identity. A declaration cannot
    // change the selected package or borrow another namespace's private names.
    if namespace.package != import.package
        || namespace.parents != import.parents
        || namespace.leaf != import.leaf
    {
        let expected_from = if checked_root_source {
            "source path for"
        } else {
            "`using` directive for"
        };
        return Err(Diagnostic::new(
            namespace.span,
            format!(
                "namespace `{}` does not match {expected_from} `{qualified}`",
                namespace.qualified()
            ),
        ));
    }
    Ok(())
}

/// Format the first declaration's original location for a duplicate diagnostic.
/// Span starts are parser byte boundaries inside the immutable source snapshot.
fn source_location(sources: &[SourceFile<'_>], position: usize) -> String {
    let file = source_at(sources, position);
    let line = file.text.as_bytes()[..position - file.offset]
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
        + 1;
    format!("{}:{line}", file.path.display())
}

/// Borrow one file's qualified interfaces from the fully checked import graph.
/// Validation ensures all qualifiers are unique and all destinations exist.
fn file_scope<'a>(
    imports: &'a [LeafPath],
    index: &HashMap<String, usize>,
    interfaces: &'a [Interface],
) -> Imports<'a> {
    imports
        .iter()
        .map(|import| {
            (
                import.leaf.as_str(),
                &interfaces[index[&import.qualified()]],
            )
        })
        .collect()
}

/// Rewrite and append namespace declarations without copying their ASTs. The
/// entry and libraries then enter the common checker and one native compilation.
fn bind_namespaces(
    module: &mut ast::Module,
    namespaces: Vec<(LocalBindings, Vec<Library>)>,
    index: &HashMap<String, usize>,
    interfaces: &[Interface],
) -> Result<(), Diagnostic> {
    // Interfaces and bodies were unzipped in discovery order. File imports never
    // change the private index shared by all contributors to that namespace.
    for ((local, files), interface) in namespaces.into_iter().zip(interfaces) {
        for file in files {
            // An import cannot shadow a declaration from any contributor,
            // because both record and function names are namespace-wide.
            for import in &file.imports {
                let shadowed = if local.functions.contains_key(&import.leaf) {
                    Some("function")
                } else if local.records.contains_key(&import.leaf) {
                    Some("struct")
                } else if local.errors.contains_key(&import.leaf) {
                    Some("error family")
                } else {
                    None
                };
                if let Some(kind) = shadowed {
                    return Err(Diagnostic::new(
                        import.span,
                        format!(
                            "namespace {kind} `{}` shadows an imported namespace",
                            import.leaf
                        ),
                    ));
                }
            }
            let imports = file_scope(&file.imports, index, interfaces);
            let scope = Scope::Library {
                local: &local.functions,
                imports: Some(&imports),
                records: Some(&local.records),
                errors: Some(&local.errors),
            };
            // Preserve the field order and bind the validated inline record
            // types to the same identities used by their constructors.
            for mut record in file.module.records {
                record.name = local.records[&record.name].clone();
                for field in &mut record.fields {
                    // Builtins have no namespace identity to rewrite.
                    if let Some(identity) = local.records.get(&field.ty) {
                        field.ty.clone_from(identity);
                    }
                }
                module.records.push(record);
            }
            // Definitions and every result signature use the same nominal family.
            for mut declaration in file.module.errors {
                // Only an exported declaration needs to expose all its children.
                let public = interface
                    .public_errors
                    .contains_key(&declaration.name)
                    .then_some(&interface.public_errors);
                scope.error_sources(&mut declaration, public)?;
                declaration.name = local.errors[&declaration.name].clone();
                module.errors.push(declaration);
            }
            // Resolve bodies before replacing names with their checked identities.
            for mut function in file.module.functions {
                scope.function(&mut function)?;
                // Return types and literals use the same schema identity as
                // the batch. Result families use the identity of their cases.
                if let Some(result) = local.records.get(&function.result) {
                    function.result.clone_from(result);
                } else if let Some((success, family)) = result_parts(&function.result) {
                    // Signature checking already established each nonbuiltin
                    // result component in the namespace's corresponding map.
                    let success = if success == "u64" {
                        success
                    } else {
                        &local.records[success]
                    };
                    function.result = format!("Result<{success},{}>", local.errors[family]);
                }
                function.name = local.functions[&function.name].clone();
                module.functions.push(function);
            }
        }
    }
    Ok(())
}
