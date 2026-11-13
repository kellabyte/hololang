//! Archive and measure the checked-IR region lifecycle on real native execution.
//!
//! Positional inputs are a fresh output directory, record count, reuse rounds,
//! and sample count. Compilation, backing setup, process startup, validation, and
//! output are outside the native clock interval. The interval contains repeated
//! construction, one opaque barrier per cycle, a linear scan, and marker reset.
//! This is a single-core mechanism measurement, not a language ranking.
//! Default output lives under build/examples; provenance includes Cargo config.

#[path = "region/measurement.rs"]
mod measurement;
#[path = "region/program.rs"]
mod program;

use holo::{check, driver::NativeToolchain, llvm};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Validate inputs, build one optimized binary, and archive sequential samples.
///
/// Returns errors for bad input, existing output, tool failures, changed sources,
/// or incorrect native output. It performs host I/O/allocations outside measured
/// execution and preserves failed-run artifacts instead of overwriting them.
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    // Reject extra positional values rather than silently measuring another case.
    if args.len() > 4 {
        return Err(
            "usage: measure_region [new-output-directory] [count] [rounds] [samples]".into(),
        );
    }
    let output = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("build/examples/region-measurement"));
    let count = number(&args, 1, 512, "count")?;
    let rounds = number(&args, 2, 1000, "rounds")?;
    let samples = number(&args, 3, 7, "samples")?;
    // Bound host oracle work, backing size, report size, and accidental long runs.
    if count == 0
        || count > 1_048_576
        || rounds == 0
        || rounds > 1_048_576
        || count
            .checked_mul(rounds)
            .is_none_or(|work| work > 100_000_000)
        || samples == 0
        || samples > 31
    {
        return Err(
            "require count/rounds in 1..=1048576, count*rounds<=100000000, samples in 1..=31"
                .into(),
        );
    }
    // Exclusive directory creation preserves any earlier evidence at this path.
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir(&output)
        .map_err(|e| format!("new output directory {}: {e}", output.display()))?;
    record_host(&output)?;
    let sources = snapshot_sources(&output)?;
    let input = measurement::workload(count, rounds);
    let expected = measurement::expected_checksum(count, rounds);
    fs::write(output.join("input.txt"), format!("{input:#?}\n")).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let checked = check::check(&input).map_err(|e| e.to_string())?;
    let checking = start.elapsed();
    let tools = NativeToolchain::discover()?;
    let start = Instant::now();
    let ir = llvm::emit(&checked, tools.target_triple());
    let lowering = start.elapsed();
    fs::write(output.join("program.ll"), &ir).map_err(|e| e.to_string())?;
    let binary = output.join("native");
    let build = tools.build(&ir, &binary, true)?;
    fs::write(
        output.join("commands.txt"),
        build.commands.join("\n") + "\n",
    )
    .map_err(|e| e.to_string())?;
    let runner = std::env::current_exe().map_err(|e| e.to_string())?;
    fs::copy(&runner, output.join("runner")).map_err(|e| e.to_string())?;
    let mut hashes = String::new();
    // Identify the exact executable/tool inputs as well as emitted and host IR.
    for path in [
        &binary,
        &output.join("runner"),
        &output.join("program.ll"),
        &output.join("input.txt"),
        &tools.opt,
        &tools.clang,
    ] {
        hashes.push_str(&format!("{}  {}\n", sha256(path)?, path.display()));
    }
    let mut raw = String::from("sample\telapsed_ns\tchecksum\tfailure_sum\tfinal_marker\n");
    let mut timings = Vec::new();
    // A separate validated warmup process primes code/pages without entering the
    // recorded sample distribution. Measured processes never overlap each other.
    run_sample(&binary, &output, "warmup", expected)?;
    for sample in 0..samples {
        let elapsed = run_sample(&binary, &output, &format!("sample-{sample}"), expected)?;
        raw.push_str(&format!("{sample}\t{elapsed}\t{expected}\t0\t0\n"));
        timings.push(elapsed);
    }
    // A source edit during a run invalidates source/binary correspondence claims.
    for (source, snapshot) in &sources {
        let current = fs::read(source).map_err(|e| e.to_string())?;
        if current != fs::read(snapshot).map_err(|e| e.to_string())? {
            return Err(format!(
                "source changed during measurement: {}",
                source.display()
            ));
        }
        hashes.push_str(&format!("{}  {}\n", sha256(snapshot)?, snapshot.display()));
    }
    fs::write(output.join("raw.tsv"), raw).map_err(|e| e.to_string())?;
    fs::write(output.join("sha256.txt"), hashes).map_err(|e| e.to_string())?;
    timings.sort_unstable();
    let minimum = timings[0];
    let maximum = timings[timings.len() - 1];
    let middle = timings.len() / 2;
    // Even-sized samples average the two central observations without u64 overflow.
    let median = if timings.len() % 2 == 0 {
        (timings[middle - 1] as u128 + timings[middle] as u128) as f64 / 2.0
    } else {
        timings[middle] as f64
    };
    let report = format!(
        "# Checked-IR region lifecycle measurement\n\n\
         Holo-new experimental checked construction core; this harness does not parse source.\n\n\
         - Records per cycle: {count}\n- Cycles per process: {rounds}\n- Validated measured processes: {samples}\n\
         - Expected checksum: {expected}\n- Logical backing capacity: {} bytes\n\
         - Peak published representation: {} bytes\n- Metadata/scalar bytes per record: 32\n\
         - Target: {}\n- Host OS/architecture: {}/{}\n\n\
         | Native elapsed | Nanoseconds |\n|---|---:|\n| Minimum | {minimum} |\n| Median | {median:.1} |\n| Maximum | {maximum} |\n\n\
         Compiler phases: checking={checking:?}, lowering={lowering:?}, LLVM verification={:?}, codegen/link={:?}.\n\n\
         ```text\n{}\n```\n\n\
         The native interval includes construction, one opaque barrier per lifecycle, linear cursor scanning, and marker reset. \
         Backing acquisition/free, compilation, process startup, oracle checking, and printing are outside it. \
         Byte payload lengths are i&31 and byte values are (i+j)&255; checksums wrap modulo2^64. \
         Every warmup/sample must produce the expected checksum, zero failure sum, and zero final marker.\n\n\
         This is not a C/Rust/Goose comparison, a source-language result, or an isolated reset-cost measurement. \
         No affinity/frequency control, RSS/commit accounting, or hardware counters were collected. \
         Host/tool probes and any unavailable CPU metadata are recorded in host.txt. \
         The logical capacity includes disclosed bounded slack; helper-call overhead and packed u64 metadata remain costs. \
         Native timing variation is reported without a speed ranking. Source snapshots identify observed run inputs; the exact runner/native binaries are retained.\n\n\
         Exact executed tool commands are in commands.txt; full IR/input, source snapshots, stdout/stderr, raw samples, and SHA-256 identities accompany this report.\n",
        count * 64,
        measurement::published_bytes(count),
        tools.target_triple(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        build.verification,
        build.codegen_and_link,
        tools.identity,
    );
    fs::write(output.join("report.md"), report).map_err(|e| e.to_string())?;
    println!(
        "Validated native lifecycle samples: {}; median {median:.1} ns",
        output.display()
    );
    Ok(())
}

/// Parse one optional unsigned positional value without interpreting path bytes.
///
/// `index` addresses `args`, `default` is used only when absent, and `label` names
/// invalid user input. Returns a value or an actionable parse error.
fn number(args: &[OsString], index: usize, default: u64, label: &str) -> Result<u64, String> {
    // Absence selects the documented default; malformed/non-UTF8 text is rejected.
    match args.get(index) {
        None => Ok(default),
        Some(value) => value
            .to_str()
            .ok_or_else(|| format!("{label} must be UTF-8 digits"))?
            .parse()
            .map_err(|e| format!("invalid {label}: {e}")),
    }
}

/// Execute one native process, archive both output streams, and verify its record.
///
/// Returns only positive native elapsed nanoseconds after exact value validation.
/// Signals, malformed output, and wrong values invalidate the run rather than
/// entering the timing distribution.
fn run_sample(binary: &Path, output: &Path, label: &str, expected: u64) -> Result<u64, String> {
    let result = Command::new(binary).output().map_err(|e| e.to_string())?;
    fs::write(output.join(format!("{label}.stdout")), &result.stdout).map_err(|e| e.to_string())?;
    fs::write(output.join(format!("{label}.stderr")), &result.stderr).map_err(|e| e.to_string())?;
    // A failed assertion/signal is never interpreted as a fast successful sample.
    if !result.status.success() {
        return Err(format!(
            "{label} native failure: {}\n{}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let stdout = String::from_utf8(result.stdout).map_err(|e| e.to_string())?;
    parse_record(&stdout, expected).map_err(|error| format!("{label}: {error}"))
}

/// Validate the four-field native observation independently of process handling.
///
/// `stdout` is the complete native output and `expected` the host oracle value.
/// Returns elapsed nanoseconds only for a correct, successful lifecycle record.
fn parse_record(stdout: &str, expected: u64) -> Result<u64, String> {
    let lines: Vec<_> = stdout.lines().collect();
    // Exact ASCII fields reject extra output, signs, Unicode numerals, and blanks.
    if lines.len() != 4
        || lines.iter().any(|line| {
            line.is_empty() || !line.is_ascii() || !line.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err(format!("malformed native record {stdout:?}"));
    }
    let values: Vec<u64> = lines
        .iter()
        .map(|line| line.parse().map_err(|e| format!("native integer: {e}")))
        .collect::<Result<_, _>>()?;
    // All work/results must validate before an elapsed interval becomes a sample.
    if values[0] == 0 || values[1] != expected || values[2] != 0 || values[3] != 0 {
        return Err(format!(
            "native output disagrees with independent oracle: {values:?}, expected checksum={expected}"
        ));
    }
    Ok(values[0])
}

/// Snapshot maintained compiler/workload inputs without importing legacy code.
///
/// Returns source/snapshot pairs for a post-run mutation check. Copying source is
/// compiler/test orchestration, outside all native timing intervals.
fn snapshot_sources(output: &Path) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    // This example belongs to the compiler package; provenance spans the whole
    // workspace because the CLI and native runtime are separate source owners.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler package is a direct workspace child")
        .to_path_buf();
    let mut paths = Vec::new();
    // Include all actual Rust/native compiler inputs without traversing generated
    // target output, archived evidence, or unrelated future Holo libraries.
    for component in ["compiler/src", "cli/src", "runtime"] {
        collect_sources(&root.join(component), &mut paths)?;
    }
    paths.extend(
        [
            "Cargo.toml",
            "Cargo.lock",
            "compiler/Cargo.toml",
            "cli/Cargo.toml",
            ".cargo/config.toml",
            "compiler/examples/measure_region.rs",
            "compiler/examples/region/measurement.rs",
            "compiler/examples/region/program.rs",
        ]
        .map(|path| root.join(path)),
    );
    paths.sort();
    let mut snapshots = Vec::new();
    // Preserve relative paths so each source has one unambiguous owning component.
    for source in paths {
        let relative = source.strip_prefix(&root).map_err(|e| e.to_string())?;
        let snapshot = output.join("sources").join(relative);
        fs::create_dir_all(snapshot.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::copy(&source, &snapshot).map_err(|e| e.to_string())?;
        snapshots.push((source, snapshot));
    }
    Ok(snapshots)
}

/// Collect authored compiler inputs recursively in deterministic host order.
///
/// Only Rust/LLVM/C files are included; generated caches and archived sources are
/// outside this tree. Returns I/O errors instead of silently dropping identities.
fn collect_sources(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    // Each real source subdirectory is traversed once; snapshots are written elsewhere.
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect_sources(&path, paths)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext, "rs" | "ll" | "c"))
        {
            paths.push(path);
        }
    }
    Ok(())
}

/// Obtain a streaming SHA-256 identity using an installed platform tool.
///
/// `path` is passed as a structured process argument, never interpolated in a
/// shell. Returns a verified hex digest or an error if neither tool succeeds.
fn sha256(path: &Path) -> Result<String, String> {
    // These equivalent tools cover the supported macOS/Linux hosts without adding
    // a downloaded compiler dependency or reading large binaries into memory.
    for (tool, flags) in [("shasum", vec!["-a", "256"]), ("sha256sum", vec![])] {
        let result = Command::new(tool).args(flags).arg(path).output();
        if let Ok(result) = result
            && result.status.success()
        {
            let stdout = String::from_utf8(result.stdout).map_err(|e| e.to_string())?;
            let hash = stdout.split_whitespace().next().unwrap_or("");
            if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Ok(hash.to_owned());
            }
        }
    }
    Err(format!("no working SHA-256 tool for {}", path.display()))
}

/// Record best-effort host and Rust toolchain identity outside native timings.
/// Failed CPU queries remain visible instead of being replaced by guessed models.
fn record_host(output: &Path) -> Result<(), String> {
    let mut report = String::new();
    let mut queries = vec![
        ("uname", vec!["-a"]),
        ("rustc", vec!["-Vv"]),
        ("cargo", vec!["-V"]),
    ];
    // Query the native platform without making unsupported sysctl assumptions on Linux.
    if cfg!(target_os = "macos") {
        queries.push(("sysctl", vec!["-n", "machdep.cpu.brand_string"]));
        queries.push((
            "sysctl",
            vec!["-n", "hw.physicalcpu", "hw.logicalcpu", "hw.memsize"],
        ));
    }
    // Metadata failures limit interpretation but do not change the native workload.
    for (tool, args) in queries {
        let mut command = Command::new(tool);
        command.args(args);
        report.push_str(&format!("{command:?}\n"));
        match command.output() {
            Ok(result) => report.push_str(&format!(
                "status={}\n{}{}\n",
                result.status,
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            )),
            Err(error) => report.push_str(&format!("unavailable: {error}\n\n")),
        }
    }
    // Linux exposes CPU descriptions as a readable proc file; preserve its availability result.
    if cfg!(target_os = "linux") {
        match fs::read_to_string("/proc/cpuinfo") {
            Ok(info) => report.push_str(&format!("/proc/cpuinfo\n{info}\n")),
            Err(error) => report.push_str(&format!("/proc/cpuinfo unavailable: {error}\n")),
        }
    }
    fs::write(output.join("host.txt"), report).map_err(|e| e.to_string())
}

/// Validate hand-computed oracle cases and rejection of misleading timing records.
///
/// Tests remain host-only; no native timing campaign runs during `cargo test`.
#[cfg(test)]
mod tests {
    use super::{measurement, parse_record};

    /// Pin independent small logical cases before trusting generated measurements.
    /// Inputs are fixed; assertions return normally only for the exact known sums.
    #[test]
    fn oracle_matches_hand_calculated_values() {
        assert_eq!(measurement::expected_checksum(1, 1), 0);
        assert_eq!(measurement::expected_checksum(2, 3), 6);
        assert_eq!(measurement::expected_checksum(4, 3), 72);
        assert_eq!(measurement::published_bytes(4), 134);
    }

    /// Reject malformed and incorrect observations before interpreting their speed.
    /// The loop covers distinct record failure modes without executing a process.
    #[test]
    fn records_require_positive_time_and_exact_correctness() {
        assert_eq!(parse_record("10\n72\n0\n0\n", 72).unwrap(), 10);
        // Each bad record changes one acceptance obligation from the valid control.
        for invalid in [
            "0\n72\n0\n0\n",
            "10\n73\n0\n0\n",
            "10\n72\n1\n0\n",
            "10\n72\n0\n1\n",
            "10\n72\n0\n0\nextra\n",
            "１０\n72\n0\n0\n",
        ] {
            assert!(parse_record(invalid, 72).is_err());
        }
    }
}
