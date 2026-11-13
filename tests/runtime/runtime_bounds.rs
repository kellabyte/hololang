//! Dynamic bounds failures must terminate before invalid native memory accesses.
//!
//! Unlike static rejection tests, these operations pass the real checker and emit
//! runtime guards. O0/O3 executions must reach the target trap, not libc abort or
//! an arbitrary signal. A guard-selected diagnostic distinguishes the intended
//! invalid access from setup failures. An ASan build additionally ensures the
//! guard precedes an index beyond the actual backing allocation.

use holo::layout::LayoutMode;
use holo::{
    check,
    driver::{NativeToolchain, TempDir},
    ir::*,
    llvm,
};
use std::{fs, os::unix::process::ExitStatusExt, process::Command};

/// Return the observed fatal instruction's signal on the driver's supported hosts.
/// AArch64 LLVM uses BRK/SIGTRAP; x86-64 uses UD2/SIGILL. This is a regression
/// expectation for the selected native targets, not a language-level signal API.
/// No child or target memory is changed while selecting the fixed expectation.
fn target_trap_signal() -> i32 {
    // The native driver admits only these architectures. Any future architecture
    // must define a test expectation explicitly instead of accepting every crash.
    match std::env::consts::ARCH {
        "aarch64" => 5,
        "x86_64" => 4,
        architecture => panic!("native trap signal needs an explicit {architecture} expectation"),
    }
}

/// Build one initialized byte record and append caller-supplied checked operations.
/// The input owns compiler vectors; native storage remains one 64-byte region.
fn sample(operations: Vec<Op>) -> Program {
    let mut body = vec![
        Op::Append {
            batch: "items".into(),
            producer: "make".into(),
            args: vec![],
            status: "status".into(),
        },
        Op::Borrow {
            name: "item".into(),
            batch: "items".into(),
            index: Expr::Const(0),
            access: Access::Readwrite,
        },
    ];
    body.extend(operations);
    Program {
        records: vec![Record {
            name: "Item".into(),
            fields: vec![Field {
                name: "data".into(),
                kind: FieldKind::Bytes,
            }],
        }],
        producers: vec![Producer {
            name: "make".into(),
            record: "Item".into(),
            error: None,
            params: vec![],
            body: vec![Build::Bytes {
                field: "data".into(),
                len: Expr::Const(1),
                value: Expr::Const(7),
            }],
        }],
        body: vec![Op::Region {
            name: "memory".into(),
            capacity: Expr::Const(64),
            fail_acquire: false,
            body: vec![Op::Batch {
                name: "items".into(),
                region: "memory".into(),
                record: "Item".into(),
                body,
            }],
            on_failure: vec![],
        }],
    }
}

/// Reject byte/index operations at the actual boundary in both optimization modes.
#[test]
fn native_bounds_and_assertions_remain_enforced() {
    let cases = [
        Op::Print(Expr::Byte {
            reference: "item".into(),
            path: vec!["data".into()],
            index: Box::new(Expr::Const(1)),
        }),
        Op::WriteByte {
            reference: "item".into(),
            path: vec!["data".into()],
            index: Expr::Const(u64::MAX),
            value: Expr::Const(0),
        },
        Op::Borrow {
            name: "missing".into(),
            batch: "items".into(),
            index: Expr::Const(1),
            access: Access::Readonly,
        },
        Op::Assert(Condition {
            op: Compare::Eq,
            left: Expr::Const(0),
            right: Expr::Const(1),
        }),
    ];
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    // Each invalid dynamic operation remains checked even after aggressive native optimization.
    for (case, operation) in cases.into_iter().enumerate() {
        let input = sample(vec![operation]);
        let checked = check::check(&input).unwrap();
        // Narrow metadata and omitted size headers must not weaken the same dynamic guards.
        for layout in [
            LayoutMode::Wide,
            LayoutMode::Derived64,
            LayoutMode::Compact32,
        ] {
            let ir = llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
            for optimized in [false, true] {
                let binary = scratch
                    .path()
                    .join(format!("invalid-{case}-{layout:?}-{optimized}"));
                tools.build(&ir, &binary, optimized).unwrap();
                let output = Command::new(binary).output().unwrap();
                assert_eq!(
                    output.status.signal(),
                    Some(target_trap_signal()),
                    "invalid operation {case} failed through an unexpected boundary: {}",
                    output.status
                );
                assert!(output.stdout.is_empty());
                assert!(output.stderr.is_empty());
            }
        }
    }
}

/// ASan must see termination from the bounds guard, rather than a heap-overflow report.
#[test]
fn byte_guard_precedes_the_memory_access() {
    let input = sample(vec![Op::Print(Expr::Byte {
        reference: "item".into(),
        path: vec!["data".into()],
        index: Box::new(Expr::Const(4096)),
    })]);
    let checked = check::check(&input).unwrap();
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    // ASan confirms every explicit encoding reaches its guard before a byte overflow.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let ir = llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        let binary = scratch.path().join(format!("guarded-{layout:?}"));
        tools.build_sanitized(&ir, &binary).unwrap();
        // Keep ASan memory checks active but let the actual target SIGILL escape
        // on x86, just as AArch64 SIGTRAP does; sanitizer reporting is not the
        // production fatal boundary tested here.
        let output = Command::new(binary)
            .env("ASAN_OPTIONS", "detect_leaks=0:handle_sigill=0")
            .output()
            .unwrap();
        assert_eq!(output.status.signal(), Some(target_trap_signal()));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("heap-buffer-overflow"));
    }
}

/// Valid boundary cases must retain ordinary execution and initialized byte values.
/// All layouts and both optimization modes share the same independent expected
/// output; this is functional coverage, not a timing or allocation measurement.
#[test]
fn valid_runtime_guards_return_without_trapping() {
    let input = sample(vec![
        Op::Print(Expr::Byte {
            reference: "item".into(),
            path: vec!["data".into()],
            index: Box::new(Expr::Const(0)),
        }),
        Op::WriteByte {
            reference: "item".into(),
            path: vec!["data".into()],
            index: Expr::Const(0),
            value: Expr::Const(9),
        },
        Op::Print(Expr::Byte {
            reference: "item".into(),
            path: vec!["data".into()],
            index: Box::new(Expr::Const(0)),
        }),
        Op::Assert(Condition {
            op: Compare::Eq,
            left: Expr::Const(1),
            right: Expr::Const(1),
        }),
    ]);
    let checked = check::check(&input).unwrap();
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    // The exact byte boundary and true assertion stay valid across physical
    // metadata choices; replacing an unconditional failure is not sufficient.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let ir = llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        // No direct libc termination declaration is available to a generated
        // assertion. The intrinsic is verified and executed by the real driver.
        assert!(ir.contains("declare void @llvm.trap()"));
        assert!(!ir.contains("@abort"));
        for optimized in [false, true] {
            let binary = scratch.path().join(format!("valid-{layout:?}-{optimized}"));
            tools.build(&ir, &binary, optimized).unwrap();
            let output = Command::new(binary).output().unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, b"7\n9\n");
            assert!(output.stderr.is_empty());
        }
    }
}

/// Bind a source guard to its native trap and preserve a distinct diagnostic edge.
/// The original checked module remains ordinary; a separate copy redirects only
/// one byte guard to exit86. Setup traps and protected loads remain unchanged.
#[test]
fn selected_bounds_guard_and_linked_trap_remain_distinct() {
    let input = sample(vec![Op::Print(Expr::Byte {
        reference: "item".into(),
        path: vec!["data".into()],
        index: Box::new(Expr::Const(4096)),
    })]);
    let checked = check::check(&input).unwrap();
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let ir = llvm::emit(&checked, tools.target_triple());
    let (predicate, _) = ir
        .lines()
        .find_map(|line| line.trim().split_once(" = icmp ult i64 4096, "))
        .expect("the intended byte guard must remain identifiable");
    let selected = format!("call void @holo.assert(i1 {predicate})");
    assert_eq!(ir.matches(&selected).count(), 1);
    let observed = ir.replacen(&selected, "call void @exit(i32 86)", 1);
    fs::write(scratch.path().join("original.ll"), &ir).unwrap();
    fs::write(scratch.path().join("observed.ll"), &observed).unwrap();
    // Both ordinary and separately instrumented modules are real verified inputs.
    // The distinctive normal exit cannot be supplied by a random hardware fault.
    for optimized in [false, true] {
        let ordinary = scratch.path().join(format!("ordinary-{optimized}"));
        tools.build(&ir, &ordinary, optimized).unwrap();
        let output = Command::new(&ordinary).output().unwrap();
        assert_eq!(output.status.signal(), Some(target_trap_signal()));
        let disassembler = tools.clang.parent().unwrap().join("llvm-objdump");
        let disassembly = Command::new(disassembler)
            .arg("--disassemble")
            .arg(&ordinary)
            .output()
            .unwrap();
        assert!(disassembly.status.success());
        let decoded = String::from_utf8(disassembly.stdout).unwrap();
        // Match the selected architecture's actual trap spelling; this makes no
        // promise that host debuggers or exception handlers perform no work.
        let instruction = if cfg!(target_arch = "aarch64") {
            "brk"
        } else {
            "ud2"
        };
        assert!(decoded.lines().any(|line| line.contains(instruction)));
        let diagnostic = scratch.path().join(format!("observed-{optimized}"));
        tools.build(&observed, &diagnostic, optimized).unwrap();
        let output = Command::new(diagnostic).output().unwrap();
        assert_eq!(output.status.code(), Some(86));
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}
