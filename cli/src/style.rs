//! Compile a small CSS-like terminal stylesheet into a fixed category table.
//!
//! Styles are parsed once per invocation, outside the source-rendering loop.
//! Selectors map to syntax categories and terminal chrome, never source semantics.
//! The supported properties are deliberately finite: RGB colour and ANSI emphasis.

use holo::highlight::HighlightKind;

/// Built-in palette, maintained as editable style data rather than Rust branches.
const DEFAULT: &str = include_str!("../styles/aurora.hss");
/// Category count plus filename header, line-number gutter, and frame border.
const COUNT: usize = HighlightKind::ALL.len() + 3;
/// Filename/header category in the compiled style table.
pub const HEADER: usize = HighlightKind::ALL.len();
/// Line-number and secondary-information category in the compiled style table.
pub const GUTTER: usize = HEADER + 1;
/// Frame and separator category in the compiled style table.
pub const BORDER: usize = GUTTER + 1;

/// Precompiled ANSI prefixes indexed directly by a syntax/chrome category.
pub struct Styles {
    /// One owned prefix per category; rendering only borrows these strings.
    prefixes: [String; COUNT],
}

impl Styles {
    /// Compile the bundled stylesheet and an optional overriding stylesheet.
    /// Unknown selectors/properties or malformed values fail before source output.
    pub fn load(overrides: Option<&str>) -> Result<Self, String> {
        let mut rules = [Rule::default(); COUNT];
        apply(DEFAULT, &mut rules)?;
        // Custom declarations cascade over defaults; omitted properties retain their values.
        if let Some(source) = overrides {
            apply(source, &mut rules)?;
        }
        Ok(Self {
            prefixes: std::array::from_fn(|index| rules[index].ansi()),
        })
    }

    /// Borrow a precomputed prefix; category indices are internal and always in range.
    pub fn prefix(&self, index: usize) -> &str {
        &self.prefixes[index]
    }
}

/// Finite terminal style properties, independent of any particular source file.
#[derive(Clone, Copy, Default)]
struct Rule {
    /// Optional RGB foreground; absence preserves the terminal's foreground.
    color: Option<[u8; 3]>,
    /// Request bold text, subject to terminal support.
    bold: bool,
    /// Request italic text, subject to terminal support.
    italic: bool,
    /// Request dim text, subject to terminal support.
    dim: bool,
    /// Request a single underline, subject to terminal support.
    underline: bool,
}

impl Rule {
    /// Encode this style once as an ANSI SGR prefix; an empty rule needs no prefix.
    /// Allocates only the short compiled style string, independent of source size.
    fn ansi(self) -> String {
        let mut codes = Vec::with_capacity(5);
        // Each independent flag contributes a standard SGR attribute only when enabled.
        for (enabled, code) in [
            (self.bold, "1"),
            (self.dim, "2"),
            (self.italic, "3"),
            (self.underline, "4"),
        ] {
            if enabled {
                codes.push(code.to_owned());
            }
        }
        // Truecolour is explicit palette data; terminals may approximate unsupported RGB.
        if let Some([red, green, blue]) = self.color {
            codes.push(format!("38;2;{red};{green};{blue}"));
        }
        // Empty declarations inherit terminal defaults without emitting a redundant escape.
        if codes.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", codes.join(";"))
        }
    }
}

/// Remove stylesheet comments and apply bounded rule blocks in written order.
/// Allocates one theme-sized scratch string, never a source-sized style structure.
fn apply(source: &str, rules: &mut [Rule; COUNT]) -> Result<(), String> {
    let mut clean = String::with_capacity(source.len());
    let mut remaining = source;
    // Each comment removes a finite prefix; unterminated comments are configuration errors.
    while let Some(start) = remaining.find("/*") {
        clean.push_str(&remaining[..start]);
        let tail = &remaining[start + 2..];
        let end = tail.find("*/").ok_or("unterminated stylesheet comment")?;
        clean.push(' ');
        remaining = &tail[end + 2..];
    }
    clean.push_str(remaining);
    remaining = clean.trim();
    // Each block advances beyond its closing brace; nested blocks are unsupported and reject.
    while !remaining.is_empty() {
        let open = remaining.find('{').ok_or("stylesheet rule requires `{`")?;
        let selector = remaining[..open].trim();
        let index = selector
            .strip_prefix('.')
            .and_then(selector_index)
            .ok_or_else(|| format!("unknown stylesheet selector {selector:?}"))?;
        let tail = &remaining[open + 1..];
        let close = tail.find('}').ok_or("stylesheet rule requires `}`")?;
        // Empty declarations are harmless; every nonempty declaration needs a known property.
        for declaration in tail[..close].split(';').map(str::trim) {
            if declaration.is_empty() {
                continue;
            }
            let (property, value) = declaration
                .split_once(':')
                .ok_or_else(|| format!("invalid stylesheet declaration {declaration:?}"))?;
            set_property(&mut rules[index], property.trim(), value.trim())?;
        }
        remaining = tail[close + 1..].trim();
    }
    Ok(())
}

/// Map a documented selector to its fixed style-table slot, without allocation.
fn selector_index(name: &str) -> Option<usize> {
    // Chrome occupies dedicated slots; all other names belong to the shared lexical vocabulary.
    match name {
        "header" => Some(HEADER),
        "gutter" => Some(GUTTER),
        "border" => Some(BORDER),
        _ => HighlightKind::from_name(name).map(|kind| kind as usize),
    }
}

/// Validate and update one property, rejecting unsupported CSS instead of ignoring it.
/// Mutates only one small rule; error text identifies the offending property/value.
fn set_property(rule: &mut Rule, property: &str, value: &str) -> Result<(), String> {
    // Each property has a closed vocabulary, keeping themes predictable and rendering constant-cost.
    match property {
        "color" => {
            rule.color = if value == "default" {
                None
            } else {
                let hex = value
                    .strip_prefix('#')
                    .filter(|hex| hex.len() == 6 && hex.is_ascii())
                    .ok_or("color requires #RRGGBB or default")?;
                let mut rgb = [0; 3];
                // Exactly three checked byte pairs encode RGB, preventing injected terminal escapes.
                for (index, channel) in rgb.iter_mut().enumerate() {
                    *channel = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                        .map_err(|_| "color requires hexadecimal #RRGGBB")?;
                }
                Some(rgb)
            };
        }
        "font-weight" => rule.bold = choice(value, "bold", "normal")?,
        "font-style" => rule.italic = choice(value, "italic", "normal")?,
        "text-decoration" => rule.underline = choice(value, "underline", "none")?,
        "dim" => rule.dim = choice(value, "true", "false")?,
        _ => return Err(format!("unsupported stylesheet property {property:?}")),
    }
    Ok(())
}

/// Parse one two-value property without accepting misspellings or arbitrary SGR text.
fn choice(value: &str, enabled: &str, disabled: &str) -> Result<bool, String> {
    // Only the two documented spellings are meaningful; every other value rejects the theme.
    if value == enabled {
        Ok(true)
    } else if value == disabled {
        Ok(false)
    } else {
        Err(format!(
            "expected {enabled:?} or {disabled:?}, found {value:?}"
        ))
    }
}
