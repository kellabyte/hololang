//! Command-line boundary for the explicitly experimental Holo source frontend.
//!
//! Source is parsed/lowered and checked before LLVM emission. Checking alone never
//! discovers or invokes LLVM. Native compilation uses the existing checked driver;
//! this file owns arguments, diagnostics, output ownership, execution, and optional
//! phase observations. Native outputs are never overwritten; fmt has explicit --write.
//! The current source subset
//! supports successful entry completion only, not general function return values.
//! CLI arguments/paths and source text must be UTF-8; source APIs remain unfrozen.
//! Layout and copy inspection share the checker and need no native tools.
//! Package commands resolve an explicit manifest into a checked source input.
//! Naming diagnostics are separate from source verification and native emission.
//! Only the lock action writes package metadata.
//! Separate text-tool adapters expose LLVM-free highlighting and formatting.

mod format;
mod highlight;
mod source_input;
mod style;

use holo::driver::{BuildTimings, NativeToolchain, TempDir};
use holo::layout::{self, LayoutMode, LayoutPlan};
use holo::{frontend, llvm, package, source::Diagnostic};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode, ExitStatus};
use std::time::Instant;

/// Supported source operations, each with a distinct artifact/execution contract.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Parse, lower, and validate source without requiring a native toolchain.
    Check,
    /// Inspect packed sizes, symbolic offsets, and header decisions without LLVM.
    InspectLayout,
    /// Inspect byte initialization decisions for the native host without LLVM.
    InspectCopies,
    /// Emit unoptimized checked LLVM text to stdout or an exclusively new file.
    EmitLlvm,
    /// Build an exclusively new native executable at the required output path.
    Build,
    /// Build a temporary native executable and run it with inherited standard I/O.
    Run,
    /// Resolve local dependencies and explicitly write the package lockfile.
    Lock,
}

/// One explicit compilation root, selected before opening any source files.
enum Input {
    /// Existing source-file mode with optional experimental leaf imports.
    File {
        /// UTF-8 entry source path.
        path: PathBuf,
        /// Trusted directory of local `holo.<leaf>` sources.
        library_root: Option<PathBuf>,
    },
    /// Manifest-driven mode with a package entry and declared dependencies.
    Package {
        /// Explicit path to `holo.toml`.
        manifest: PathBuf,
        /// Verify the resolved dependency snapshot against `holo.lock`.
        locked: bool,
    },
}

/// Validated invocation options; no source or native artifacts are owned yet.
struct Options {
    /// Requested compilation/execution boundary.
    action: Action,
    /// Source file or manifest defining the compilation inputs.
    input: Input,
    /// Requested new artifact; required only for build, optional for LLVM emission.
    output: Option<PathBuf>,
    /// Whether native code generation uses the driver's O3 rather than O0 mode.
    release: bool,
    /// Whether compiler phase observations are printed to stderr.
    timings: bool,
    /// Whether source naming diagnostics fail an otherwise valid compilation.
    strict_names: bool,
    /// Experimental physical policy; full-width derived sizes are the source default.
    layout: LayoutMode,
}

/// Invoke the compiler, printing failures on stderr and preserving native status.
///
/// Arguments are read once. No shell is used; commands receive separate arguments.
/// Compiler failures return status one, and help returns success without I/O work.
fn main() -> ExitCode {
    // Text tools have their own option/output contracts and never enter native compilation.
    let result = match env::args_os()
        .nth(1)
        .as_deref()
        .and_then(|value| value.to_str())
    {
        Some("highlight") => highlight::run(),
        Some("fmt") => format::run(),
        _ => compiler_command(),
    };
    // One boundary renders source/tool errors consistently; normal output remains clean UTF-8.
    match result {
        Ok(status) => status,
        Err(error) => {
            eprintln!("holo: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Run the existing experimental compiler command after text-tool dispatch.
/// Owns compiler argument validation and preserves existing native output/status contracts.
fn compiler_command() -> Result<ExitCode, String> {
    arguments().and_then(|options| {
        // Help has no input or compilation effects.
        match options {
            Some(options) => execute(options),
            None => {
                println!("{}", usage());
                Ok(ExitCode::SUCCESS)
            }
        }
    })
}

/// Return concise usage and the supported experimental command/option boundaries.
/// This allocates no source data and performs no discovery or mutation.
fn usage() -> &'static str {
    "Usage: holo <check|inspect-layout|inspect-copies|emit-llvm|build|run> <source.holo> [options]\n\
            holo <check|inspect-layout|inspect-copies|emit-llvm|build|run> --package <holo.toml> [--locked] [options]\n\
            holo lock --package <holo.toml>\n\
     \n\
     Text tools (no LLVM): holo highlight <source.holo|-> [--header] [-n] [--frame]\n\
                          holo fmt <source.holo|-> [--check|--write]\n\
     Use holo highlight --help or holo fmt --help for text-tool options.\n\
     \n\
     --library-root <directory>  Resolve experimental local using directives\n\
     --package <holo.toml>       Select a local package and its dependencies\n\
     --locked                    Require the existing package lockfile to match\n\
     --release       Use O3 for build/run (default: O0)\n\
     -o <new-path>   Required for build; optional for emit-llvm; never overwrite\n\
     --timings       Print compiler phase observations to stderr\n\
     --strict-names  Treat source naming convention warnings as errors\n\
     --layout <mode> derived (default, u64 lengths), wide, compact (checked u32)\n\
     --help          Show this help\n\
     \n\
     check and inspection need no LLVM installation. Native/LLVM commands require installed LLVM."
}

/// Parse process arguments into strict options, or `None` for a standalone help.
///
/// Returns an owned explanation for missing, repeated, or unsupported arguments.
/// Options may surround the single input path; `--` ends option interpretation.
/// Non-UTF-8 arguments/paths are rejected before any source file is opened.
fn arguments() -> Result<Option<Options>, String> {
    let arguments = env::args_os()
        .skip(1)
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| "command-line arguments and paths must be valid UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Only a standalone help form bypasses normal required options. Unknown
    // commands and additional arguments remain visible errors instead of guesses.
    if arguments == ["--help"] || arguments == ["-h"] {
        return Ok(None);
    }
    let Some(command) = arguments.first() else {
        return Err(usage().into());
    };
    let action = match command.as_str() {
        "check" => Action::Check,
        "inspect-layout" => Action::InspectLayout,
        "inspect-copies" => Action::InspectCopies,
        "emit-llvm" => Action::EmitLlvm,
        "build" => Action::Build,
        "run" => Action::Run,
        "lock" => Action::Lock,
        _ => return Err(format!("unknown command {command:?}\n{}", usage())),
    };
    // Command-local help also performs no file or toolchain operations.
    if arguments.len() == 2 && matches!(arguments[1].as_str(), "--help" | "-h") {
        return Ok(None);
    }
    let mut input = None;
    let mut output = None;
    let mut legacy_experimental = false;
    let mut release = false;
    let mut timings = false;
    let mut strict_names = false;
    let mut layout = None;
    let mut library_root = None;
    let mut manifest = None;
    let mut locked = false;
    let mut positional_only = false;
    let mut pending = arguments.into_iter().skip(1);
    // Each iteration consumes one argument (and -o consumes its value), so the
    // finite invocation terminates while rejecting duplicate/unknown spellings.
    while let Some(argument) = pending.next() {
        match argument.as_str() {
            "--" if !positional_only => positional_only = true,
            "--experimental" if !positional_only => {
                // Older scripts can still pass the former opt-in once.
                set_flag(&mut legacy_experimental, "--experimental")?;
            }
            "--release" if !positional_only => set_flag(&mut release, "--release")?,
            "--timings" if !positional_only => set_flag(&mut timings, "--timings")?,
            "--strict-names" if !positional_only => set_flag(&mut strict_names, "--strict-names")?,
            "--locked" if !positional_only => set_flag(&mut locked, "--locked")?,
            "--layout" if !positional_only => {
                // Each layout has one fixed interpretation; repeated or unknown values are errors.
                if layout.is_some() {
                    return Err("--layout may be specified only once".into());
                }
                layout = Some(match pending.next().as_deref() {
                    Some("wide") => LayoutMode::Wide,
                    Some("derived") => LayoutMode::Derived64,
                    Some("compact") => LayoutMode::Compact32,
                    _ => return Err("--layout requires wide, derived, or compact".into()),
                });
            }
            "--library-root" if !positional_only => {
                // A single explicit root prevents fallback discovery or ambiguous imports.
                if library_root.is_some() {
                    return Err("--library-root may be specified only once".into());
                }
                let path = pending
                    .next()
                    .ok_or("--library-root requires a directory")?;
                // An empty path implies the working directory, and a leading
                // dash may be the next option. Require an explicit spelling.
                if path.is_empty() || path.starts_with('-') {
                    return Err("--library-root requires a directory path. Use ./ for a name beginning with a dash".into());
                }
                library_root = Some(PathBuf::from(path));
            }
            "--package" if !positional_only => {
                // A single named manifest keeps resolution independent of ambient
                // project discovery. An option-shaped value is a missing path.
                if manifest.is_some() {
                    return Err("--package may be specified only once".into());
                }
                let path = pending
                    .next()
                    .filter(|path| !path.is_empty() && !path.starts_with('-'))
                    .ok_or("--package requires a manifest path. Use ./ for a name beginning with a dash")?;
                manifest = Some(PathBuf::from(path));
            }
            "-o" if !positional_only => {
                // The value is always a path; even a leading dash is literal here.
                if output.is_some() {
                    return Err("-o may be specified only once".into());
                }
                output = Some(PathBuf::from(pending.next().ok_or("-o requires a path")?));
            }
            _ => {
                // A leading dash is accepted as a filename only after --; this
                // prevents mistyped options from becoming accidental source paths.
                if !positional_only && argument.starts_with('-') {
                    return Err(format!("unknown option {argument:?}"));
                }
                if input.replace(PathBuf::from(argument)).is_some() {
                    return Err("expected exactly one source path".into());
                }
            }
        }
    }
    let input = match manifest {
        // These sources of identity are independent modes. Mixing them would
        // make it unclear which entry or import roots the compiler should use.
        Some(_) if input.is_some() || library_root.is_some() => {
            return Err("--package cannot be combined with a source path or --library-root".into());
        }
        Some(manifest) => Input::Package { manifest, locked },
        None if action == Action::Lock => return Err("lock requires --package <holo.toml>".into()),
        None if locked => return Err("--locked requires --package <holo.toml>".into()),
        None => Input::File {
            path: input.ok_or("expected a source path")?,
            library_root,
        },
    };
    // Updating the lock resolves new inputs. Layout and locked verification
    // have meaning only when compiling from the resulting graph.
    if action == Action::Lock && (locked || layout.is_some()) {
        return Err("lock does not accept --locked or --layout".into());
    }
    // Locking resolves manifests without checking source declarations.
    if action == Action::Lock && strict_names {
        return Err("--strict-names applies only to checked source commands".into());
    }
    // Reject ineffective flags rather than suggesting that checking/text emission
    // applies native O3 or that run/check owns a caller-selected output artifact.
    if release && !matches!(action, Action::Build | Action::Run) {
        return Err("--release applies only to build and run".into());
    }
    if output.is_some() && !matches!(action, Action::Build | Action::EmitLlvm) {
        return Err("-o applies only to build and emit-llvm".into());
    }
    if action == Action::Build && output.is_none() {
        return Err("build requires -o <new-path>".into());
    }
    Ok(Some(Options {
        action,
        input,
        output,
        release,
        timings,
        strict_names,
        // Full-width derivation passed conformance and matched measurements;
        // narrow lengths still require an explicit selection, while wide is a control.
        layout: layout.unwrap_or(LayoutMode::Derived64),
    }))
}

/// Set one previously absent boolean option, rejecting an accidental duplicate.
/// `flag` is mutated only on success; no files or compiler state are affected.
fn set_flag(flag: &mut bool, name: &str) -> Result<(), String> {
    // Repeated flags usually indicate an invocation mistake, not distinct effects.
    if *flag {
        return Err(format!("{name} may be specified only once"));
    }
    *flag = true;
    Ok(())
}

/// Parse/check one source and carry out the requested checked compilation action.
///
/// Errors retain source diagnostics or contextual filesystem/tool explanations.
/// Native artifacts use exclusive creation through the existing driver. Run owns
/// its scratch executable until the child finishes, then reclaims compiler files.
fn execute(options: Options) -> Result<ExitCode, String> {
    match &options.input {
        Input::Package { manifest, locked } => {
            let start = Instant::now();
            let graph = package::load(manifest, *locked)?;
            phase(options.timings, "package_resolve", start);
            // The explicit lock action needs neither an executable entry nor LLVM.
            if options.action == Action::Lock {
                graph.write_lock()?;
                return Ok(ExitCode::SUCCESS);
            }
            let start = Instant::now();
            // A manifest without an application entry still has a complete
            // library check. Return before selecting the executable pipeline.
            if options.action == Action::Check && !graph.has_entry() {
                let diagnostics = frontend::check_experimental_library_package_with_naming(
                    &graph,
                    options.strict_names,
                )?;
                phase(options.timings, "check_library", start);
                return Ok(report_naming_diagnostics(diagnostics, options.strict_names));
            }
            let project = frontend::parse_experimental_package(&graph)?;
            phase(options.timings, "parse_lower", start);
            compile_source(
                &options,
                project.program(),
                |error| project.render(error),
                |warning| project.render_warning(warning),
            )
        }
        Input::File { path, library_root } => {
            let source = fs::read_to_string(path)
                .map_err(|error| format!("read UTF-8 source {}: {error}", path.display()))?;
            let start = Instant::now();
            // An explicit leaf root selects the existing compatibility resolver.
            // Ordinary standalone checking never consults local library files.
            if let Some(root) = library_root {
                let project = frontend::parse_experimental_project(path, &source, root)?;
                phase(options.timings, "parse_lower", start);
                compile_source(
                    &options,
                    project.program(),
                    |error| project.render(error),
                    |warning| project.render_warning(warning),
                )
            } else {
                let path = path.to_str().expect("arguments validated UTF-8");
                let render = |error: &Diagnostic| error.render(path, &source);
                let parsed =
                    frontend::parse_experimental(&source).map_err(|error| render(&error))?;
                phase(options.timings, "parse_lower", start);
                compile_source(&options, &parsed, render, |warning| {
                    warning.render_warning(path, &source)
                })
            }
        }
    }
}

/// Check resolved source and dispatch through the single checked native path.
/// The caller supplies source renderers so diagnostics keep their original files.
/// Returns the command status or a diagnostic and only writes requested outputs.
fn compile_source(
    options: &Options,
    parsed: &frontend::SourceProgram,
    render: impl Fn(&Diagnostic) -> String,
    render_warning: impl Fn(&Diagnostic) -> String,
) -> Result<ExitCode, String> {
    let start = Instant::now();
    let checked = parsed.check().map_err(|error| render(&error))?;
    phase(options.timings, "check", start);
    // Check and inspection validate layout without a toolchain. Hold any report
    // until strict naming checks have passed so failures keep stdout clean.
    // Native actions still plan their layout once through the emitter.
    let checked_output = match options.action {
        Action::Check => {
            LayoutPlan::new(&checked, options.layout).map_err(|error| error.to_string())?;
            Some(String::new())
        }
        Action::InspectLayout => {
            Some(layout::inspect(&checked, options.layout).map_err(|error| error.to_string())?)
        }
        Action::InspectCopies => {
            LayoutPlan::new(&checked, options.layout).map_err(|error| error.to_string())?;
            Some(llvm::explain_copies(
                &checked,
                NativeToolchain::host_target_triple()?,
            ))
        }
        _ => None,
    };
    let diagnostics = parsed.naming_diagnostics().iter().map(|diagnostic| {
        // Select severity before rendering so source spans remain identical.
        if options.strict_names {
            render(diagnostic)
        } else {
            render_warning(diagnostic)
        }
    });
    let status = report_naming_diagnostics(diagnostics, options.strict_names);
    // Stop strict violations before writing stdout, discovering tools or creating files.
    if status != ExitCode::SUCCESS {
        return Ok(status);
    }
    // Checked-only actions finish without tool discovery or a native subprocess.
    if let Some(output) = checked_output {
        print!("{output}");
        return Ok(ExitCode::SUCCESS);
    }
    let start = Instant::now();
    let tools = NativeToolchain::discover()?;
    phase(options.timings, "toolchain_discovery", start);
    let start = Instant::now();
    let ir = llvm::emit_with_layout(&checked, tools.target_triple(), options.layout)
        .map_err(|error| error.to_string())?;
    phase(options.timings, "llvm_lower", start);
    // Each action owns precisely its documented artifact boundary. Emission is
    // unoptimized LLVM text; only native build/run select a code generation level.
    match options.action {
        Action::Check | Action::InspectLayout | Action::InspectCopies | Action::Lock => {
            unreachable!("check/inspection returned before tool discovery")
        }
        Action::EmitLlvm => {
            match &options.output {
                Some(path) => {
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)
                        .map_err(|error| {
                            format!("create new LLVM output {}: {error}", path.display())
                        })?;
                    file.write_all(ir.as_bytes()).map_err(|error| {
                        format!("write LLVM output {}: {error}", path.display())
                    })?;
                }
                None => io::stdout()
                    .lock()
                    .write_all(ir.as_bytes())
                    .map_err(|error| format!("write LLVM stdout: {error}"))?,
            }
            Ok(ExitCode::SUCCESS)
        }
        Action::Build => {
            let output = options
                .output
                .as_deref()
                .expect("build output required during parsing");
            build(&tools, &ir, output, options.release, options.timings)?;
            Ok(ExitCode::SUCCESS)
        }
        Action::Run => {
            let scratch = TempDir::new()?;
            let executable = scratch.path().join("holo-program");
            build(&tools, &ir, &executable, options.release, options.timings)?;
            let status = Command::new(&executable)
                .status()
                .map_err(|error| format!("run native program: {error}"))?;
            native_status(status)
        }
    }
}

/// Write pre-rendered naming diagnostics to stderr and return the policy status.
/// Consume one owned source excerpt and terminating newline per iteration without
/// buffering the output. Warnings preserve success. Strict mode fails after all
/// violations are written. Stderr uses the same write behavior as other CLI errors.
fn report_naming_diagnostics(
    diagnostics: impl IntoIterator<Item = String>,
    strict: bool,
) -> ExitCode {
    let mut found = false;
    // Stream every diagnostic so strict mode reports the full set in one invocation.
    for diagnostic in diagnostics {
        eprint!("{diagnostic}");
        found = true;
    }
    // Only explicit strict mode makes a source-style violation fatal.
    if strict && found {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Build verified LLVM with the native driver and optionally report phase costs.
///
/// Arguments describe checked LLVM, a new output path, and the requested mode.
/// Returns driver failure text without masking errors; timings and exact commands
/// go only to stderr, never into generated program output or copied source files.
fn build(
    tools: &NativeToolchain,
    ir: &str,
    output: &std::path::Path,
    release: bool,
    timings: bool,
) -> Result<(), String> {
    let start = Instant::now();
    let details = tools.build(ir, output, release)?;
    phase(timings, "native_driver", start);
    // Driver subphases include actual verification/codegen/link work, while its
    // recorded commands include argument paths rather than environment values.
    if timings {
        driver_timings(&details);
    }
    Ok(())
}

/// Print named elapsed compiler work when requested, with no benchmark inference.
/// `start` is consumed only as a wall-clock observation; generated code is unchanged.
fn phase(enabled: bool, name: &str, start: Instant) {
    // Ordinary compiler output remains quiet unless timing observations are asked for.
    if enabled {
        eprintln!(
            "timing {name}: {:.3} ms",
            start.elapsed().as_secs_f64() * 1_000.0
        );
    }
}

/// Report the driver's measured subphases and recorded commands on stderr only.
/// This borrows existing metadata, never inspects environment values or source text.
fn driver_timings(details: &BuildTimings) {
    eprintln!(
        "timing native_verification: {:.3} ms",
        details.verification.as_secs_f64() * 1_000.0
    );
    eprintln!(
        "timing native_codegen_link: {:.3} ms",
        details.codegen_and_link.as_secs_f64() * 1_000.0
    );
    // Preserve the actual argument vectors as debug-rendered by the native driver.
    for command in &details.commands {
        eprintln!("command: {command}");
    }
}

/// Preserve the child process's code, or report signal termination as a failure.
///
/// The current source entry return subset can explicitly succeed with zero;
/// runtime assertions/bounds traps may terminate unsuccessfully. Unix signal
/// codes follow the conventional 128+signal mapping, and diagnostics use stderr.
fn native_status(status: ExitStatus) -> Result<ExitCode, String> {
    // A normal exit retains its native code, including any future nonzero subset.
    if let Some(code) = status.code() {
        let code = u8::try_from(code)
            .map_err(|_| format!("native exit code is outside supported range: {code}"))?;
        if code != 0 {
            eprintln!("holo: native program {status}");
        }
        return Ok(ExitCode::from(code));
    }
    // Every currently supported native host is Unix. Keep conditional compilation
    // explicit so this CLI can still type-check on a host the driver will reject.
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            let code = u8::try_from(128 + signal).unwrap_or(1);
            eprintln!("holo: native program {status}");
            return Ok(ExitCode::from(code));
        }
    }
    Err(format!(
        "native program terminated without an exit code: {status}"
    ))
}
