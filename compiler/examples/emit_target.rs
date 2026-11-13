//! Emit verified-source LLVM for a selected Linux target through the compiler API.
//!
//! This example lets cross-target diagnostics inspect the same checked program
//! without running a target driver. It only writes LLVM to stdout. Source text
//! and checked structures live in the compiler process, outside benchmark timing.

use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, Write};

use holo::{frontend, layout::LayoutMode, llvm};

/// Parse the supplied source and emit the Derived64 module for one Linux target.
/// Extra arguments and unknown targets fail before code generation. I/O and
/// compiler errors propagate, and the caller owns native verification/linking.
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let target = args.next().ok_or("usage: emit_target TARGET SOURCE")?;
    let source = args.next().ok_or("usage: emit_target TARGET SOURCE")?;
    // Keep this example within the Linux targets supported by the native core.
    if args.next().is_some()
        || !matches!(
            target.as_str(),
            "aarch64-unknown-linux-musl" | "x86_64-unknown-linux-musl"
        )
    {
        return Err("expected one supported Linux musl target and one source".into());
    }
    let text = fs::read_to_string(source)?;
    let parsed = frontend::parse_experimental(&text)?;
    let checked = parsed.check()?;
    let ir = llvm::emit_with_layout(&checked, &target, LayoutMode::Derived64)?;
    io::stdout().lock().write_all(ir.as_bytes())?;
    Ok(())
}
