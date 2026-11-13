//! Build and execute the checked region-core workload, retaining reviewable artifacts.
//!
//! Inputs are an optional fresh output directory and installed LLVM tools. This
//! example constructs IR directly; it does not parse Holo or stand in for a source
//! compiler. Native stdout and compiler timings are recorded separately.
//! Default output lives under build/examples; existing directories are refused.

#[path = "region/program.rs"]
mod program;

use holo::{check, driver::NativeToolchain, llvm};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

/// Own the example's artifacts and report tool failures without a fake successful run.
fn main() -> Result<(), String> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("build/examples/region"));
    // Preserve prior reports and binaries rather than overwriting evidence.
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir(&output)
        .map_err(|e| format!("new output directory {}: {e}", output.display()))?;
    let program = program::program();
    let start = Instant::now();
    let checked = check::check(&program).map_err(|e| e.to_string())?;
    let checking = start.elapsed();
    let tools = NativeToolchain::discover()?;
    let start = Instant::now();
    let ir = llvm::emit(&checked, tools.target_triple());
    let lowering = start.elapsed();
    fs::write(output.join("program.ll"), &ir).map_err(|e| e.to_string())?;
    fs::write(output.join("input.txt"), format!("{program:#?}\n")).map_err(|e| e.to_string())?;
    let mut report = format!(
        "Checked construction IR example; this harness does not parse source.\nChecking: {checking:?}\nLowering: {lowering:?}\nTarget: {}\n{}\n",
        tools.target_triple(),
        tools.identity
    );
    // Compare optimization levels sequentially; these runs validate behavior, not speed.
    for optimized in [false, true] {
        let label = if optimized {
            "optimized"
        } else {
            "unoptimized"
        };
        let binary = output.join(label);
        let timing = tools.build(&ir, &binary, optimized)?;
        let result = Command::new(&binary).output().map_err(|e| e.to_string())?;
        fs::write(output.join(format!("{label}.stdout")), &result.stdout)
            .map_err(|e| e.to_string())?;
        fs::write(output.join(format!("{label}.stderr")), &result.stderr)
            .map_err(|e| e.to_string())?;
        // An assertion failure or signal is never reported as a valid native execution.
        if !result.status.success() {
            return Err(format!(
                "{label}: {}\n{}",
                result.status,
                String::from_utf8_lossy(&result.stderr)
            ));
        }
        report.push_str(&format!(
            "{label}: verify={:?}, codegen/link={:?}\n",
            timing.verification, timing.codegen_and_link
        ));
    }
    fs::write(output.join("report.txt"), report).map_err(|e| e.to_string())?;
    println!("Validated native example artifacts: {}", output.display());
    Ok(())
}
