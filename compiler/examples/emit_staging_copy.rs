//! Emit candidate LLVM for checked macOS arm64 staging and batch traversal.
//!
//! This compiler API diagnostic keeps the ordinary CLI and parsed language
//! unchanged. Source checking is shared with the baseline, and the caller
//! owns native verification, link inputs, and performance measurement.

use std::{
    env,
    error::Error,
    fs,
    io::{self, Write},
};

use holo::{driver::NativeToolchain, frontend, layout::LayoutMode, llvm};

/// Check a standalone source and emit its selected opt-in Derived64 LLVM.
/// The optional policy flag follows the source path. Bad arguments, invalid
/// source and failed writes exit before native linking.
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let usage = "usage: emit_staging_copy SOURCE [--stable-slots | --proved-slots | --uniform-batches | --outlined-bulk]";
    let source = args.next().ok_or(usage)?;
    // Each opt-in selects one compiler policy on the same checked source.
    let emit = match args.next() {
        None => llvm::emit_with_staging_copy,
        Some(flag) if flag == "--stable-slots" && args.next().is_none() => {
            llvm::emit_with_stable_staging_slots
        }
        Some(flag) if flag == "--proved-slots" && args.next().is_none() => {
            llvm::emit_with_proved_staging_slots
        }
        Some(flag) if flag == "--uniform-batches" && args.next().is_none() => {
            llvm::emit_with_uniform_batch_extents
        }
        Some(flag) if flag == "--outlined-bulk" && args.next().is_none() => {
            llvm::emit_with_outlined_bulk_staging
        }
        Some(_) => return Err(usage.into()),
    };
    let target = NativeToolchain::host_target_triple()?;
    // Fail explicitly so a scalar-only host build cannot be mistaken for a trial.
    if !target.starts_with("arm64-apple-macosx") {
        return Err("staging-copy trial requires native macOS arm64".into());
    }
    let text = fs::read_to_string(source)?;
    let parsed = frontend::parse_experimental(&text)?;
    let checked = parsed.check()?;
    let ir = emit(&checked, target, LayoutMode::Derived64)?;
    io::stdout().lock().write_all(ir.as_bytes())?;
    Ok(())
}
