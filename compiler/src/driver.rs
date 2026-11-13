//! Native verification, code generation, and temporary-artifact ownership.
//!
//! A matched `opt`/`clang` installation compiles checked LLVM for the current
//! 64-bit macOS/Linux host. Verification is separate from Clang's target-aware
//! optimization pipeline. Subprocesses and temporary files are measured compiler
//! costs, not generated-program memory operations. No archived code is imported.
//! ARM64 macOS imports use eager chained fixups, moving helper symbol resolution
//! to image loading before Holo execution. Other targets retain experimental
//! linkage; this does not certify their runtime effects or bound startup cost.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Matched native LLVM tools plus the explicit host target used by the emitter.
#[derive(Debug)]
pub struct NativeToolchain {
    /// Optimizer/verifier from the selected installation.
    pub opt: PathBuf,
    /// Native code generator/link driver from the same installation.
    pub clang: PathBuf,
    /// Host target; only supported 64-bit architectures are admitted.
    triple: String,
    /// Recorded actual version output, including the backend release.
    pub identity: String,
}

/// Wall-clock driver phases; these are not native kernel measurements.
#[derive(Debug)]
pub struct BuildTimings {
    /// LLVM verification and textual serialization time.
    pub verification: Duration,
    /// Target-aware optimization, code generation, and linking time.
    pub codegen_and_link: Duration,
    /// Exact debug-rendered subprocess commands, recorded before execution.
    pub commands: Vec<String>,
}

impl NativeToolchain {
    /// Identify the native host triple without touching the filesystem or invoking LLVM.
    /// Unsupported hosts return the same diagnostic as native tool discovery.
    pub fn host_target_triple() -> Result<&'static str, String> {
        // The current runtime uses 64-bit sizes and libc on these two OS families.
        match (env::consts::OS, env::consts::ARCH) {
            ("macos", "aarch64") => Ok("arm64-apple-macosx11.0.0"),
            ("macos", "x86_64") => Ok("x86_64-apple-macosx10.13.0"),
            ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
            ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
            _ => Err("native core currently requires 64-bit macOS/Linux on ARM64/x86-64".into()),
        }
    }

    /// Discover installed matched tools without downloading dependencies.
    /// Returns an error for unsupported hosts or missing tools; environment is read only.
    pub fn discover() -> Result<Self, String> {
        let triple = Self::host_target_triple()?;
        let mut candidates = Vec::new();
        // An explicit installation is authoritative; never silently replace a bad override.
        if let Some(prefix) = env::var_os("HOLO_LLVM_PREFIX") {
            candidates.push(PathBuf::from(prefix).join("bin"));
        } else {
            candidates.extend([
                PathBuf::from("/opt/homebrew/opt/llvm/bin"),
                PathBuf::from("/usr/local/opt/llvm/bin"),
                PathBuf::from("/usr/lib/llvm-22/bin"),
                PathBuf::from("/usr/lib/llvm-21/bin"),
            ]);
            // PATH entries remain paired directories, rather than mixing tool versions.
            if let Some(paths) = env::var_os("PATH") {
                candidates.extend(env::split_paths(&paths));
            }
        }
        // Probe candidates in priority order; version output identifies the tools actually selected.
        for bin in candidates {
            let opt = bin.join("opt");
            let clang = bin.join("clang");
            if opt.is_file() && clang.is_file() {
                let opt_version = run(Command::new(&opt).arg("--version"), "opt identity")?;
                let clang_version = run(Command::new(&clang).arg("--version"), "clang identity")?;
                let identity = format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&opt_version.stdout),
                    String::from_utf8_lossy(&clang_version.stdout)
                );
                return Ok(Self {
                    opt,
                    clang,
                    triple: triple.into(),
                    identity,
                });
            }
        }
        Err("matched LLVM opt/clang not found; set HOLO_LLVM_PREFIX to an installed prefix".into())
    }

    /// Return the validated target string for LLVM emission; no allocation or mutation.
    pub fn target_triple(&self) -> &str {
        &self.triple
    }

    /// Verify and build native LLVM without overwriting an existing output.
    /// `optimized` selects Clang O3/O0; language checks remain identical.
    /// Returns driver timings or diagnostics; temporary inputs are reclaimed on exit.
    pub fn build(&self, ir: &str, output: &Path, optimized: bool) -> Result<BuildTimings, String> {
        self.build_inner(ir, output, optimized, false)
    }

    /// Instrument generated function bodies with AddressSanitizer for validation.
    /// This changes a test build only, never an ordinary release program.
    pub fn build_sanitized(&self, ir: &str, output: &Path) -> Result<BuildTimings, String> {
        self.build_inner(ir, output, true, true)
    }

    /// Own verification/codegen plumbing shared by normal and instrumented builds.
    /// Reject mismatched targets/existing paths before writing any caller artifact.
    fn build_inner(
        &self,
        ir: &str,
        output: &Path,
        optimized: bool,
        sanitized: bool,
    ) -> Result<BuildTimings, String> {
        let expected = format!("target triple = \"{}\"", self.triple);
        // Prevent accidentally linking IR emitted for another target.
        if !ir.lines().any(|line| line.trim() == expected) {
            return Err("LLVM module target does not match the native driver".into());
        }
        // symlink_metadata also detects dangling symlinks, which must not be overwritten.
        if fs::symlink_metadata(output).is_ok() {
            return Err(format!("refusing to overwrite {}", output.display()));
        }
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|e| format!("create output directory: {e}"))?;
        let scratch = TempDir::new()?;
        let input = scratch.path().join("input.ll");
        let verified = scratch.path().join("verified.ll");
        let binary = scratch.path().join("native");
        let source = if sanitized {
            instrument_address_safety(ir)?
        } else {
            ir.into()
        };
        fs::write(&input, source).map_err(|e| format!("write LLVM input: {e}"))?;
        let mut commands = Vec::new();
        let mut verify = Command::new(&self.opt);
        verify
            .arg("-passes=verify")
            .arg("-S")
            .arg(&input)
            .arg("-o")
            .arg(&verified);
        commands.push(format!("{verify:?}"));
        let start = Instant::now();
        run(&mut verify, "LLVM verification")?;
        let verification = start.elapsed();
        let canonical =
            fs::read_to_string(&verified).map_err(|e| format!("read verified LLVM: {e}"))?;
        // LLVM canonicalization decides the effective target when input repeats
        // directives. Recheck that result rather than trusting one matching raw line.
        if !canonical.lines().any(|line| line.trim() == expected) {
            return Err("verified LLVM module target does not match the native driver".into());
        }
        let mut native = Command::new(&self.clang);
        native.arg("-target").arg(&self.triple).arg("-x").arg("ir");
        // Pin a portable CPU baseline rather than accidentally specializing to the build host.
        if env::consts::ARCH == "aarch64" {
            native.arg("-mcpu=generic");
        } else {
            native.arg("-march=x86-64");
        }
        // Query the current SDK to avoid Homebrew LLVM's stale installation-time SDK paths.
        if cfg!(target_os = "macos") {
            let mut query = Command::new("xcrun");
            query.arg("--show-sdk-path");
            commands.push(format!("{query:?}"));
            let sdk = run(&mut query, "macOS SDK discovery")?;
            native
                .arg("-isysroot")
                .arg(String::from_utf8_lossy(&sdk.stdout).trim());
        }
        native
            .arg(if optimized { "-O3" } else { "-O0" })
            .arg("-Wno-override-module");
        // ARM64 macOS targets macOS11+, where chained imports are supported.
        // Resolve imported helpers during image loading, before Holo execution;
        // lazy first-call dyld binding is outside the reviewed helper contract.
        // This moves loader work to startup, not into each generated function.
        // Other targets retain their experimental linkage and are not certified
        // by the finite Darwin ARM64 runtime profile.
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            native.arg("-Wl,-fixup_chains");
        }
        // Sanitizer passes require function attributes as well as link/codegen flags.
        if sanitized {
            native.args(["-fsanitize=address", "-fno-omit-frame-pointer"]);
        }
        // Measurement helpers contain only clock/barrier boundaries. Ordinary
        // modules pay no extra C compilation or native calls when they are unused.
        if ir.contains("call i64 @holo_now(") || ir.contains("call void @holo_opaque(") {
            let support = scratch.path().join("native_support.c");
            fs::write(&support, include_str!("../../runtime/measurement.c"))
                .map_err(|e| format!("write measurement support: {e}"))?;
            native.arg("-x").arg("c").arg(&support);
        }
        // Hosted input has its own native entry. The canonical definition is
        // the link signal, so unrelated checked binaries pay no support costs.
        if canonical
            .lines()
            .any(|line| line.starts_with("define i32 @holo_hosted_entry("))
        {
            // The output declaration is present only for checked send/wait
            // users. Preserve the input-only native source and compiler flags.
            let output = canonical
                .lines()
                .any(|line| line.starts_with("declare i64 @holo_send_experimental("));
            let session = canonical
                .lines()
                .any(|line| line.starts_with("declare i64 @holo_next_session_experimental("));
            link_hosted_input(&mut native, scratch.path(), output, session)?;
        } else if canonical
            .lines()
            .any(|line| line.starts_with("define i32 @holo_tcp_entry("))
        {
            // The TCP entry takes sole close authority. Keep its process entry
            // separate from the legacy host, which retains and closes its fd.
            link_owned_tcp(&mut native, scratch.path())?;
        }
        native
            .arg("-x")
            .arg("ir")
            .arg(&verified)
            .arg("-o")
            .arg(&binary);
        commands.push(format!("{native:?}"));
        let start = Instant::now();
        run(&mut native, "LLVM native compilation/linking")?;
        let codegen_and_link = start.elapsed();
        // Exclusive creation closes the race after the earlier existence check;
        // another process's file or symlink must never become an overwrite target.
        let mut destination = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .map_err(|e| format!("create native output: {e}"))?;
        let mut source = fs::File::open(&binary).map_err(|e| format!("read native output: {e}"))?;
        std::io::copy(&mut source, &mut destination)
            .map_err(|e| format!("install native output: {e}"))?;
        fs::set_permissions(
            output,
            fs::metadata(&binary)
                .map_err(|e| e.to_string())?
                .permissions(),
        )
        .map_err(|e| format!("set native output permissions: {e}"))?;
        Ok(BuildTimings {
            verification,
            codegen_and_link,
            commands,
        })
    }
}

/// Stage hosted adapter sources in the owned build directory and add C inputs.
/// Errors abort before linking. Other modules do not stage this support.
fn link_hosted_input(
    native: &mut Command,
    scratch: &Path,
    output: bool,
    session: bool,
) -> Result<(), String> {
    fs::write(
        scratch.join("refill.h"),
        include_str!("../../runtime/refill/refill.h"),
    )
    .map_err(|e| format!("write hosted input header: {e}"))?;
    // Output users compile the optional POSIX send boundary and its host setup.
    if output {
        let send = scratch.join("send.c");
        fs::write(&send, include_str!("../../runtime/refill/send.c"))
            .map_err(|e| format!("write hosted output adapter: {e}"))?;
        native
            .arg("-DHOLO_HOSTED_OUTPUT")
            .arg("-x")
            .arg("c")
            .arg(send);
    }
    // Only session-aware entries may own a loopback listener.
    if session {
        let listener = scratch.join("session.c");
        fs::write(&listener, include_str!("../../runtime/refill/session.c"))
            .map_err(|e| format!("write hosted session adapter: {e}"))?;
        native
            .arg("-DHOLO_HOSTED_SESSION")
            .arg("-x")
            .arg("c")
            .arg(listener);
    }
    // Compile the adapter and process entry as separate native translation units.
    for (name, source) in [
        ("refill.c", include_str!("../../runtime/refill/refill.c")),
        ("host.c", include_str!("../../runtime/refill/host.c")),
    ] {
        let path = scratch.join(name);
        fs::write(&path, source).map_err(|e| format!("write hosted input {name}: {e}"))?;
        native.arg("-x").arg("c").arg(path);
    }
    Ok(())
}

/// Link the checked TCP entry with its native handoff, wait broker and parking permits.
/// Staged inputs live only in the build scratch directory. A write failure
/// aborts before linking, and other executable profiles keep their inputs.
fn link_owned_tcp(native: &mut Command, scratch: &Path) -> Result<(), String> {
    // The shared range and transfer adapters keep the same native contracts.
    for (name, source) in [
        ("refill.h", include_str!("../../runtime/refill/refill.h")),
        (
            "tcp_wait.h",
            include_str!("../../runtime/refill/tcp_wait.h"),
        ),
        (
            "tcp_concurrent.h",
            include_str!("../../runtime/refill/tcp_concurrent.h"),
        ),
        (
            "tcp_permits.h",
            include_str!("../../runtime/refill/tcp_permits.h"),
        ),
    ] {
        fs::write(scratch.join(name), source)
            .map_err(|e| format!("write hosted TCP {name}: {e}"))?;
    }
    // Darwin supplies pthreads through libSystem. Linux needs the native
    // thread flag for the C runtime profile and linker.
    if cfg!(target_os = "linux") {
        native.arg("-pthread");
    }
    for (name, source) in [
        ("refill.c", include_str!("../../runtime/refill/refill.c")),
        ("send.c", include_str!("../../runtime/refill/send.c")),
        ("tcp.c", include_str!("../../runtime/refill/tcp.c")),
        (
            "tcp_wait.c",
            include_str!("../../runtime/refill/tcp_wait.c"),
        ),
        (
            "tcp_concurrent.c",
            include_str!("../../runtime/refill/tcp_concurrent.c"),
        ),
        (
            "tcp_permits.c",
            include_str!("../../runtime/refill/tcp_permits.c"),
        ),
    ] {
        let path = scratch.join(name);
        fs::write(&path, source).map_err(|e| format!("write hosted TCP {name}: {e}"))?;
        native.arg("-x").arg("c").arg(path);
    }
    Ok(())
}

/// Add the ASan function attribute to this emitter's single-line definitions.
/// Input/output are LLVM text; reject unexpected definition syntax instead of silently
/// claiming instrumented coverage. This allocation belongs to test compilation only.
fn instrument_address_safety(ir: &str) -> Result<String, String> {
    let mut result = String::new();
    // Every generated definition is examined; external declarations are left unchanged.
    for line in ir.lines() {
        if line.trim_start().starts_with("define ") {
            let header = line
                .trim_end()
                .strip_suffix('{')
                .ok_or("unexpected multiline LLVM definition for ASan")?;
            result.push_str(header);
            result.push_str(" sanitize_address {\n");
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    Ok(result)
}

/// Execute one tool and preserve stdout/stderr on nonzero exit; never invokes a shell.
fn run(command: &mut Command, stage: &str) -> Result<Output, String> {
    let output = command.output().map_err(|e| format!("{stage}: {e}"))?;
    // A nonzero tool status is a build failure, not a successful verification result.
    if !output.status.success() {
        return Err(format!(
            "{stage}: {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output)
}

/// Private temporary directory, removed when its owner leaves scope.
#[derive(Debug)]
pub struct TempDir {
    /// Unique path owned by this handle.
    path: PathBuf,
}

impl TempDir {
    /// Create a unique process-local scratch directory; return OS errors explicitly.
    /// Only compiler/test artifacts live here, never caller-owned source.
    pub fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // Retry name collisions without deleting or reusing another directory's contents.
        loop {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!("holo-core-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("create temporary directory: {e}")),
            }
        }
    }

    /// Borrow the owned path without allocation or transferring cleanup responsibility.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    /// Reclaim only this handle's scratch files; cleanup failure does not mask a build error.
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
