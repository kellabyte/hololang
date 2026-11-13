//! Source-formatting command adapter with explicit output ownership.
//!
//! The shared formatter borrows input and returns validated source text. This
//! boundary owns stdin/stdout, check status and opt-in atomic file replacement.
//! Native compilation and LLVM discovery are never part of source formatting.

use crate::source_input::{self, SourceInput};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

/// Formatting help; default output is stdout so inspection never overwrites source.
const HELP: &str = "Usage: holo fmt <source.holo|-> [--check|--write]
       holo fmt --stdin [--check]

  --check   Exit 1 when formatting would change the source; write no source output
  --write   Atomically replace a regular source file after successful formatting
  --stdin   Read UTF-8 source from stdin (default output: stdout)
  --help    Show this help

Formatting uses the experimental source grammar, preserves identifiers/comments,
and needs no LLVM installation. The default writes formatted source to stdout.";

/// One formatting invocation with explicit source and mutation policy.
struct Options {
    /// File or stream acquired once before syntax validation.
    input: SourceInput,
    /// Whether differences are reported through status instead of source output.
    check: bool,
    /// Whether a validated regular file may be replaced atomically.
    write: bool,
}

/// Format one validated input, checking or replacing only when explicitly requested.
/// Parse errors and I/O failures leave source untouched; broken stdout pipes succeed.
pub fn run() -> Result<ExitCode, String> {
    let Some(options) = parse(source_input::arguments()?)? else {
        println!("{HELP}");
        return Ok(ExitCode::SUCCESS);
    };
    // Validate the mutation target before reading it, including unchanged files and special files.
    if options.write {
        let SourceInput::File(path) = &options.input else {
            unreachable!("stdin --write rejected during argument validation");
        };
        regular_file(path)?;
    }
    let source = options.input.read()?;
    let formatted = holo::format::format(&source)
        .map_err(|error| error.render(options.input.label(), &source))?;
    // Check mode has no output artifact and distinguishes differences from unchanged input.
    if options.check {
        if source == formatted {
            return Ok(ExitCode::SUCCESS);
        }
        return Err(format!("{} needs formatting", options.input.label()));
    }
    // Only explicit --write can replace a file, and an unchanged file retains its inode/mtime.
    if options.write {
        if source != formatted {
            let SourceInput::File(path) = &options.input else {
                unreachable!("stdin --write rejected during argument validation");
            };
            replace(path, &source, &formatted)?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    let stdout = io::stdout();
    let mut output = stdout.lock();
    // The editor protocol is plain UTF-8 text without headers, ANSI, or progress messages.
    match output
        .write_all(formatted.as_bytes())
        .and_then(|()| output.flush())
    {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(ExitCode::SUCCESS),
        Err(error) => Err(format!("write formatted source: {error}")),
    }
}

/// Parse finite formatting options and reject ambiguous input/mutation policies.
/// Standalone help returns None before any source or filesystem access.
fn parse(arguments: Vec<String>) -> Result<Option<Options>, String> {
    // Extra help arguments are errors rather than a way to hide malformed invocations.
    if arguments == ["--help"] || arguments == ["-h"] {
        return Ok(None);
    }
    let mut input = None;
    let mut check = false;
    let mut write = false;
    let mut positional = false;
    // Every iteration consumes one option or operand, so parsing is invocation-sized.
    for argument in arguments {
        match argument.as_str() {
            "--" if !positional => positional = true,
            "--stdin" if !positional => source_input::set(&mut input, SourceInput::Stdin)?,
            "--check" if !positional => crate::set_flag(&mut check, "--check")?,
            "--write" if !positional => crate::set_flag(&mut write, "--write")?,
            _ => {
                // A dash is stdin; other leading-dash paths need the explicit -- boundary.
                if !positional && argument != "-" && argument.starts_with('-') {
                    return Err(format!("unknown fmt option {argument:?}"));
                }
                source_input::set(&mut input, SourceInput::path(argument))?;
            }
        }
    }
    let input = input.ok_or_else(|| format!("expected a source path or --stdin\n{HELP}"))?;
    // Check and replacement are distinct effects; stream input has no owned replacement path.
    if check && write {
        return Err("--check and --write are mutually exclusive".into());
    }
    if write && matches!(input, SourceInput::Stdin) {
        return Err("--write requires a regular file, not stdin".into());
    }
    Ok(Some(Options {
        input,
        check,
        write,
    }))
}

/// Replace a regular file through a same-directory temporary and atomic rename.
/// Preserves permissions, checks for intervening edits, and removes failed scratch
/// files. This is not a lock against concurrent writers; hard links are not retained.
fn replace(path: &Path, original: &str, formatted: &str) -> Result<(), String> {
    let metadata = regular_file(path)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = None;
    // Exclusive creation prevents collisions with other formatters; bounded retries prevent hangs.
    for attempt in 0..32 {
        let candidate = parent.join(format!(".holo-fmt-{}-{attempt}.tmp", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("create formatting temporary: {error}")),
        }
    }
    let (temporary, mut file) = temporary.ok_or("could not reserve a formatting temporary")?;
    let result = (|| -> Result<(), String> {
        // Restrict the empty temporary before writing, so private source bytes are never
        // briefly exposed through a more permissive process umask.
        file.set_permissions(metadata.permissions())
            .and_then(|()| file.write_all(formatted.as_bytes()))
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("write formatting temporary: {error}"))?;
        let current = fs::read_to_string(path)
            .map_err(|error| format!("recheck source before replacement: {error}"))?;
        // A visible intervening edit must never be replaced with stale formatted text.
        if current != original {
            return Err("source changed during formatting; no replacement performed".into());
        }
        fs::rename(&temporary, path)
            .map_err(|error| format!("replace formatted source {}: {error}", path.display()))
    })();
    // Successful rename already consumes the scratch path; failed work is best-effort cleaned up.
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Inspect a replacement target without following symlinks, rejecting special files.
/// Returns metadata for permission preservation; no source bytes are read or changed.
fn regular_file(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect source {}: {error}", path.display()))?;
    // Only regular files have the ownership/replacement contract offered by --write.
    if !metadata.file_type().is_file() {
        return Err("--write accepts regular files only; symbolic links are unsupported".into());
    }
    Ok(metadata)
}
