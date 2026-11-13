//! Shared source acquisition for text tools, independent of native compilation.
//!
//! Each invocation owns one UTF-8 input buffer. Paths and stdin remain explicit;
//! source analysis borrows that buffer and never opens a compiler toolchain.

use std::io::{self, Read};
use std::path::PathBuf;

/// One explicitly selected input stream; file ownership remains with the caller.
pub enum SourceInput {
    /// A source file read once before analysis.
    File(PathBuf),
    /// Standard input, consumed through EOF.
    Stdin,
}

impl SourceInput {
    /// Interpret a UTF-8 positional path; `-` explicitly selects standard input.
    /// This owns only path metadata and performs no I/O.
    pub fn path(value: String) -> Self {
        // Dash is the conventional stream operand; all other strings are literal paths.
        if value == "-" {
            Self::Stdin
        } else {
            Self::File(PathBuf::from(value))
        }
    }

    /// Borrow the display name used by headers and source diagnostics, without allocation.
    pub fn label(&self) -> &str {
        // CLI argument validation establishes UTF-8 paths before acquisition.
        match self {
            Self::File(path) => path.to_str().expect("validated UTF-8 path"),
            Self::Stdin => "<stdin>",
        }
    }

    /// Read a single UTF-8 source buffer, preserving newlines and literal bytes.
    /// Returns contextual I/O/encoding errors; no partial source reaches analysis.
    pub fn read(&self) -> Result<String, String> {
        // Both acquisition paths have the same UTF-8 contract and own one input buffer.
        match self {
            Self::File(path) => std::fs::read_to_string(path)
                .map_err(|error| format!("read UTF-8 source {}: {error}", path.display())),
            Self::Stdin => {
                let mut source = String::new();
                io::stdin()
                    .lock()
                    .read_to_string(&mut source)
                    .map_err(|error| format!("read UTF-8 stdin: {error}"))?;
                Ok(source)
            }
        }
    }
}

/// Record one input operand, rejecting ambiguity before any source is read.
/// Takes ownership of `value`; a successful call mutates only `input`.
pub fn set(input: &mut Option<SourceInput>, value: SourceInput) -> Result<(), String> {
    // Multiple sources would make output ownership and editor stdin contracts ambiguous.
    if input.is_some() {
        return Err("expected exactly one source path or --stdin".into());
    }
    *input = Some(value);
    Ok(())
}

/// Convert OS arguments to the text tools' UTF-8 contract before interpreting options.
/// Collects invocation-sized metadata and returns an encoding error without I/O.
pub fn arguments() -> Result<Vec<String>, String> {
    std::env::args_os()
        .skip(2)
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| "text-tool arguments and paths must be valid UTF-8".into())
        })
        .collect()
}
