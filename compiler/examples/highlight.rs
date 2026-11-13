//! Reproducible scan/highlight microbenchmark of real source-tooling library APIs.
//!
//! Source loading, corpus replication, coverage checks, allocation of sample
//! storage, and JSON output occur outside timed regions. Scanner/highlighter phases
//! run sequentially with black-boxed checksums and 31 complete raw samples each.
//! The single-threaded host allocator wrapper counts allocations/reallocations;
//! it changes no allocation layouts or ownership. Results concern warm resident
//! lexical work on a repeated corpus, excluding terminal I/O, theme parsing, strict
//! compilation, cold startup, and semantic analysis. They establish no comparison
//! against another implementation and no whole-language performance ranking.

use holo::highlight::{HighlightKind, highlight};
use holo::lex::{LexemeKind, scan};
use std::alloc::{GlobalAlloc, Layout, System};
use std::env;
use std::fs;
use std::hint::black_box;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Number of recorded samples; high quantiles are descriptive order statistics only.
const SAMPLES: usize = 31;
/// Untimed runs for each sequential phase before its sample collection.
const WARMUPS: usize = 5;
/// Minimum replicated resident corpus size, independent of the source's content.
const CORPUS_BYTES: usize = 1024 * 1024;
/// Full corpus traversals per sample to reduce one-clock-read overhead.
const PASSES: usize = 8;
/// Process-wide allocation/reallocation call count; this example runs one thread.
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

/// Transparent system allocator adapter counting allocation-producing API calls.
struct CountingAllocator;

// SAFETY: Every allocation, deallocation, and reallocation delegates unchanged to
// System with the exact pointer/layout/size provided by the GlobalAlloc caller.
// The relaxed counter accesses have no allocation, synchronization, or panic path.
unsafe impl GlobalAlloc for CountingAllocator {
    /// Allocate `layout` through System and count the call, retaining System's null failure.
    /// The caller owns the returned block and must satisfy GlobalAlloc's layout contract.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: The caller's validated GlobalAlloc layout is forwarded without change.
        unsafe { System.alloc(layout) }
    }

    /// Release `pointer` with its original `layout`; no ownership survives this operation.
    /// The caller must supply a live System-allocated block with the matching layout.
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: This wrapper never changes pointers/layouts or substitutes another allocator.
        unsafe { System.dealloc(pointer, layout) }
    }

    /// Resize `pointer` to `size` through System and count the allocation-producing call.
    /// Returns System's pointer/null result; caller ownership follows GlobalAlloc's contract.
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: The original allocation and requested replacement size are forwarded exactly.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

/// Apply the allocation observer only to this example executable, not the compiler library.
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Independently timed lexical operation; both variants consume the complete source.
#[derive(Clone, Copy)]
enum Phase {
    /// Shared lossless lexical scan without styling classifications.
    Scan,
    /// Shared lexical scan plus local highlighting category selection.
    Highlight,
}

/// One phase's complete observations, stored outside all lexical timing loops.
struct Observations {
    /// Ordered elapsed nanoseconds for complete PASSES-corpus samples.
    nanoseconds: Vec<u128>,
    /// Allocation/reallocation calls observed only within measured regions.
    allocations: u64,
    /// Combined observable output checksum, preventing dead-code removal.
    checksum: u64,
}

/// Load an optional source path, validate coverage, measure both phases, and emit JSON.
/// Returns contextual input/output errors; all disposable records are written by stdout redirection.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    let path = arguments
        .next()
        .unwrap_or_else(|| "tests/fixtures/region.holo".into());
    // Unused arguments are rejected so a report cannot silently ignore requested settings.
    if arguments.next().is_some() {
        return Err(
            "usage: cargo run --release -p holo-compiler --example highlight -- [source.holo]"
                .into(),
        );
    }
    let original = fs::read_to_string(&path)?;
    // Nonempty input ensures finite replication and observable source work in every sample.
    if original.is_empty() {
        return Err("source must be nonempty".into());
    }
    let copies = CORPUS_BYTES.div_ceil(original.len());
    let corpus = original.repeat(copies);
    verify(&corpus);
    let scan_results = measure(Phase::Scan, &corpus);
    let highlight_results = measure(Phase::Highlight, &corpus);
    let stdout = io::stdout();
    let mut output = stdout.lock();
    writeln!(output, "{{")?;
    writeln!(output, "  \"source\": {},", json_string(&path))?;
    writeln!(output, "  \"source_bytes\": {},", original.len())?;
    writeln!(
        output,
        "  \"source_fnv1a64\": \"{:016x}\",",
        source_hash(&original)
    )?;
    writeln!(output, "  \"corpus_bytes\": {},", corpus.len())?;
    writeln!(
        output,
        "  \"copies\": {copies}, \"passes_per_sample\": {PASSES}, \"warmups\": {WARMUPS}, \"samples\": {SAMPLES},"
    )?;
    write!(output, "  \"scan\": ")?;
    report(&mut output, &scan_results, corpus.len() * PASSES)?;
    write!(output, ",\n  \"highlight\": ")?;
    report(&mut output, &highlight_results, corpus.len() * PASSES)?;
    writeln!(output, "\n}}")?;
    Ok(())
}

/// Verify exact, nonempty source partitions before interpreting any timing samples.
/// Panics on a coverage/category invariant failure; source bytes remain unchanged.
fn verify(source: &str) {
    let mut end = 0;
    let mut raw = scan(source);
    // Both APIs must cover every byte in exactly the same lexical ranges.
    for styled in highlight(source) {
        let lexical = raw.next().expect("highlight cannot add a lexical range");
        assert_eq!(lexical.span, styled.span);
        assert_eq!(styled.span.start, end);
        assert!(styled.span.end > end);
        assert!(source.get(styled.span.start..styled.span.end).is_some());
        assert!((styled.kind as usize) < HighlightKind::ALL.len());
        end = styled.span.end;
    }
    assert!(raw.next().is_none());
    assert_eq!(end, source.len());
}

/// Traverse all of `source` once with the selected phase and return a black-boxed checksum.
/// No output collection, allocation, I/O, or mutation of source occurs in this function.
fn traverse(phase: Phase, source: &str) -> u64 {
    let mut checksum = 0u64;
    // Phase selection happens once per corpus traversal, outside each token loop.
    match phase {
        Phase::Scan => {
            // Every range and category contributes observable data; none of the
            // scanner's classification result can be discarded as unused work.
            for token in scan(black_box(source)) {
                checksum = checksum
                    .wrapping_add(token.span.end as u64)
                    .wrapping_add(lexeme_id(token.kind));
            }
        }
        Phase::Highlight => {
            // Category identity joins byte extents so classification cannot be optimized away.
            for token in highlight(black_box(source)) {
                checksum = checksum
                    .wrapping_add(token.span.end as u64)
                    .wrapping_add(token.kind as u64);
            }
        }
    }
    black_box(checksum)
}

/// Encode every scanner category, including punctuation identity, into observable output.
/// This fixed mapping allocates nothing and makes no claim about semantic validity.
fn lexeme_id(kind: LexemeKind) -> u64 {
    // Explicit category values keep the checksum independent of Rust enum layout choices.
    match kind {
        LexemeKind::Identifier => 1,
        LexemeKind::Number => 2,
        LexemeKind::Symbol(symbol) => 100 + u64::from(symbol),
        LexemeKind::EqualEqual => 3,
        LexemeKind::NotEqual => 4,
        LexemeKind::LessEqual => 5,
        LexemeKind::GreaterEqual => 6,
        LexemeKind::Whitespace => 7,
        LexemeKind::Newline => 8,
        LexemeKind::LineComment => 9,
        LexemeKind::BlockComment => 10,
        LexemeKind::Invalid => 11,
        LexemeKind::Text => 12,
        LexemeKind::Bytes => 13,
    }
}

/// Warm one phase, then record complete samples and allocation counts sequentially.
/// Sample storage is reserved first; timer/observer overhead is included in elapsed sample time.
fn measure(phase: Phase, source: &str) -> Observations {
    // Untimed warmups exercise the same API and resident corpus as the measured phase.
    for _ in 0..WARMUPS {
        black_box(traverse(phase, source));
    }
    let mut results = Observations {
        nanoseconds: Vec::with_capacity(SAMPLES),
        allocations: 0,
        checksum: 0,
    };
    // Each sample consumes the same finite byte count; all results, including outliers, remain.
    for _ in 0..SAMPLES {
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        let start = Instant::now();
        let mut checksum = 0u64;
        // Multiple full traversals amortize clock-read cost without changing the lexical work.
        for _ in 0..PASSES {
            checksum = checksum.wrapping_add(traverse(phase, source));
        }
        let elapsed = start.elapsed().as_nanos();
        let after = ALLOCATIONS.load(Ordering::Relaxed);
        assert!(elapsed > 0, "sample duration must be measurable");
        results.nanoseconds.push(elapsed);
        results.allocations += after - before;
        results.checksum = results.checksum.wrapping_add(black_box(checksum));
    }
    results
}

/// Emit raw observations and descriptive nearest-rank quantiles for one phase.
/// Sorting and JSON formatting allocate only outside timing; I/O errors propagate.
fn report(output: &mut impl Write, result: &Observations, bytes: usize) -> io::Result<()> {
    let mut ordered = result.nanoseconds.clone();
    ordered.sort_unstable();
    let average = ordered.iter().sum::<u128>() as f64 / ordered.len() as f64;
    let throughput = bytes as f64 * 1e9 / average / (1024.0 * 1024.0);
    write!(
        output,
        "{{\"bytes_per_sample\": {bytes}, \"unit\": \"nanoseconds\", \"avg\": {average:.3}, \"min\": {}, \"p50\": {}, \"p75\": {}, \"p90\": {}, \"p95\": {}, \"p99\": {}, \"p99_9\": {}, \"max\": {}, \"mib_per_second_from_average\": {throughput:.3}, \"measured_allocations\": {}, \"checksum\": {}, \"raw\": {:?}}}",
        ordered[0],
        quantile(&ordered, 0.5),
        quantile(&ordered, 0.75),
        quantile(&ordered, 0.9),
        quantile(&ordered, 0.95),
        quantile(&ordered, 0.99),
        quantile(&ordered, 0.999),
        ordered[ordered.len() - 1],
        result.allocations,
        result.checksum,
        result.nanoseconds
    )
}

/// Select one nearest-rank order statistic from nonempty sorted positive durations.
/// Panics for empty caller input; current callers always supply the fixed SAMPLES count.
fn quantile(ordered: &[u128], fraction: f64) -> u128 {
    let index = ((ordered.len() as f64 * fraction).ceil() as usize).saturating_sub(1);
    ordered[index.min(ordered.len() - 1)]
}

/// Compute a reproducible noncryptographic identity for the original input bytes.
/// This distinguishes corpus content in JSON; external SHA-256 records remain preferable provenance.
fn source_hash(source: &str) -> u64 {
    let mut value = 0xcbf29ce484222325u64;
    // FNV-1a processes each original input byte once outside all timed traversals.
    for byte in source.bytes() {
        value ^= u64::from(byte);
        value = value.wrapping_mul(0x100000001b3);
    }
    value
}

/// Encode a UTF-8 path as a JSON string without third-party dependencies.
/// Allocates only report text; escapes control bytes and leaves printable Unicode intact.
fn json_string(value: &str) -> String {
    use std::fmt::Write as _;
    let mut output = String::from("\"");
    // JSON escapes preserve exact path identity, including platform-valid control characters.
    for character in value.chars() {
        match character {
            '\"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\0'..='\u{1f}' => write!(&mut output, "\\u{:04x}", character as u32)
                .expect("writing to String cannot fail"),
            _ => output.push(character),
        }
    }
    output.push('\"');
    output
}
