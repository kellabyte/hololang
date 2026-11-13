//! Buffered terminal presentation for the shared borrowed-span highlighter.
//!
//! The compiler classifies source; this adapter owns ANSI, headers and gutters.
//! Styles compile once and source is streamed without a token-sized output tree.
//! Plain redirected output preserves bytes; terminal display escapes control text.

use crate::source_input::{self, SourceInput};
use crate::style::{self, Styles};
use holo::highlight;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

/// Text-tool help, independent of the experimental native compiler invocation.
const HELP: &str = "Usage: holo highlight <source.holo|-> [options]
       holo highlight --stdin [options]

  --color <auto|always|never>  ANSI policy (also --color=value)
  --header / --no-header     Filename and language header
  -n, --line-numbers         Number source lines
  --no-line-numbers          Hide line numbers
  --frame                   Compact left-edge frame
  --plain                   Hide header, line numbers and frame
  --style <path.hss>         Override the bundled CSS-like palette
  --stdin                   Read UTF-8 source from standard input
  --help                    Show this help

Terminals default to a header and line numbers; pipes default to plain source.
NO_COLOR disables automatic colour; --color=always explicitly overrides it.
Highlighting tolerates unfinished source and needs no LLVM installation.";

/// Validated colour policy, resolved once against the output stream and environment.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    /// Use colour only for an eligible terminal without NO_COLOR or TERM=dumb.
    Auto,
    /// Explicitly request ANSI even when redirected or NO_COLOR is set.
    Always,
    /// Emit no ANSI sequences.
    Never,
}

/// Validated presentation options; source text and style files are acquired later.
struct Options {
    /// Exactly one source file or input stream.
    input: SourceInput,
    /// Automatic or explicitly selected terminal colour policy.
    color: Color,
    /// Optional explicit header preference, otherwise determined by terminal detection.
    header: Option<bool>,
    /// Optional explicit gutter preference, otherwise determined by terminal detection.
    numbers: Option<bool>,
    /// Whether a left-edge frame surrounds the source presentation.
    frame: bool,
    /// Whether terminal decoration defaults are suppressed.
    plain: bool,
    /// Optional custom stylesheet path, loaded before any output.
    stylesheet: Option<String>,
}

/// Parse, acquire, and render one highlighted source; malformed options produce no source output.
/// Uses one owned source buffer and a 64 KiB output buffer; broken pipes are normal completion.
pub fn run() -> Result<ExitCode, String> {
    let Some(options) = parse(source_input::arguments()?)? else {
        println!("{HELP}");
        return Ok(ExitCode::SUCCESS);
    };
    let custom = options
        .stylesheet
        .as_ref()
        .map(|path| {
            std::fs::read_to_string(path)
                .map_err(|error| format!("read stylesheet {path}: {error}"))
        })
        .transpose()?;
    let styles = Styles::load(custom.as_deref())?;
    let source = options.input.read()?;
    let stdout = io::stdout();
    let terminal = stdout.is_terminal();
    // Explicit colour takes precedence; automatic colour respects conventional opt-outs.
    let color = match options.color {
        Color::Always => true,
        Color::Never => false,
        Color::Auto => {
            terminal
                && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
                && std::env::var_os("TERM").is_none_or(|value| value != "dumb")
        }
    };
    let header = !options.plain && options.header.unwrap_or(terminal);
    let numbers = !options.plain && options.numbers.unwrap_or(terminal);
    let frame = !options.plain && options.frame;
    let mut output = BufWriter::with_capacity(64 * 1024, stdout.lock());
    let result = render(
        &mut output,
        &source,
        options.input.label(),
        &styles,
        Display {
            color,
            header,
            numbers,
            frame,
            sanitize: terminal || color || header || numbers || frame,
        },
    )
    .and_then(|()| output.flush());
    // A downstream pager/head can close early; other output failures remain visible.
    match result {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(ExitCode::SUCCESS),
        Err(error) => Err(format!("write highlighted source: {error}")),
    }
}

/// Validate the finite highlight option vocabulary before reading files or stdin.
/// A standalone help request returns None; conflicting or repeated options reject.
fn parse(arguments: Vec<String>) -> Result<Option<Options>, String> {
    // Help stands alone so misspelled or extra arguments cannot disappear silently.
    if arguments == ["--help"] || arguments == ["-h"] {
        return Ok(None);
    }
    let mut input = None;
    let mut color = None;
    let mut header = None;
    let mut numbers = None;
    let mut frame = false;
    let mut plain = false;
    let mut stylesheet = None;
    let mut positional = false;
    let mut pending = arguments.into_iter();
    // Each branch consumes the current option and at most one value, bounding argument work.
    while let Some(argument) = pending.next() {
        match argument.as_str() {
            "--" if !positional => positional = true,
            "--stdin" if !positional => source_input::set(&mut input, SourceInput::Stdin)?,
            "--header" if !positional => unique(&mut header, true, "header preference")?,
            "--no-header" if !positional => unique(&mut header, false, "header preference")?,
            "--line-numbers" | "-n" if !positional => {
                unique(&mut numbers, true, "line-number preference")?;
            }
            "--no-line-numbers" if !positional => {
                unique(&mut numbers, false, "line-number preference")?;
            }
            "--frame" if !positional => crate::set_flag(&mut frame, "--frame")?,
            "--plain" if !positional => crate::set_flag(&mut plain, "--plain")?,
            "--style" if !positional => unique(
                &mut stylesheet,
                pending.next().ok_or("--style requires a path")?,
                "--style",
            )?,
            _ if !positional && (argument == "--color" || argument.starts_with("--color=")) => {
                let value = argument
                    .strip_prefix("--color=")
                    .map(str::to_owned)
                    .or_else(|| pending.next())
                    .ok_or("--color requires auto, always, or never")?;
                let policy = match value.as_str() {
                    "auto" => Color::Auto,
                    "always" => Color::Always,
                    "never" => Color::Never,
                    _ => return Err("--color requires auto, always, or never".into()),
                };
                unique(&mut color, policy, "--color")?;
            }
            _ => {
                // A literal dash selects stdin; other leading-dash paths require --.
                if !positional && argument != "-" && argument.starts_with('-') {
                    return Err(format!("unknown highlight option {argument:?}"));
                }
                source_input::set(&mut input, SourceInput::path(argument))?;
            }
        }
    }
    // Contradictory display requests are rejected rather than resolved by argument order.
    if plain && (header == Some(true) || numbers == Some(true) || frame) {
        return Err("--plain conflicts with --header, --line-numbers, and --frame".into());
    }
    Ok(Some(Options {
        input: input.ok_or_else(|| format!("expected a source path or --stdin\n{HELP}"))?,
        color: color.unwrap_or(Color::Auto),
        header,
        numbers,
        frame,
        plain,
        stylesheet,
    }))
}

/// Set an option once; duplicates and opposing preferences report a deterministic error.
/// Only invocation metadata is mutated, with no I/O or source allocation.
fn unique<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), String> {
    // Repeated options usually indicate a typo or contradictory preferences.
    if slot.is_some() {
        return Err(format!("{name} may be specified only once"));
    }
    *slot = Some(value);
    Ok(())
}

/// Fully resolved presentation decisions, with no environment queries during rendering.
#[derive(Clone, Copy)]
struct Display {
    /// Whether compiled ANSI prefixes may be written.
    color: bool,
    /// Whether filename and language metadata are visible.
    header: bool,
    /// Whether source lines carry a right-aligned numeric gutter.
    numbers: bool,
    /// Whether compact frame edges are visible.
    frame: bool,
    /// Whether source control characters must be displayed as escaped text.
    sanitize: bool,
}

/// Stream one source view with fixed style state and no per-token heap allocation.
/// Uncoloured undecorated pipe output takes a direct write path and preserves bytes.
fn render(
    output: &mut impl Write,
    source: &str,
    label: &str,
    styles: &Styles,
    display: Display,
) -> io::Result<()> {
    // The most common piping path needs neither tokenization nor line counting.
    if !display.color && !display.header && !display.numbers && !display.frame && !display.sanitize
    {
        return output.write_all(source.as_bytes());
    }
    let lines = source.bytes().filter(|byte| *byte == b'\n').count()
        + usize::from(!source.is_empty() && !source.ends_with('\n'));
    let mut renderer = Renderer {
        output,
        styles,
        display,
        active: None,
        line: 1,
        width: lines.max(1).ilog10() as usize + 1,
        start: true,
    };
    // Header/frame chrome is opt-in for redirected output and never modifies the source file.
    if display.header || display.frame {
        renderer.style(style::BORDER)?;
        renderer.output.write_all("╭─ ".as_bytes())?;
        if display.header {
            renderer.style(style::HEADER)?;
            visible(renderer.output, label)?;
            renderer.style(style::GUTTER)?;
            write!(renderer.output, "  ·  Holo  ·  {lines} lines")?;
        } else {
            renderer.output.write_all(b"Holo")?;
        }
        renderer.reset()?;
        renderer.output.write_all(b"\n")?;
    }
    // Classification is skipped entirely when only uncoloured decoration is requested.
    if display.color {
        for token in highlight::highlight(source) {
            renderer.fragment(
                &source[token.span.start..token.span.end],
                token.kind as usize,
            )?;
        }
    } else {
        renderer.fragment(source, highlight::HighlightKind::Plain as usize)?;
    }
    renderer.reset()?;
    // A display footer needs its own line; undecorated colour output retains EOF exactly.
    if !renderer.start && (display.header || display.numbers || display.frame) {
        renderer.output.write_all(b"\n")?;
    }
    if display.frame {
        renderer.style(style::BORDER)?;
        renderer.output.write_all("╰─\n".as_bytes())?;
        renderer.reset()?;
    }
    Ok(())
}

/// Constant-size terminal state shared across borrowed source fragments.
struct Renderer<'a, W: Write> {
    /// Caller-owned buffered output; errors propagate without retries or hidden output copies.
    output: &'a mut W,
    /// Invocation-owned, precompiled palette.
    styles: &'a Styles,
    /// Resolved display options.
    display: Display,
    /// Current nonempty ANSI style, allowing adjacent equal styles to coalesce.
    active: Option<usize>,
    /// One-based current source line.
    line: usize,
    /// Decimal gutter width computed once from total source lines.
    width: usize,
    /// Whether the next source fragment begins a fresh display line.
    start: bool,
}

impl<W: Write> Renderer<'_, W> {
    /// Switch to a compiled category only when it changes; defaults require no prefix.
    /// Writes at most one reset/prefix pair and performs no allocation or parsing.
    fn style(&mut self, index: usize) -> io::Result<()> {
        let prefix = self.styles.prefix(index);
        let desired = (self.display.color && !prefix.is_empty()).then_some(index);
        // Equal styles need no terminal traffic; empty styles share the terminal-default state.
        if desired != self.active {
            self.reset()?;
            if desired.is_some() {
                self.output.write_all(prefix.as_bytes())?;
            }
            self.active = desired;
        }
        Ok(())
    }

    /// Restore terminal defaults only if an ANSI prefix is currently active.
    fn reset(&mut self) -> io::Result<()> {
        // No escape sequences are emitted in uncoloured mode or for an already-default state.
        if self.active.take().is_some() {
            self.output.write_all(b"\x1b[0m")?;
        }
        Ok(())
    }

    /// Emit a borrowed lexical fragment, inserting gutters at actual LF/CRLF boundaries.
    /// Maintains line state across multi-line comments and tokens without copying their text.
    fn fragment(&mut self, text: &str, kind: usize) -> io::Result<()> {
        // Split includes delimiters so no synthetic final source line is introduced.
        for piece in text.split_inclusive('\n') {
            if self.start {
                if self.display.numbers {
                    self.style(style::GUTTER)?;
                    write!(self.output, "{:>width$}", self.line, width = self.width)?;
                    self.style(style::BORDER)?;
                    self.output.write_all(" │ ".as_bytes())?;
                } else if self.display.frame {
                    self.style(style::BORDER)?;
                    self.output.write_all("│ ".as_bytes())?;
                }
                self.start = false;
            }
            let (body, newline) = if let Some(body) = piece.strip_suffix("\r\n") {
                (body, "\r\n")
            } else if let Some(body) = piece.strip_suffix('\n') {
                (body, "\n")
            } else {
                (piece, "")
            };
            self.style(kind)?;
            // Terminal display must not execute control bytes embedded in source comments.
            if self.display.sanitize {
                visible(self.output, body)?;
            } else {
                self.output.write_all(body.as_bytes())?;
            }
            // Reset before delimiters so syntax attributes never bleed into a gutter or prompt.
            if !newline.is_empty() {
                self.reset()?;
                self.output.write_all(newline.as_bytes())?;
                self.line += 1;
                self.start = true;
            }
        }
        Ok(())
    }
}

/// Write text with terminal-control scalars escaped, preserving printable UTF-8 and tabs.
/// Uses borrowed slices and integer formatting only for unusual control characters.
fn visible(output: &mut impl Write, text: &str) -> io::Result<()> {
    let mut start = 0;
    // Character boundaries preserve Unicode while preventing C0/C1 terminal command injection.
    for (offset, character) in text.char_indices() {
        if character.is_control() && character != '\t' {
            output.write_all(&text.as_bytes()[start..offset])?;
            if character.is_ascii() {
                write!(output, "\\x{:02x}", character as u32)?;
            } else {
                write!(output, "\\u{{{:x}}}", character as u32)?;
            }
            start = offset + character.len_utf8();
        }
    }
    output.write_all(&text.as_bytes()[start..])
}
