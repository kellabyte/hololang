//! Conservative source formatting for the experimental Holo grammar.
//!
//! Parsing establishes syntactic boundaries before whitespace is rewritten. All
//! identifiers, literal spellings, punctuation, and comment bytes remain in order;
//! existing statement newlines remain visible. New line breaks occur only at
//! parsed declaration/statement/field boundaries, braces, semicolons, and preferred
//! function-header clauses. Result arms retain their enclosing expression's
//! indentation boundary. The result is reparsed and compared without spans
//! before it is returned. This is a source tool, not semantic checking or lowering.
//!
//! The formatter owns an output string and a lossless token vector, while parser
//! trees allocate host memory. Boundary sorting costs O(b log b) for b selected
//! boundaries; emission scans tokens/source once and performs no width reflow.

use crate::frontend::{ast, parse};
use crate::lex::{Lexeme, LexemeKind, scan};
use crate::source::{Diagnostic, Span};

/// Fixed indentation for the currently experimental canonical source layout.
const INDENT_WIDTH: usize = 4;

/// One parsed header's range after its name and before its opening body brace.
struct Header {
    /// First byte after the function name; later header lines indent once.
    start: usize,
    /// Opening body brace, which aligns with the function declaration.
    end: usize,
}

/// Parsed source locations where inserting a line boundary preserves grammar.
struct Layout {
    /// Sorted, unique byte offsets requiring a line break before their token.
    breaks: Vec<usize>,
    /// Source-ordered function header ranges, excluding their body braces.
    headers: Vec<Header>,
    /// Source-ordered type ranges where angle brackets are type punctuation.
    header_types: Vec<Span>,
    /// Source-ordered result arm lists, potentially nested through block arms.
    handlers: Vec<Span>,
    /// Indentation-delimited destination field lists, disjoint by grammar.
    destinations: Vec<Span>,
    /// Contextual words that separate from a grouped operand.
    spaced_words: Vec<usize>,
}

/// Pending layout and the already emitted source; comments are borrowed on write.
struct Output {
    /// Owned formatted source, initially reserved to the original byte length.
    text: String,
    /// Existing source line boundaries waiting before the next real lexeme.
    source_breaks: usize,
    /// Whether a parsed structural boundary additionally requires one newline.
    forced_break: bool,
    /// Previous emitted lexeme on the same line, used only for horizontal spacing.
    previous: Option<Lexeme>,
}

/// Format syntactically valid experimental `source`, preserving its parsed meaning.
/// Returns owned text or the first syntax diagnostic; no files or compiler state
/// are mutated. Malformed source produces no partial output. Numeric policies,
/// identifiers, comments, declaration order, and expression grouping are preserved;
/// the formatter does not require the restricted semantic/lifetime checker to pass.
pub fn format(source: &str) -> Result<String, Diagnostic> {
    let original = parse::for_format(source)?;
    let tokens: Vec<_> = scan(source).collect();
    let layout = Layout::new(&original, source, &tokens);
    let formatted = emit(source, &tokens, &layout);
    let reparsed = parse::for_format(&formatted).map_err(|_| {
        Diagnostic::new(
            Span { start: 0, end: 0 },
            "formatter could not preserve this experimental source layout",
        )
    })?;
    // Token equality protects comments, literal spellings, grouping, and original
    // declaration interleaving; AST equality additionally protects newline meaning.
    if !same_tokens(source, &formatted) || !same_module(&original, &reparsed) {
        return Err(Diagnostic::new(
            Span { start: 0, end: 0 },
            "formatter rejected a layout that changed source structure",
        ));
    }
    Ok(formatted)
}

impl Layout {
    /// Collect safe layout boundaries from parsed syntax and lossless `tokens`.
    /// Owns offsets only; source bytes are never copied. The parser has already
    /// validated that each function's first brace ends its simple-type header.
    fn new(module: &ast::Module, source: &str, tokens: &[Lexeme]) -> Self {
        let mut layout = Self {
            breaks: Vec::new(),
            headers: Vec::with_capacity(module.functions.len()),
            header_types: Vec::new(),
            handlers: Vec::new(),
            destinations: Vec::new(),
            spaced_words: Vec::new(),
        };
        // Field and declaration boundaries come from syntax. A public record's
        // boundary includes its marker and a field named `return` stays a field.
        for record in &module.records {
            layout.breaks.push(record.span.start);
            for field in &record.fields {
                layout.breaks.push(field.span.start);
            }
        }
        // Error declarations share record-style case boundaries, including a public marker.
        for error in &module.errors {
            layout.breaks.push(error.span.start);
            layout
                .breaks
                .extend(error.cases.iter().map(|case| case.span.start));
        }
        let mut cursor = 0;
        // Functions retain source order even though the module stores record and
        // function declarations separately. The monotonic cursor visits tokens once.
        for function in &module.functions {
            while tokens[cursor].span.start < function.span.start {
                cursor += 1;
            }
            let mut header = tokens[cursor..]
                .iter()
                .take_while(|token| token.kind != LexemeKind::Symbol('{'))
                .filter(|token| !trivia(token.kind));
            let introducer = header.next().expect("parsed function introducer");
            // The public marker shares the declaration line with `func`.
            if &source[introducer.span.start..introducer.span.end] == "public" {
                header.next().expect("parsed public function introducer");
            }
            let name = header.next().expect("parsed function name");
            layout.breaks.push(introducer.span.start);
            // The result span marks the preceding `returns` clause even when
            // the type contains several tokens or begins on the next line.
            let result_keyword = header
                .take_while(|token| token.span.end <= function.result_span.start)
                .last()
                .expect("parsed returns keyword");
            layout.breaks.push(result_keyword.span.start);
            // A written contract clause immediately follows the function name;
            // parsed guarantee spans distinguish it from argument identifiers.
            if let Some((_, guarantee)) = function.guarantees.first() {
                let with = tokens[cursor..]
                    .iter()
                    .take_while(|token| token.span.start < guarantee.start)
                    .filter(|token| !trivia(token.kind))
                    .last()
                    .expect("parsed with introducer");
                layout.breaks.push(with.span.start);
            }
            for argument in &function.args {
                layout.breaks.push(argument.span.start);
                layout.header_types.push(argument.type_span);
            }
            layout.header_types.push(function.result_span);
            // This cursor advances to a syntactically established opening brace,
            // rather than inferring block kind from capitalization or token spelling.
            while tokens[cursor].kind != LexemeKind::Symbol('{') {
                cursor += 1;
            }
            let end = tokens[cursor].span.start;
            layout.breaks.push(end);
            layout.headers.push(Header {
                start: name.span.end,
                end,
            });
            layout.collect_statements(&function.body);
        }
        // Distinct AST paths can name the same starting token (a statement and
        // its record expression); one sorted boundary prevents duplicate breaks.
        layout.breaks.sort_unstable();
        layout.breaks.dedup();
        layout.handlers.sort_unstable_by_key(|range| range.start);
        layout
            .destinations
            .sort_unstable_by_key(|range| range.start);
        layout.spaced_words.sort_unstable();
        layout
    }

    /// Collect parsed statement and nested record-field starts into the layout.
    /// Traversal follows the parser's bounded tree and stores source offsets only.
    fn collect_statements(&mut self, statements: &[ast::Statement]) {
        // Every recursive descent visits a strict child; parser depth limits bound stack use.
        for statement in statements {
            self.breaks.push(statement.span.start);
            // Each syntax variant contributes its expression children and written bodies.
            match &statement.kind {
                ast::StatementKind::Let {
                    value, otherwise, ..
                } => {
                    self.collect_expression(value);
                    if let Some(body) = otherwise {
                        self.collect_statements(body);
                    }
                }
                ast::StatementKind::Assign { target, value } => {
                    self.collect_expression(target);
                    self.collect_expression(value);
                }
                ast::StatementKind::Expr(expression) | ast::StatementKind::Return(expression) => {
                    self.collect_expression(expression);
                }
                ast::StatementKind::PrintText(_) => {}
                ast::StatementKind::Construct { subject, arms }
                | ast::StatementKind::HandleResult { subject, arms } => {
                    self.collect_expression(subject);
                    // The continuation level belongs to the arm headers and
                    // every line of their blocks, including each closing brace.
                    self.handlers.push(Span {
                        start: arms[0].span.start,
                        end: statement.span.end,
                    });
                    for arm in arms {
                        self.collect_statements(&arm.body);
                    }
                }
                ast::StatementKind::Scope(body)
                | ast::StatementKind::Loop(body)
                | ast::StatementKind::TcpScope { body, .. } => {
                    self.collect_statements(body);
                }
                ast::StatementKind::WithRegion {
                    capacity,
                    body,
                    on_failure,
                } => {
                    // The capacity stays in the introducer. Success and
                    // recovery contribute separate statement boundaries.
                    self.collect_expression(capacity);
                    self.collect_statements(body);
                    self.collect_statements(on_failure);
                }
                ast::StatementKind::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    self.collect_expression(condition);
                    self.collect_statements(then_body);
                    self.collect_statements(else_body);
                }
                ast::StatementKind::For { iterable, body, .. } => {
                    self.collect_expression(iterable);
                    self.collect_statements(body);
                }
                ast::StatementKind::Refill {
                    slot,
                    limit,
                    data_body,
                    other_body,
                    ..
                } => {
                    // Each outcome has its own statement boundaries. The slot and
                    // limit stay in the original header and may contain expressions.
                    self.collect_expression(slot);
                    self.collect_expression(limit);
                    self.collect_statements(data_body);
                    self.collect_statements(other_body);
                }
            }
        }
    }

    /// Visit `expression` for record boundaries and result continuations, preserving order.
    /// Mutates layout offsets only; scalar leaves have no children or allocation cost.
    fn collect_expression(&mut self, expression: &ast::Expr) {
        // Match actual syntax so braces used for values receive the same safe field
        // boundaries as declaration braces without treating arbitrary names as types.
        match &expression.kind {
            ast::ExprKind::Call { callee, args } => {
                self.collect_expression(callee);
                for argument in args {
                    self.collect_expression(argument);
                }
            }
            ast::ExprKind::Member { base, .. } => self.collect_expression(base),
            ast::ExprKind::Index { base, index } => {
                self.collect_expression(base);
                self.collect_expression(index);
            }
            ast::ExprKind::Record {
                fields, indented, ..
            } => {
                // Braces already indent their own fields; a colon needs a range.
                if *indented {
                    self.destinations.push(expression.span);
                }
                for field in fields {
                    self.breaks.push(field.span.start);
                    self.collect_expression(&field.value);
                }
            }
            ast::ExprKind::Binary {
                op,
                operator,
                left,
                right,
            } => {
                if matches!(op, ast::BinaryOp::LogicalAnd | ast::BinaryOp::LogicalOr) {
                    self.spaced_words.push(operator.start);
                }
                self.collect_expression(left);
                self.collect_expression(right);
            }
            ast::ExprKind::Not { operator, value } => {
                self.spaced_words.push(operator.start);
                self.collect_expression(value);
            }
            ast::ExprKind::Ok(value) => {
                self.spaced_words.push(expression.span.start);
                self.collect_expression(value);
            }
            ast::ExprKind::Error { payload, .. } => {
                // A payload can contain an ordinary expression with its own formatter boundaries.
                if let Some(payload) = payload {
                    self.collect_expression(payload);
                }
            }
            ast::ExprKind::Try(value) => self.collect_expression(value),
            ast::ExprKind::Handle { subject, arms } => {
                self.collect_expression(subject);
                // Parsed dispatches always contain at least one arm. Keep comments
                // between them on the same continuation indentation as the arms.
                self.handlers.push(Span {
                    start: arms[0].span.start,
                    end: expression.span.end,
                });
                for arm in arms {
                    self.collect_expression(&arm.value);
                }
            }
            ast::ExprKind::Number(_)
            | ast::ExprKind::Bool(_)
            | ast::ExprKind::Bytes(_)
            | ast::ExprKind::Name(_)
            | ast::ExprKind::ErrorFamilyRoot { .. } => {}
        }
    }
}

/// Emit `tokens` with `layout`, retaining every non-whitespace source fragment.
/// Returns one owned output; brace/list counters control indentation only and are
/// safe because the parser established balanced delimiters before this function.
fn emit(source: &str, tokens: &[Lexeme], layout: &Layout) -> String {
    let mut output = Output {
        text: String::with_capacity(source.len()),
        source_breaks: 0,
        forced_break: false,
        previous: None,
    };
    let mut braces = 0usize;
    let mut lists = 0usize;
    let mut boundary = 0usize;
    let mut header = 0usize;
    let mut header_type = 0usize;
    let mut handler = 0usize;
    let mut active_handlers = Vec::new();
    let mut destination = 0usize;
    let mut spaced_word = 0usize;
    // Lexemes partition the immutable source. Each token is emitted or consumed
    // as pending whitespace exactly once, including complete multiline comments.
    for &token in tokens {
        match token.kind {
            LexemeKind::Whitespace => continue,
            LexemeKind::Newline => {
                output.source_breaks = (output.source_breaks + 1).min(2);
                continue;
            }
            _ => {}
        }
        // Monotonic boundary/header cursors avoid searching the complete offset
        // vectors for every token. A brace belongs to its body, not its header.
        while boundary < layout.breaks.len() && layout.breaks[boundary] < token.span.start {
            boundary += 1;
        }
        if boundary < layout.breaks.len() && layout.breaks[boundary] == token.span.start {
            output.forced_break = true;
            boundary += 1;
        }
        while header < layout.headers.len() && token.span.start >= layout.headers[header].end {
            header += 1;
        }
        let in_header =
            header < layout.headers.len() && token.span.start >= layout.headers[header].start;
        while let Some(span) = layout.header_types.get(header_type)
            && token.span.start >= span.end
        {
            header_type += 1;
        }
        let in_type = layout
            .header_types
            .get(header_type)
            .is_some_and(|span| token.span.start >= span.start && token.span.start < span.end);
        // A block arm may itself contain another arm list. Nested ranges finish
        // innermost first; each is pushed and popped once in the token scan.
        while active_handlers
            .last()
            .is_some_and(|&end| token.span.start >= end)
        {
            active_handlers.pop();
        }
        while let Some(range) = layout
            .handlers
            .get(handler)
            .filter(|range| token.span.start >= range.start)
        {
            active_handlers.push(range.end);
            handler += 1;
        }
        // Constructor fields use a continuation level without introducing braces.
        while destination < layout.destinations.len()
            && token.span.start >= layout.destinations[destination].end
        {
            destination += 1;
        }
        let in_destination = destination < layout.destinations.len()
            && token.span.start >= layout.destinations[destination].start;
        // Closing delimiters align with their opener. Braces always have a safe
        // preceding grammar boundary; list closers preserve existing line breaks.
        match token.kind {
            LexemeKind::Symbol('}') => {
                braces -= 1;
                output.forced_break = true;
            }
            LexemeKind::Symbol(')' | ']') => lists -= 1,
            _ => {}
        }
        // A trailing comment stays with its opening brace/explicit separator.
        // The next parsed statement/field still forces a newline, so retaining
        // this attachment cannot join otherwise separate executable statements.
        if matches!(
            token.kind,
            LexemeKind::LineComment | LexemeKind::BlockComment
        ) && output.source_breaks == 0
            && output
                .previous
                .is_some_and(|previous| matches!(previous.kind, LexemeKind::Symbol('{' | ';')))
        {
            output.forced_break = false;
        }
        // Preserve the space between a contextual word and a grouped operand.
        // Calls with those names still use tight parentheses.
        while spaced_word < layout.spaced_words.len()
            && layout.spaced_words[spaced_word] < token.span.start
        {
            spaced_word += 1;
        }
        let grouped_operand = token.kind == LexemeKind::Symbol('(')
            && spaced_word != 0
            && output.previous.is_some_and(|previous| {
                previous.span.start == layout.spaced_words[spaced_word - 1]
            });
        output.write(
            source,
            token,
            braces
                + lists
                + usize::from(in_header)
                + active_handlers.len()
                + usize::from(in_destination),
            in_type,
            grouped_operand,
        );
        // Opening braces and explicit statement separators admit a newline;
        // never force one after `}`, which may be followed by a postfix call.
        match token.kind {
            LexemeKind::Symbol('{') => {
                braces += 1;
                output.forced_break = true;
            }
            LexemeKind::Symbol('(' | '[') => lists += 1,
            LexemeKind::Symbol(';') => output.forced_break = true,
            _ => {}
        }
    }
    // One final LF gives files a stable ending; trivia-only files remain empty.
    if !output.text.is_empty() {
        output.text.push('\n');
    }
    output.text
}

impl Output {
    /// Append one source `token` at `indent` levels, flushing pending layout first.
    /// Copies the token's original bytes once. Existing blank lines are bounded
    /// to one, and forced/source boundaries coalesce rather than adding blank lines.
    fn write(
        &mut self,
        source: &str,
        token: Lexeme,
        indent: usize,
        in_type: bool,
        grouped_operand: bool,
    ) {
        let newlines = self.source_breaks.max(usize::from(self.forced_break));
        // Leading blank space disappears; pending real/structural line breaks
        // otherwise end the preceding line before any indentation is emitted.
        if newlines > 0 && !self.text.is_empty() {
            for _ in 0..newlines {
                self.text.push('\n');
            }
            self.previous = None;
        }
        self.source_breaks = 0;
        self.forced_break = false;
        // A new line receives indentation only once; interior comment lines stay
        // byte-for-byte intact rather than being reparsed as source whitespace.
        if self.previous.is_none() {
            for _ in 0..indent * INDENT_WIDTH {
                self.text.push(' ');
            }
        } else if grouped_operand
            || needs_space(
                source,
                self.previous.expect("same-line previous token"),
                token,
                in_type,
            )
        {
            self.text.push(' ');
        }
        self.text
            .push_str(&source[token.span.start..token.span.end]);
        self.previous = Some(token);
    }
}

/// Decide horizontal separation between adjacent emitted lexemes in `source`.
/// Returns no owned text; punctuation remains tight and words/operators separate.
fn needs_space(source: &str, previous: Lexeme, current: Lexeme, in_type: bool) -> bool {
    // Type angle brackets bind to the written type; comparisons elsewhere
    // retain normal operator spacing. A comma still separates type arguments.
    if in_type
        && (matches!(current.kind, LexemeKind::Symbol('<' | '>'))
            || previous.kind == LexemeKind::Symbol('<'))
    {
        return false;
    }
    // Tight postfix/group punctuation has no prefix space, except a grouped
    // condition/value after statement keywords, where a space improves readability.
    match current.kind {
        LexemeKind::Symbol('(') => match previous.kind {
            LexemeKind::Identifier => matches!(
                &source[previous.span.start..previous.span.end],
                "if" | "for" | "return"
            ),
            LexemeKind::Number | LexemeKind::Symbol(')' | ']' | '}') => false,
            _ => true,
        },
        LexemeKind::Symbol('.' | '[' | ')' | ']' | ',' | ';' | ':') => false,
        _ => !matches!(previous.kind, LexemeKind::Symbol('.' | '(' | '[')),
    }
}

/// Identify layout/comment trivia, which does not select a header grammar token.
/// Returns a constant-size predicate without inspecting or allocating source bytes.
fn trivia(kind: LexemeKind) -> bool {
    matches!(
        kind,
        LexemeKind::Whitespace
            | LexemeKind::Newline
            | LexemeKind::LineComment
            | LexemeKind::BlockComment
    )
}

/// Require exact ordered non-whitespace token spelling and comment bytes.
/// Scans both sources without owned token strings; newline semantics are checked
/// independently through parsed tree comparison below.
fn same_tokens(left: &str, right: &str) -> bool {
    let retained =
        |token: &Lexeme| !matches!(token.kind, LexemeKind::Whitespace | LexemeKind::Newline);
    scan(left)
        .filter(retained)
        .map(|token| (token.kind, &left[token.span.start..token.span.end]))
        .eq(scan(right)
            .filter(retained)
            .map(|token| (token.kind, &right[token.span.start..token.span.end])))
}

/// Compare modules without source spans; token comparison also protects interleaving.
/// Borrows both trees, allocates nothing, and returns false at the first difference.
fn same_module(left: &ast::Module, right: &ast::Module) -> bool {
    left.records.len() == right.records.len()
        && left
            .records
            .iter()
            .zip(&right.records)
            .all(|(left, right)| {
                left.name == right.name
                    && left.fields.len() == right.fields.len()
                    && left
                        .fields
                        .iter()
                        .zip(&right.fields)
                        .all(|(left, right)| left.name == right.name && left.ty == right.ty)
            })
        && left.errors.len() == right.errors.len()
        && left.errors.iter().zip(&right.errors).all(|(left, right)| {
            left.name == right.name
                && left.cases.len() == right.cases.len()
                && left.cases.iter().zip(&right.cases).all(|(left, right)| {
                    left.name == right.name
                        && match (&left.payload, &right.payload) {
                            (Some(left), Some(right)) => {
                                left.name == right.name && left.ty == right.ty
                            }
                            (None, None) => true,
                            _ => false,
                        }
                        // A child alias must retain its family source across formatting.
                        && match (&left.source, &right.source) {
                            (Some(left), Some(right)) => same_expression(left, right),
                            (None, None) => true,
                            _ => false,
                        }
                })
        })
        && left.functions.len() == right.functions.len()
        && left
            .functions
            .iter()
            .zip(&right.functions)
            .all(|(left, right)| same_function(left, right))
}

/// Compare function declarations, contracts, argument modes and bodies without spans.
/// Does not resolve symbols or infer effects; source syntax itself is the contract.
fn same_function(left: &ast::Function, right: &ast::Function) -> bool {
    left.name == right.name
        && left.result == right.result
        && left
            .guarantees
            .iter()
            .map(|(name, _)| name)
            .eq(right.guarantees.iter().map(|(name, _)| name))
        && left.args.len() == right.args.len()
        && left.args.iter().zip(&right.args).all(|(left, right)| {
            left.name == right.name
                && left.ty == right.ty
                && left.access == right.access
                && left.passing == right.passing
        })
        && same_statements(&left.body, &right.body)
}

/// Compare ordered bodies recursively, ignoring only source coordinates.
/// The parser bounds recursive expression/statement nesting before this traversal.
fn same_statements(left: &[ast::Statement], right: &[ast::Statement]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            // Matching variants must preserve every name, mode, expression and
            // explicit body; an absent recovery differs from an empty written one.
            match (&left.kind, &right.kind) {
                (
                    ast::StatementKind::Let {
                        name: ln,
                        mutable: lm,
                        writable_view: lw,
                        value: lv,
                        otherwise: lo,
                    },
                    ast::StatementKind::Let {
                        name: rn,
                        mutable: rm,
                        writable_view: rw,
                        value: rv,
                        otherwise: ro,
                    },
                ) => {
                    ln == rn
                        && lm == rm
                        && lw == rw
                        && same_expression(lv, rv)
                        && match (lo, ro) {
                            (Some(left), Some(right)) => same_statements(left, right),
                            (None, None) => true,
                            _ => false,
                        }
                }
                (
                    ast::StatementKind::Assign {
                        target: lt,
                        value: lv,
                    },
                    ast::StatementKind::Assign {
                        target: rt,
                        value: rv,
                    },
                ) => same_expression(lt, rt) && same_expression(lv, rv),
                (ast::StatementKind::Expr(left), ast::StatementKind::Expr(right))
                | (ast::StatementKind::Return(left), ast::StatementKind::Return(right)) => {
                    same_expression(left, right)
                }
                (ast::StatementKind::PrintText(left), ast::StatementKind::PrintText(right)) => {
                    left == right
                }
                (
                    ast::StatementKind::Construct {
                        subject: ls,
                        arms: la,
                    },
                    ast::StatementKind::Construct {
                        subject: rs,
                        arms: ra,
                    },
                )
                | (
                    ast::StatementKind::HandleResult {
                        subject: ls,
                        arms: la,
                    },
                    ast::StatementKind::HandleResult {
                        subject: rs,
                        arms: ra,
                    },
                ) => {
                    same_expression(ls, rs)
                        && la.len() == ra.len()
                        && la.iter().zip(ra).all(|(left, right)| {
                            same_result_pattern(&left.pattern, &right.pattern)
                                && same_statements(&left.body, &right.body)
                        })
                }
                (ast::StatementKind::Scope(left), ast::StatementKind::Scope(right))
                | (ast::StatementKind::Loop(left), ast::StatementKind::Loop(right)) => {
                    same_statements(left, right)
                }
                (
                    ast::StatementKind::WithRegion {
                        capacity: lc,
                        body: lb,
                        on_failure: lf,
                    },
                    ast::StatementKind::WithRegion {
                        capacity: rc,
                        body: rb,
                        on_failure: rf,
                    },
                ) => same_expression(lc, rc) && same_statements(lb, rb) && same_statements(lf, rf),
                (
                    ast::StatementKind::TcpScope {
                        stream: left_stream,
                        body: left_body,
                    },
                    ast::StatementKind::TcpScope {
                        stream: right_stream,
                        body: right_body,
                    },
                ) => left_stream == right_stream && same_statements(left_body, right_body),
                (
                    ast::StatementKind::If {
                        condition: lc,
                        then_body: lt,
                        else_body: le,
                    },
                    ast::StatementKind::If {
                        condition: rc,
                        then_body: rt,
                        else_body: re,
                    },
                ) => same_expression(lc, rc) && same_statements(lt, rt) && same_statements(le, re),
                (
                    ast::StatementKind::For {
                        name: ln,
                        access: la,
                        iterable: li,
                        body: lb,
                    },
                    ast::StatementKind::For {
                        name: rn,
                        access: ra,
                        iterable: ri,
                        body: rb,
                    },
                ) => ln == rn && la == ra && same_expression(li, ri) && same_statements(lb, rb),
                (
                    ast::StatementKind::Refill {
                        slot: left_slot,
                        limit: left_limit,
                        stream: left_stream,
                        data_name: left_data,
                        data_body: left_data_body,
                        status_name: left_status,
                        other_body: left_other_body,
                    },
                    ast::StatementKind::Refill {
                        slot: right_slot,
                        limit: right_limit,
                        stream: right_stream,
                        data_name: right_data,
                        data_body: right_data_body,
                        status_name: right_status,
                        other_body: right_other_body,
                    },
                ) => {
                    // The syntax tool preserves the owner operand and both
                    // outcome scopes independently of source range changes.
                    left_data == right_data
                        && left_status == right_status
                        && left_stream.as_ref().map(|(name, _)| name)
                            == right_stream.as_ref().map(|(name, _)| name)
                        && same_expression(left_slot, right_slot)
                        && same_expression(left_limit, right_limit)
                        && same_statements(left_data_body, right_data_body)
                        && same_statements(left_other_body, right_other_body)
                }
                _ => false,
            }
        })
}

/// Compare expression trees, preserving call order, grouping effects, and operators.
/// Borrows all strings/children and returns false for every unmatched syntax variant.
fn same_expression(left: &ast::Expr, right: &ast::Expr) -> bool {
    // Parentheses are represented by their binding effect on the tree; exact
    // parenthesis spelling is independently protected by retained-token comparison.
    match (&left.kind, &right.kind) {
        (ast::ExprKind::Number(left), ast::ExprKind::Number(right)) => left == right,
        (ast::ExprKind::Bool(left), ast::ExprKind::Bool(right)) => left == right,
        (ast::ExprKind::Bytes(left), ast::ExprKind::Bytes(right)) => left == right,
        (ast::ExprKind::Name(left), ast::ExprKind::Name(right)) => left == right,
        (ast::ExprKind::Ok(left), ast::ExprKind::Ok(right))
        | (ast::ExprKind::Try(left), ast::ExprKind::Try(right))
        | (ast::ExprKind::Not { value: left, .. }, ast::ExprKind::Not { value: right, .. }) => {
            same_expression(left, right)
        }
        (
            ast::ExprKind::Error {
                case: left,
                payload: lp,
            },
            ast::ExprKind::Error {
                case: right,
                payload: rp,
            },
        ) => {
            same_expression(left, right)
                && match (lp, rp) {
                    (Some(left), Some(right)) => same_expression(left, right),
                    (None, None) => true,
                    _ => false,
                }
        }
        (
            ast::ExprKind::Handle {
                subject: ls,
                arms: la,
            },
            ast::ExprKind::Handle {
                subject: rs,
                arms: ra,
            },
        ) => {
            same_expression(ls, rs)
                && la.len() == ra.len()
                && la.iter().zip(ra).all(|(left, right)| {
                    same_result_pattern(&left.pattern, &right.pattern)
                        && same_expression(&left.value, &right.value)
                })
        }
        (
            ast::ExprKind::Call {
                callee: lc,
                args: la,
            },
            ast::ExprKind::Call {
                callee: rc,
                args: ra,
            },
        ) => {
            same_expression(lc, rc)
                && la.len() == ra.len()
                && la
                    .iter()
                    .zip(ra)
                    .all(|(left, right)| same_expression(left, right))
        }
        (
            ast::ExprKind::Member {
                base: lb,
                field: lf,
            },
            ast::ExprKind::Member {
                base: rb,
                field: rf,
            },
        ) => lf == rf && same_expression(lb, rb),
        (
            ast::ExprKind::Index {
                base: lb,
                index: li,
            },
            ast::ExprKind::Index {
                base: rb,
                index: ri,
            },
        ) => same_expression(lb, rb) && same_expression(li, ri),
        (
            ast::ExprKind::Record {
                name: ln,
                fields: lf,
                indented: li,
            },
            ast::ExprKind::Record {
                name: rn,
                fields: rf,
                indented: ri,
            },
        ) => {
            ln == rn
                && li == ri
                && lf.len() == rf.len()
                && lf.iter().zip(rf).all(|(left, right)| {
                    left.name == right.name && same_expression(&left.value, &right.value)
                })
        }
        (
            ast::ExprKind::Binary {
                op: lo,
                left: ll,
                right: lr,
                ..
            },
            ast::ExprKind::Binary {
                op: ro,
                left: rl,
                right: rr,
                ..
            },
        ) => lo == ro && same_expression(ll, rl) && same_expression(lr, rr),
        _ => false,
    }
}

/// Compare arm patterns without changing or resolving their case identities.
/// Optional binding names and all written qualifiers must survive formatting.
fn same_result_pattern(left: &ast::ResultPattern, right: &ast::ResultPattern) -> bool {
    // A fallback and success are distinct even if their local spellings agree.
    match (left, right) {
        (ast::ResultPattern::Ok(left), ast::ResultPattern::Ok(right))
        | (ast::ResultPattern::Fallback(left), ast::ResultPattern::Fallback(right)) => {
            left == right
        }
        (
            ast::ResultPattern::Error {
                case: left,
                binding: lb,
            },
            ast::ResultPattern::Error {
                case: right,
                binding: rb,
            },
        )
        | (
            ast::ResultPattern::Rejected {
                case: left,
                binding: lb,
            },
            ast::ResultPattern::Rejected {
                case: right,
                binding: rb,
            },
        ) => lb == rb && same_expression(left, right),
        _ => false,
    }
}
