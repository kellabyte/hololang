//! Byte-span diagnostics shared by the experimental lexer, parser, and lowering.
//!
//! Spans index immutable UTF-8 input rather than copying source fragments. The
//! renderer defensively clamps externally constructed spans to character
//! boundaries. Reported columns count Unicode scalar values; display tabs expand
//! to four spaces. Terminal control scalars in rendered text become visible
//! escapes. These are compiler diagnostics, with no generated-code cost.

use std::fmt::{self, Write};

/// Half-open byte range in one immutable source string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// First included UTF-8 byte offset.
    pub start: usize,
    /// First excluded byte offset; equal endpoints represent a missing token/EOF.
    pub end: usize,
}

/// One source diagnostic, retaining a precise range and an owned explanation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Source bytes associated with the diagnostic.
    pub span: Span,
    /// Human-readable explanation.
    pub message: String,
}

impl Diagnostic {
    /// Own `message` for `span` without copying source text or mutating input.
    /// The renderer handles invalid caller-supplied offsets defensively.
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }

    /// Render this failure with `path`, scalar line/column, and its first source line.
    /// Returns owned text with escaped path, message, and excerpt controls. Carets
    /// follow the displayed excerpt and mark EOF/missing tokens at least once.
    /// Inputs stay unchanged and every source slice uses UTF-8 boundaries.
    pub fn render(&self, path: &str, source: &str) -> String {
        self.render_with_severity(path, source, "error")
    }

    /// Render an advisory diagnostic with the same location and escaping as an error.
    /// Returns owned output without changing the diagnostic or the source.
    pub fn render_warning(&self, path: &str, source: &str) -> String {
        self.render_with_severity(path, source, "warning")
    }

    /// Render an original source location with the caller's static severity label.
    /// The returned string owns the excerpt, message, and visible caret range.
    fn render_with_severity(&self, path: &str, source: &str, severity: &str) -> String {
        let start = floor_boundary(source, self.span.start.min(source.len()));
        let end = ceil_boundary(source, self.span.end.min(source.len()).max(start));
        let prefix = &source[..start];
        let line_number = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let line_start = prefix.rfind('\n').map_or(0, |offset| offset + 1);
        let raw_end = source[start..]
            .find('\n')
            .map_or(source.len(), |offset| start + offset);
        // CR belongs to a CRLF delimiter, rather than the visible source excerpt.
        let line_end = if raw_end < source.len()
            && raw_end > line_start
            && source.as_bytes()[raw_end - 1] == b'\r'
        {
            raw_end - 1
        } else {
            raw_end
        };
        let shown_start = start.min(line_end);
        let shown_end = end.min(line_end).max(shown_start);
        let column = source[line_start..shown_start].chars().count() + 1;
        let gutter = line_number.to_string().len();
        let mut output = String::new();
        append_visible(&mut output, path, false);
        write!(output, ":{line_number}:{column}: {severity}: ").expect("formatting into String");
        append_visible(&mut output, &self.message, false);
        write!(output, "\n{:gutter$} |\n{line_number} | ", "").expect("formatting into String");

        // Count transformed widths in the same pass that displays the source.
        let before = append_visible(&mut output, &source[line_start..shown_start], true);
        let width = append_visible(&mut output, &source[shown_start..shown_end], true);
        append_visible(&mut output, &source[shown_end..line_end], true);
        write!(output, "\n{:gutter$} | ", "").expect("formatting into String");
        output.extend(std::iter::repeat_n(' ', before));
        output.extend(std::iter::repeat_n('^', width.max(1)));
        output.push('\n');
        output
    }
}

impl fmt::Display for Diagnostic {
    /// Write the stored explanation as-is for callers that do their own rendering.
    /// Returns the formatter's I/O result without modifying the diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Diagnostic {}

/// Find the nearest character boundary at/before an in-range byte `offset`.
/// This moves only across one UTF-8 sequence and allocates nothing.
fn floor_boundary(source: &str, mut offset: usize) -> usize {
    // A malformed external span may point to a continuation byte; walk to its lead.
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Find the nearest character boundary at/after an in-range byte `offset`.
/// This includes the complete highlighted scalar and allocates nothing.
fn ceil_boundary(source: &str, mut offset: usize) -> usize {
    // End offsets inside a scalar include that scalar instead of truncating UTF-8.
    while !source.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

/// Append visible text and return its caret-display width in one pass.
/// Only source excerpts expand tabs. Printable scalars count once, while every
/// character of an ASCII escape occupies a column. Only `output` can grow.
fn append_visible(output: &mut String, text: &str, expand_tabs: bool) -> usize {
    let mut width = 0;
    // Every scalar is preserved, expanded, or escaped before it reaches output.
    for character in text.chars() {
        // Excerpt tabs use the existing four-column convention. In a path or
        // message they fall through to the escape branch with other controls.
        if expand_tabs && character == '\t' {
            output.push_str("    ");
            width += 4;
        } else if character.is_control() {
            let start = output.len();
            // C0 and DEL use two-digit byte escapes. C1 controls use their
            // Unicode scalar spelling, distinct from their UTF-8 bytes.
            let written = if character.is_ascii() {
                write!(output, "\\x{:02x}", u32::from(character))
            } else {
                write!(output, "{}", character.escape_unicode())
            };
            written.expect("formatting into String");
            width += output.len() - start;
        } else {
            output.push(character);
            width += 1;
        }
    }
    width
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, Span};

    /// Check metadata and excerpt escapes, preserving scalar columns and caret widths.
    #[test]
    fn contextual_render_escapes_controls_and_aligns_carets() {
        let source = "ignored\r\n\t\x1bé\u{009b}\x07z\r\n";
        let start = source.find('\u{009b}').unwrap();
        let end = source.find('z').unwrap();
        let diagnostic = Diagnostic::new(Span { start, end }, "bad\r\x08\u{009b}");

        assert_eq!(
            diagnostic.render("a\x1b\n\t.holo", source),
            concat!(
                "a\\x1b\\x0a\\x09.holo:2:4: error: bad\\x0d\\x08\\u{9b}\n",
                "  |\n",
                "2 |     \\x1bé\\u{9b}\\x07z\n",
                "  |          ^^^^^^^^^^\n",
            ),
        );
    }

    /// A lone carriage return is input text, and EOF still receives one caret.
    #[test]
    fn contextual_render_displays_carriage_return_at_eof() {
        let diagnostic = Diagnostic::new(Span { start: 2, end: 2 }, "end");

        assert_eq!(
            diagnostic.render("file.holo", "x\r"),
            "file.holo:1:3: error: end\n  |\n1 | x\\x0d\n  |      ^\n",
        );
    }
}
