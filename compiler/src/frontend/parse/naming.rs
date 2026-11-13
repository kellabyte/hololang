//! Advisory spelling checks for declared ASCII identifiers.
//!
//! The parser supplies the exact token span after identifying a declaration's
//! role. Checking neither resolves symbols nor rewrites the AST. Conforming
//! names need a bounded number of byte scans and no extra allocation.

use std::fmt::Write;

use crate::source::{Diagnostic, Span};

/// Declaration or binding roles with a naming convention in the current grammar.
#[derive(Clone, Copy)]
pub(super) enum NameKind {
    /// A written component of a source namespace header.
    Namespace,
    /// A private or public function declaration.
    Function,
    /// A named argument in a function header.
    Parameter,
    /// A named struct declaration.
    Record,
    /// A named error family declaration.
    ErrorFamily,
    /// A case declared directly inside an error family.
    ErrorCase,
    /// A case naming a child error family.
    ChildAlias,
    /// A record field or a named error payload.
    Field,
    /// A local binding in a statement or outcome arm.
    Binding,
}

impl NameKind {
    /// Check the convention for `name` and own a diagnostic over `span` on failure.
    /// Names and spans remain unchanged. A suggestion is advisory text only.
    pub(super) fn warning(self, name: &str, span: Span) -> Option<Diagnostic> {
        // Outcome discards and potential special prefixes belong to separate
        // language rules, so this pass does not interpret leading underscores.
        if name.starts_with('_') {
            return None;
        }
        // Named types and their variants use capitals. All current lexical
        // binding and namespace roles use lowercase words separated by underscores.
        let (role, style, upper) = match self {
            Self::Namespace => ("Namespace component", "snake_case", false),
            Self::Function => ("Function", "snake_case", false),
            Self::Parameter => ("Parameter", "snake_case", false),
            Self::Record => ("Record", "PascalCase", true),
            Self::ErrorFamily => ("Error family", "PascalCase", true),
            Self::ErrorCase => ("Error case", "PascalCase", true),
            Self::ChildAlias => ("Child error alias", "PascalCase", true),
            Self::Field => ("Field", "snake_case", false),
            Self::Binding => ("Binding", "snake_case", false),
        };
        let bytes = name.as_bytes();
        let (first, rest) = bytes.split_first()?;
        // Choose an initial case and check the suffix under that convention.
        let (expected_first, valid_rest) = if upper {
            (first.to_ascii_uppercase(), pascal_suffix(rest))
        } else {
            (first.to_ascii_lowercase(), snake_suffix(rest))
        };
        // Successful checks leave the optional diagnostics buffer untouched.
        if expected_first == *first && valid_rest {
            return None;
        }
        let mut message = format!("{role} name `{name}` should use {style}.");
        // Change only the initial case when that suffices. For lowercase words
        // also recognize simple camel-case boundaries, with no acronym guesses.
        if valid_rest {
            write!(
                message,
                " Consider `{}{}`.",
                char::from(expected_first),
                &name[1..]
            )
            .expect("formatting into String");
        } else if !upper && let Some(suggestion) = camel_to_snake(bytes) {
            write!(message, " Consider `{suggestion}`.").expect("formatting into String");
        }
        Some(Diagnostic::new(span, message))
    }
}

/// Check lowercase identifier bytes following the first letter without copying.
/// An underscore separates nonempty segments, and digits stay with their segment.
fn snake_suffix(rest: &[u8]) -> bool {
    let mut separator = false;
    // Every byte extends the current segment or opens one new segment.
    for &byte in rest {
        // Reject uppercase letters and empty segments while scanning the suffix.
        if byte == b'_' && !separator {
            separator = true;
        } else if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            separator = false;
        } else {
            return false;
        }
    }
    !separator
}

/// Check identifier bytes after an assumed uppercase initial without allocating.
/// Diagnose clear capital runs without guessing ambiguous word boundaries.
fn pascal_suffix(rest: &[u8]) -> bool {
    let mut run = 1;
    // Three adjacent capitals imply at least a two-letter all-capitals word.
    // Two can be a single-letter word followed by a new word, as in XRay.
    for &byte in rest {
        // A lowercase byte starts or extends a word. A digit or the end of
        // the spelling cannot justify a two-capital boundary.
        if byte.is_ascii_uppercase() {
            run += 1;
            if run == 3 {
                return false;
            }
        } else if byte.is_ascii_lowercase() || (byte.is_ascii_digit() && run < 2) {
            run = 0;
        } else {
            return false;
        }
    }
    run < 2
}

/// Convert a single unseparated spelling with clear camel-case boundaries.
/// Mixed separators and consecutive capitals leave the broader spelling open.
/// Allocates only for the advisory spelling when each boundary is mechanical.
fn camel_to_snake(bytes: &[u8]) -> Option<String> {
    // An existing underscore mixed with capitals or a run of capitals can
    // encode multiple word-boundary choices. Leave those to the author.
    if bytes.contains(&b'_')
        || bytes
            .windows(2)
            .any(|pair| pair[0].is_ascii_uppercase() && pair[1].is_ascii_uppercase())
    {
        return None;
    }
    let (first, rest) = bytes.split_first()?;
    let mut suggestion = String::with_capacity(bytes.len());
    suggestion.push(char::from(first.to_ascii_lowercase()));
    // Each single capital starts a new lowercase segment; all other ASCII
    // letters and digits retain their position.
    for &byte in rest {
        // The initial letter already starts the first segment.
        if byte.is_ascii_uppercase() {
            suggestion.push('_');
        }
        suggestion.push(char::from(byte.to_ascii_lowercase()));
    }
    Some(suggestion)
}

#[cfg(test)]
/// Syntax-level checks for declaration sites, spelling and optional collection.
mod tests {
    use super::{NameKind, Span};
    use crate::frontend::parse::{for_format, package_library_with_naming};

    /// Keep conventional initialisms, digits, and ambiguous single-letter words.
    /// Suggest a spelling only when letter-case boundaries make it mechanical.
    #[test]
    fn naming_advice_uses_mechanical_boundaries() {
        let span = Span { start: 0, end: 1 };
        // The accepted examples retain their compact numeric terms and normal
        // word capitals without needing a dictionary of known acronyms.
        for spelling in [
            "HttpClient",
            "IoError",
            "Utf8Decoder",
            "Ipv6Address",
            "XRay",
        ] {
            assert!(NameKind::Record.warning(spelling, span).is_none());
        }
        // An all-capital word warns even when adjacent to another word.
        for spelling in ["HTTPClient", "IOError", "UrlID"] {
            let warning = NameKind::Record
                .warning(spelling, span)
                .expect("PascalCase");
            assert!(!warning.message.contains("Consider"));
        }
        // The lowercase convention keeps existing digit terms and defers
        // prefix policy, including the explicit outcome discard.
        for spelling in ["utf8_decoder", "ipv6_address", "_", "_ignoredValue"] {
            assert!(NameKind::Binding.warning(spelling, span).is_none());
        }
        let warning = NameKind::Function
            .warning("requestCount", span)
            .expect("snake_case");
        assert_eq!(warning.span, span);
        assert!(warning.message.contains("Consider `request_count`"));
        let warning = NameKind::Function
            .warning("parseHTTP", span)
            .expect("snake_case");
        assert!(!warning.message.contains("Consider"));
    }

    /// Verify exact name spans at a file offset and distinguish declared
    /// names from qualified references. Formatting has the same syntax tree.
    #[test]
    fn naming_collection_uses_declaration_spans_and_preserves_spelling() {
        let source = "namespace demo_.frame__tools
using demo.ref__only
struct frame_record { badField u64 }
error family_error {
    badCase(BadPayload u64)
    from ref__only.child_error as badAlias
}
public func requestCount
    arg InputData frame_record as readonly copy
    returns u64
{
    var localCount = InputData.badField
    for LoopIndex in 1 {
        refill InputData.badField, 8 as ReadBytes { return 1 }
        else ReadStatus { return 2 }
    }
    let resultValue = RequestCount()
        on ok(OkValue) then OkValue
        on badAlias.badCase(_) then 2
        on error(BadReason) then BadReason
    return resultValue
}";
        let offset = 37;
        let (library, warnings) =
            package_library_with_naming(source, offset, true).expect("library syntax");
        let names = warnings
            .iter()
            .map(|warning| &source[warning.span.start - offset..warning.span.end - offset])
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "demo_",
                "frame__tools",
                "frame_record",
                "badField",
                "family_error",
                "badCase",
                "BadPayload",
                "badAlias",
                "requestCount",
                "InputData",
                "localCount",
                "LoopIndex",
                "ReadBytes",
                "ReadStatus",
                "resultValue",
                "OkValue",
                "BadReason",
            ]
        );
        let (silent, warnings) =
            package_library_with_naming(source, offset, false).expect("same grammar");
        assert!(warnings.is_empty());
        assert_eq!(
            silent.module.functions[0].name,
            library.module.functions[0].name
        );
        assert_eq!(
            for_format(source).unwrap().functions[0].name,
            "requestCount"
        );
    }
}
