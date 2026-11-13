//! End-to-end native checks for the fresh construction IR and toolchain driver.
//!
//! These tests compile actual checked input to LLVM and execute O0/O3 and ASan
//! binaries. Expected values are calculated independently from the emitter; an
//! intentionally invalid raw-IR control verifies that ASan actually detects errors.

#[path = "../../compiler/examples/region/program.rs"]
mod program;

use holo::{
    check,
    driver::{NativeToolchain, TempDir},
    llvm,
};
use std::fs;
use std::process::Command;

/// Validate destination calls, stable aliases, all failure tags, and marker reuse
/// in both optimization modes. The audited Darwin arm64 driver must also bind
/// imports before contracted execution; other targets retain their existing link
/// flags. No native timing result is interpreted as a speedup.
#[test]
fn nested_region_core_executes_in_both_modes() {
    let input = program::program();
    let checked = check::check(&input).unwrap();
    let tools = NativeToolchain::discover().unwrap();
    let ir = llvm::emit(&checked, tools.target_triple());
    assert!(!ir.contains("noalias"));
    assert!(!ir.contains("@realloc"));
    let scratch = TempDir::new().unwrap();
    // Each new binary is built independently, preserving checks at both optimization levels.
    for optimized in [false, true] {
        let binary = scratch.path().join(format!("program-{optimized}"));
        let timings = tools.build(&ir, &binary, optimized).unwrap();
        // Eager chained fixups remove a first-call loader path on the explicitly
        // reviewed profile. This command assertion complements the profile
        // audit's actual Mach-O binding-table inspection.
        assert_eq!(
            timings
                .commands
                .iter()
                .any(|command| command.contains("-Wl,-fixup_chains")),
            cfg!(all(target_os = "macos", target_arch = "aarch64"))
        );
        let output = Command::new(binary).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // Entry occupies 32+payload bytes: 34 + 8*32 + (1+...+8) = 326.
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "944\n9\n77\n9\n326\n0\n42\n0\n1\n1\n"
        );
    }
}

/// Exercise the complete acceptance program under real heap-access instrumentation.
#[test]
fn nested_region_core_is_address_sanitized() {
    let input = program::program();
    let checked = check::check(&input).unwrap();
    let tools = NativeToolchain::discover().unwrap();
    let ir = llvm::emit(&checked, tools.target_triple());
    let scratch = TempDir::new().unwrap();
    let binary = scratch.path().join("sanitized");
    tools.build_sanitized(&ir, &binary).unwrap();
    let output = Command::new(binary)
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Prove sanitizer flags/attributes reach function bodies using a known bad control.
/// This invalid IR bypasses the checker only to validate the test tool, not the language.
#[test]
fn sanitizer_control_detects_an_actual_overflow() {
    let tools = NativeToolchain::discover().unwrap();
    let source = format!(
        "target triple = \"{}\"\ndeclare ptr @malloc(i64)\ndefine i32 @main() {{\nentry:\n %p = call ptr @malloc(i64 1)\n %bad = getelementptr i8, ptr %p, i64 1\n store volatile i8 7, ptr %bad, align 1\n ret i32 0\n}}\n",
        tools.target_triple()
    );
    let scratch = TempDir::new().unwrap();
    let binary = scratch.path().join("bad-control");
    tools.build_sanitized(&source, &binary).unwrap();
    let output = Command::new(binary)
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("AddressSanitizer"));
}

/// Ensure a build cannot replace an existing caller artifact, even before codegen.
#[test]
fn native_driver_preserves_existing_outputs() {
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let output = scratch.path().join("keep");
    fs::write(&output, "original").unwrap();
    let ir = format!("target triple = \"{}\"\n", tools.target_triple());
    assert!(
        tools
            .build(&ir, &output, true)
            .unwrap_err()
            .contains("overwrite")
    );
    assert_eq!(fs::read_to_string(output).unwrap(), "original");
}

/// Repeated raw target directives must not hide the effective canonical target.
#[test]
fn native_driver_rejects_a_canonical_target_change() {
    let tools = NativeToolchain::discover().unwrap();
    // Choose a valid different 64-bit target so LLVM can canonicalize the module.
    let other = if tools.target_triple().starts_with("x86_64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let source = format!(
        "target triple = \"{}\"\ntarget triple = \"{other}\"\n",
        tools.target_triple()
    );
    let scratch = TempDir::new().unwrap();
    let output = scratch.path().join("mismatched");
    assert!(
        tools
            .build(&source, &output, true)
            .unwrap_err()
            .contains("target")
    );
    assert!(!output.exists());
}
