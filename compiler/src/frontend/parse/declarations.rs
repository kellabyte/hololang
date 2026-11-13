//! Top-level records, error families, and aligned function-header parsing.
//!
//! The parser requires explicit argument access/passing modes and an explicit
//! result clause. It preserves declarations and spans; resolving their types,
//! contracts, and constructor behavior belongs to frontend lowering/checking.

use super::{LeafPath, Library, NameKind, Parser, PathMode, valid_leaf_name, valid_path_segment};
use crate::frontend::ast::{
    Argument, EnumDecl, ErrorCaseDecl, ErrorDecl, FieldDecl, Function, Module, RecordDecl,
};
use crate::lex::TokenKind;
use crate::source::{Diagnostic, Span};

impl Parser<'_> {
    /// Parse `using` directives and ordinary declarations in either entry mode.
    /// The parser owns the returned syntax and performs no dependency reads.
    pub(super) fn project_entry(
        &mut self,
        mode: PathMode,
    ) -> Result<(Module, Vec<LeafPath>), Diagnostic> {
        self.separators();
        let imports = self.imports(mode)?;
        Ok((self.module()?, imports))
    }

    /// Parse a library's headers and declarations and collect public names for
    /// binding. Package libraries also accept records and errors. Bodies use the ordinary
    /// parser and the resolver checks identity without adding generated state.
    pub(super) fn library(&mut self, mode: PathMode) -> Result<Library, Diagnostic> {
        let mut records = Vec::new();
        let mut enums = Vec::new();
        let mut errors = Vec::new();
        let mut functions = Vec::new();
        let mut public_names = Vec::new();
        let mut public_records = Vec::new();
        let mut public_enums = Vec::new();
        let mut public_errors = Vec::new();
        self.separators();
        // Recognize the optional source identity before any declarations.
        let namespace = if self.at_word("namespace") {
            let path = self.leaf_path("namespace", mode)?;
            self.end_statement()?;
            Some(path)
        } else {
            None
        };
        // The legacy loader keeps its leaf-only boundary. In a package each
        // file keeps its own `using` directives after the namespace header.
        let imports = match mode {
            PathMode::Rooted => Vec::new(),
            PathMode::Package => self.imports(mode)?,
        };
        // Parse each declaration exactly once after the headers.
        while !matches!(self.current().kind, TokenKind::Eof) {
            self.reject_import_directive()?;
            // A second or late header cannot rename part of a library. Keep
            // the rooted diagnostic specific to its function-only grammar.
            if self.at_word("namespace") {
                return Err(Diagnostic::new(
                    self.span(),
                    match mode {
                        PathMode::Rooted => {
                            "namespace declaration must appear once before library functions"
                        }
                        PathMode::Package => {
                            "namespace declaration must appear once before library declarations"
                        }
                    },
                ));
            }
            let start = self.span().start;
            let public = self.take_word("public");
            // Package types reuse standalone declaration grammar. The rooted
            // branch still requires a function, including after `public`.
            if matches!(mode, PathMode::Package) && self.at_word("struct") {
                let mut record = self.record()?;
                // Include the visibility marker in the formatter's declaration
                // boundary and collect only published names for the resolver.
                if public {
                    record.span.start = start;
                    public_records.push(record.name.clone());
                }
                records.push(record);
            } else if matches!(mode, PathMode::Package) && self.at_word("enum") {
                let mut declaration = self.enumeration()?;
                if public {
                    declaration.span.start = start;
                    public_enums.push(declaration.name.clone());
                }
                enums.push(declaration);
            } else if matches!(mode, PathMode::Package) && self.at_word("error") {
                let mut error = self.error_family()?;
                // Retain the marker for formatting and export the name only in package mode.
                if public {
                    error.span.start = start;
                    public_errors.push(error.name.clone());
                }
                errors.push(error);
            } else {
                let mut function = self.function()?;
                // Keep the function's marker attached and record its visibility
                // separately from its ordinary parsed body.
                if public {
                    function.span.start = start;
                    public_names.push(function.name.clone());
                }
                functions.push(function);
            }
            self.separators();
        }
        Ok(Library {
            module: Module {
                records,
                enums,
                errors,
                functions,
            },
            public_names,
            public_records,
            public_enums,
            public_errors,
            namespace,
            imports,
        })
    }

    /// Read leading `using` directives and leave the cursor at the declarations.
    /// Each returned path owns its written name segments. A malformed header stops
    /// parsing before resolution can read any dependency.
    fn imports(&mut self, mode: PathMode) -> Result<Vec<LeafPath>, Diagnostic> {
        let mut imports = Vec::new();
        // Each directive consumes a statement boundary before the next header.
        while self.at_word("using") {
            imports.push(self.leaf_path("using", mode)?);
            self.end_statement()?;
        }
        Ok(imports)
    }

    /// Diagnose the retired directive at a file declaration boundary.
    /// Names in function signatures and bodies remain contextual identifiers.
    fn reject_import_directive(&self) -> Result<(), Diagnostic> {
        // At this boundary a bare `import` can only be a former directive.
        if self.at_word("import") {
            return Err(Diagnostic::new(
                self.span(),
                "replace `import` with `using` for namespace directives",
            ));
        }
        Ok(())
    }

    /// Read a header and its qualified name without consuming the terminator.
    /// The rooted shape is exactly `holo.leaf`. Package paths have one or more
    /// namespace segments, shared by imports and declarations.
    fn leaf_path(&mut self, keyword: &str, mode: PathMode) -> Result<LeafPath, Diagnostic> {
        let start = self.expect_word(keyword)?.start;
        // Package mode resolves the owner later. The legacy mode retains its
        // root restriction and diagnostic before opening any file.
        let (package, package_span) = match mode {
            PathMode::Rooted => {
                let span = self.expect_word("holo")?;
                ("holo".to_owned(), span)
            }
            PathMode::Package => {
                let (package, span) = self.identifier()?;
                // Keep path validity separate from the naming convention.
                if !valid_path_segment(&package)
                    || matches!(package.as_str(), "core" | "true" | "false")
                {
                    return Err(Diagnostic::new(span, "expected a lowercase package name"));
                }
                (package, span)
            }
        };
        self.expect_symbol('.')?;
        // A namespace header declares each component. A using directive
        // references an existing identity and contributes no naming warning.
        if keyword == "namespace" {
            self.record_naming(&package, package_span, NameKind::Namespace);
        }
        let mut parents = Vec::new();
        let (leaf, end) = loop {
            let (segment, span) = self.identifier()?;
            // Match source path validity on every component.
            if !valid_leaf_name(&segment) {
                return Err(Diagnostic::new(
                    span,
                    "expected a lowercase library leaf name",
                ));
            }
            if keyword == "namespace" {
                self.record_naming(&segment, span, NameKind::Namespace);
            }
            // Package mode consumes more dotted segments. A second dot
            // in rooted mode is still rejected at the statement terminator.
            if matches!(mode, PathMode::Package) && self.take_symbol('.') {
                parents.push(segment);
            } else {
                break (segment, span.end);
            }
        };
        Ok(LeafPath {
            package,
            parents,
            leaf,
            span: Span { start, end },
        })
    }

    /// Consume complete top-level declarations until EOF, retaining source order.
    /// Unknown top-level forms are rejected rather than interpreted as executable code.
    pub(super) fn module(&mut self) -> Result<Module, Diagnostic> {
        let mut records = Vec::new();
        let mut enums = Vec::new();
        let mut errors = Vec::new();
        let mut functions = Vec::new();
        self.separators();
        // Each recognized declaration consumes at least its introducer and braces.
        while !matches!(self.current().kind, TokenKind::Eof) {
            self.reject_import_directive()?;
            if self.at_word("struct") {
                records.push(self.record()?);
            } else if self.at_word("enum") {
                enums.push(self.enumeration()?);
            } else if self.at_word("error") {
                errors.push(self.error_family()?);
            } else if self.at_word("func") {
                functions.push(self.function()?);
            } else {
                return Err(Diagnostic::new(
                    self.span(),
                    "expected `struct` or `func` declaration, an `enum`, or an `error` family",
                ));
            }
            self.separators();
        }
        Ok(Module {
            records,
            enums,
            errors,
            functions,
        })
    }

    /// Parse a closed list of variant names without assigning representation tags.
    fn enumeration(&mut self) -> Result<EnumDecl, Diagnostic> {
        let start = self.expect_word("enum")?.start;
        let (name, _) = self.declared_identifier(NameKind::Enum)?;
        self.newlines();
        self.expect_symbol('{')?;
        self.separators();
        let mut variants = Vec::new();
        while !self.at_symbol('}') {
            variants.push(self.declared_identifier(NameKind::EnumVariant)?);
            if self.take_symbol(',') || matches!(self.current().kind, TokenKind::Ident(_)) {
                self.separators();
            } else {
                self.end_statement()?;
            }
        }
        let end = self.expect_symbol('}')?.end;
        Ok(EnumDecl {
            name,
            variants,
            span: Span { start, end },
        })
    }

    /// Read one error declaration with local cases or named child families.
    /// Owns parsed names and preserves spans for semantic type and identity checks.
    fn error_family(&mut self) -> Result<ErrorDecl, Diagnostic> {
        let start = self.expect_word("error")?.start;
        let (name, _) = self.declared_identifier(NameKind::ErrorFamily)?;
        self.newlines();
        self.expect_symbol('{')?;
        let mut cases = Vec::new();
        self.separators();
        // Each successful iteration consumes a name and its boundary. An EOF
        // fails through `identifier`, so an unterminated family cannot spin.
        while !self.at_symbol('}') {
            let (name, span) = self.identifier()?;
            // `from` followed by a family name introduces a child alias. Bare
            // `from` and `from(payload u64)` remain ordinary case names.
            let (name, payload, source, end) =
                if name == "from" && matches!(&self.current().kind, TokenKind::Ident(_)) {
                    let source = self.case_name()?;
                    self.expect_word("as")?;
                    let (alias, end) = self.declared_identifier(NameKind::ChildAlias)?;
                    (alias, None, Some(source), end.end)
                } else if self.take_symbol('(') {
                    self.record_naming(&name, span, NameKind::ErrorCase);
                    self.newlines();
                    let (field, begin) = self.declared_identifier(NameKind::Field)?;
                    let (ty, end) = self.type_name()?;
                    self.newlines();
                    let close = self.expect_symbol(')')?;
                    (
                        name,
                        Some(FieldDecl {
                            name: field,
                            ty,
                            span: Span {
                                start: begin.start,
                                end: end.end,
                            },
                        }),
                        None,
                        close.end,
                    )
                } else {
                    self.record_naming(&name, span, NameKind::ErrorCase);
                    (name, None, None, span.end)
                };
            cases.push(ErrorCaseDecl {
                name,
                payload,
                source,
                span: Span {
                    start: span.start,
                    end,
                },
            });
            // Commas permit compact lists with the same semantics as written lines.
            if self.take_symbol(',') {
                self.separators();
            } else {
                self.end_statement()?;
            }
        }
        let end = self.expect_symbol('}')?.end;
        Ok(ErrorDecl {
            name,
            cases,
            span: Span { start, end },
        })
    }

    /// Parse a record's ordered logical fields without deciding their native layout.
    /// The returned span covers both braces; field/type names remain unresolved.
    fn record(&mut self) -> Result<RecordDecl, Diagnostic> {
        let start = self.expect_word("struct")?.start;
        let (name, _) = self.declared_identifier(NameKind::Record)?;
        self.newlines();
        self.expect_symbol('{')?;
        let mut fields = Vec::new();
        self.separators();
        // Stop at the record's brace; EOF inside a record is always a missing delimiter.
        while !self.at_symbol('}') {
            if matches!(self.current().kind, TokenKind::Eof) {
                return Err(Diagnostic::new(
                    self.span(),
                    "expected `}` to end the struct",
                ));
            }
            let (field, begin) = self.declared_identifier(NameKind::Field)?;
            let (ty, end) = self.type_name()?;
            fields.push(FieldDecl {
                name: field,
                ty,
                span: Span {
                    start: begin.start,
                    end: end.end,
                },
            });
            // A comma is convenient for compact declarations; ordinary aligned fields use lines.
            if self.take_symbol(',') {
                self.separators();
            } else {
                self.end_statement()?;
            }
        }
        let end = self.expect_symbol('}')?.end;
        Ok(RecordDecl {
            name,
            fields,
            span: Span { start, end },
        })
    }

    /// Parse the preferred function header and one executable body.
    /// `with` precedes arguments, `returns` introduces the result type, and `{` ends the header.
    fn function(&mut self) -> Result<Function, Diagnostic> {
        let start = self.expect_word("func")?.start;
        let (name, _) = self.declared_identifier(NameKind::Function)?;
        self.newlines();
        let mut guarantees = Vec::new();
        // Only one contract clause is admitted; semantic validation checks its names/effects.
        if self.take_word("with") {
            self.newlines();
            guarantees.push(self.identifier()?);
            while self.take_symbol(',') {
                self.newlines();
                guarantees.push(self.identifier()?);
            }
            self.newlines();
        }
        let mut args = Vec::new();
        // Fixed header-clause keywords remove any dependence on indentation width.
        while self.at_word("arg") {
            let begin = self.expect_word("arg")?.start;
            self.newlines();
            let (argument, _) = self.declared_identifier(NameKind::Parameter)?;
            self.newlines();
            let (ty, type_span) = self.argument_type()?;
            self.newlines();
            self.expect_word("as")?;
            self.newlines();
            let (access, access_span) = self.identifier()?;
            if !matches!(access.as_str(), "readonly" | "readwrite") {
                return Err(Diagnostic::new(
                    access_span,
                    "argument access must be `readonly` or `readwrite`",
                ));
            }
            self.newlines();
            let (passing, mode_span) = self.identifier()?;
            if !matches!(passing.as_str(), "copy" | "reference") {
                return Err(Diagnostic::new(
                    mode_span,
                    "argument passing must be `copy` or `reference`",
                ));
            }
            args.push(Argument {
                name: argument,
                ty,
                type_span,
                access,
                passing,
                span: Span {
                    start: begin,
                    end: mode_span.end,
                },
            });
            self.newlines();
        }
        // Reordered/duplicate clauses should fail at their introducer, not later in the body.
        if self.at_word("with") {
            return Err(Diagnostic::new(
                self.span(),
                "a single `with` clause must precede all arguments",
            ));
        }
        self.expect_word("returns")?;
        self.newlines();
        let (result, result_span, typed_return) = self.return_type()?;
        self.newlines();
        let previous = std::mem::replace(&mut self.typed_return, typed_return);
        let body = self.block();
        self.typed_return = previous;
        let (body, end) = body?;
        Ok(Function {
            name,
            args,
            guarantees,
            result,
            result_span,
            body,
            span: Span { start, end },
        })
    }

    /// Parse a named argument type or a batch of one named record.
    /// State and batch schemas may use a namespace import qualifier.
    fn argument_type(&mut self) -> Result<(String, Span), Diagnostic> {
        if !self.at_word("Batch") || self.tokens[self.position + 1].kind != TokenKind::Symbol('<') {
            return self.argument_record_name();
        }
        let start = self.expect_word("Batch")?.start;
        self.expect_symbol('<')?;
        self.newlines();
        let (record, _) = self.argument_record_name()?;
        self.newlines();
        let end = self.expect_symbol('>')?.end;
        Ok((format!("Batch<{record}>"), Span { start, end }))
    }

    /// Keep an optional namespace qualifier with the type name for binding.
    fn argument_record_name(&mut self) -> Result<(String, Span), Diagnostic> {
        let (mut name, mut span) = self.type_name()?;
        if self.take_symbol('.') {
            let (member, member_span) = self.type_name()?;
            name.push('.');
            name.push_str(&member);
            span.end = member_span.end;
        }
        Ok((name, span))
    }

    /// Parse scalar Option and Result return types, canonicalizing their spacing.
    /// Named returns keep their ordinary grammar. The flag selects Result payload syntax.
    fn return_type(&mut self) -> Result<(String, Span, bool), Diagnostic> {
        // Bare Option and Result can still name user-defined types.
        let option = self.at_word("Option");
        if !(option || self.at_word("Result"))
            || self.tokens[self.position + 1].kind != TokenKind::Symbol('<')
        {
            let (name, span) = self.type_name()?;
            return Ok((name, span, false));
        }
        let start = self.advance().span.start;
        self.expect_symbol('<')?;
        self.newlines();
        let (success, success_span) = self.type_name()?;
        self.newlines();
        if option {
            if success != "u64" {
                return Err(Diagnostic::new(
                    success_span,
                    "optional function returns currently require `Option<u64>`",
                ));
            }
            let end = self.expect_symbol('>')?.end;
            return Ok((format!("Option<{success}>"), Span { start, end }, false));
        }
        self.expect_symbol(',')?;
        self.newlines();
        let (family, _) = self.type_name()?;
        self.newlines();
        let end = self.expect_symbol('>')?.end;
        Ok((
            format!("Result<{success},{family}>"),
            Span { start, end },
            true,
        ))
    }

    /// Parse one simple named type, explicitly excluding unfinished generic grammar.
    /// Returns its spelling/span; supported types are checked by lowering later.
    fn type_name(&mut self) -> Result<(String, Span), Diagnostic> {
        let result = self.identifier()?;
        // Reject rather than accidentally discard a parameterized type's remainder.
        if self.at_symbol('<') {
            return Err(Diagnostic::new(
                self.span(),
                "parameterized types are not supported by the experimental source subset",
            ));
        }
        Ok(result)
    }
}
