//! Native and independent-model checks for successful early entry completion.
//!
//! Fixtures terminate inside nested storage/control scopes. Test-only LLVM
//! instrumentation tracks backing acquisitions and verifies reverse-order frees
//! before process exit. It changes no data-processing semantics and is never part
//! of compiler output or performance comparisons. O0, O3, and ASan run real code.

#[path = "../support/model/mod.rs"]
mod model;

use holo::check;
use holo::driver::{NativeToolchain, TempDir};
use holo::ir::{Access, Compare, Condition, Expr, Op, Producer, Program, Record};
use holo::layout::LayoutMode;
use std::process::Command;

/// Build a constant equality condition for the selected exit branch.
fn selected() -> Condition {
    Condition {
        op: Compare::Eq,
        left: Expr::Const(0),
        right: Expr::Const(0),
    }
}

/// Return operations that expose any incorrectly continued execution in stdout.
fn stop(value: u64) -> Vec<Op> {
    vec![
        Op::Print(Expr::Const(value)),
        Op::ExitZero,
        Op::Print(Expr::Const(999)),
    ]
}

/// Construct a batch with one published empty record, followed by `body`.
fn batch(name: &str, region: &str, body: Vec<Op>) -> Op {
    let mut operations = vec![Op::Append {
        batch: name.into(),
        producer: "empty".into(),
        args: vec![],
        status: "status".into(),
    }];
    operations.extend(body);
    Op::Batch {
        name: name.into(),
        region: region.into(),
        record: "Empty".into(),
        body: operations,
    }
}

/// Wrap successful acquisition of a small bounded region around supplied work.
fn region(name: &str, body: Vec<Op>) -> Op {
    Op::Region {
        name: name.into(),
        capacity: Expr::Const(64),
        fail_acquire: false,
        body,
        on_failure: vec![Op::Print(Expr::Const(998)), Op::ExitZero],
    }
}

/// Add the common schema/producer definitions to an entry-body fixture.
fn program(body: Vec<Op>) -> Program {
    Program {
        records: vec![Record {
            name: "Empty".into(),
            fields: vec![],
        }],
        producers: vec![Producer {
            name: "empty".into(),
            record: "Empty".into(),
            error: None,
            params: vec![],
            body: vec![],
        }],
        body,
    }
}

/// Nested batch/cursor/repeat/if exits free every acquired owner once, in reverse.
#[test]
fn nested_storage_and_control_flow_exit_matches_model_and_native() {
    let inner = region(
        "inner",
        vec![batch(
            "inner_items",
            "inner",
            vec![
                Op::Repeat {
                    count: Expr::Const(u64::MAX),
                    index: "i".into(),
                    body: vec![Op::If {
                        condition: selected(),
                        then_body: stop(11),
                        else_body: vec![Op::Print(Expr::Const(997))],
                    }],
                },
                Op::Print(Expr::Const(996)),
            ],
        )],
    );
    let input = program(vec![
        region(
            "outer",
            vec![batch(
                "outer_items",
                "outer",
                vec![Op::ForEach {
                    batch: "outer_items".into(),
                    reference: "item".into(),
                    access: Access::Readonly,
                    body: vec![inner, Op::Print(Expr::Const(995))],
                }],
            )],
        ),
        Op::Print(Expr::Const(994)),
    ]);
    compare(&input, &[11]);
}

/// An unacquired inner region must never contribute a cleanup to its failure exit.
#[test]
fn acquisition_failure_exit_cleans_only_successful_enclosing_owners() {
    let input = program(vec![
        region(
            "outer",
            vec![batch(
                "items",
                "outer",
                vec![Op::Region {
                    name: "unavailable".into(),
                    capacity: Expr::Const(64),
                    fail_acquire: true,
                    body: stop(993),
                    on_failure: stop(22),
                }],
            )],
        ),
        Op::Print(Expr::Const(992)),
    ]);
    compare(&input, &[22]);
}

/// A nonexiting sibling must retain the same active cleanup context for later exit.
#[test]
fn sibling_path_and_normal_scope_cleanup_remain_valid() {
    let mut condition = selected();
    condition.left = Expr::Local("index".into());
    condition.right = Expr::Const(1);
    let input = program(vec![
        region("finished", vec![]),
        region(
            "active",
            vec![Op::Repeat {
                count: Expr::Const(3),
                index: "index".into(),
                body: vec![Op::If {
                    condition,
                    then_body: stop(32),
                    else_body: vec![Op::Print(Expr::Const(31))],
                }],
            }],
        ),
    ]);
    compare(&input, &[31, 32]);
}

/// Successful completion also works when there are no active storage scopes.
#[test]
fn entry_exit_without_storage_stops_later_operations() {
    compare(&program(stop(44)), &[44]);
}

/// Compare independent model output and instrumented O0/O3/ASan native execution.
///
/// The same checked input is used in every build. Native helper assertions catch
/// leaked, repeated, or out-of-order backing releases before host exit flushes
/// output. Temporary artifacts are owned by the test directory and removed later.
fn compare(input: &Program, expected: &[u64]) {
    assert_eq!(model::run(input).unwrap(), expected);
    let tools = NativeToolchain::discover().expect("entry-exit tests require LLVM tools");
    let checked = check::check(input).unwrap();
    let scratch = TempDir::new().unwrap();
    // Empty units have a different physical stride in the derived modes, while
    // cleanup remains one reverse action per acquired owner rather than per value.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        assert_eq!(model::run_with_layout(input, layout).unwrap(), expected);
        let llvm = holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        let instrumented = track_backing_cleanup(&llvm);
        // Real execution at both levels checks cleanup control flow independently of
        // optimizations; ASan additionally diagnoses invalid releases and heap access.
        for mode in [0, 1, 2] {
            let executable = scratch.path().join(format!("exit-{layout:?}-{mode}"));
            if mode == 2 {
                tools.build_sanitized(&instrumented, &executable).unwrap();
            } else {
                tools.build(&instrumented, &executable, mode == 1).unwrap();
            }
            let output = Command::new(executable)
                .env("ASAN_OPTIONS", "detect_leaks=0")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "mode {mode}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let actual: Vec<u64> = String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .map(|line| line.parse().unwrap())
                .collect();
            assert_eq!(actual, expected, "mode {mode}");
        }
    }
}

/// Instrument compiler-generated allocation/free/exit calls with test-only guards.
///
/// A bounded native stack records at most sixteen active region pointers. Every
/// free must match its top; exit requires it to be empty. Wrapper calls are added
/// after replacement, so their calls reach the actual host functions unchanged.
fn track_backing_cleanup(llvm: &str) -> String {
    let mut tracked = llvm
        .replace("call ptr @malloc(", "call ptr @test.malloc(")
        .replace("call void @free(", "call void @test.free(")
        .replace("call void @exit(", "call void @test.exit(");
    tracked.push_str(
        r#"
; Test-only backing lifecycle instrumentation; never emitted by the compiler.
@test.owners = internal global [16 x ptr] zeroinitializer
@test.count = internal global i64 0
; Acquire size bytes through the real allocator. Record successful ownership and
; return its base; null has no cleanup action. Guard the bounded test table.
define internal ptr @test.malloc(i64 %size) {
entry:
  %base = call ptr @malloc(i64 %size)
  %present = icmp ne ptr %base, null
  ; An unavailable allocation never enters the active-owner stack.
  br i1 %present, label %acquired, label %done
acquired:
  %count = load i64, ptr @test.count
  %fits = icmp ult i64 %count, 16
  call void @holo.assert(i1 %fits)
  %slot = getelementptr [16 x ptr], ptr @test.owners, i64 0, i64 %count
  store ptr %base, ptr %slot
  %next = add i64 %count, 1
  store i64 %next, ptr @test.count
  br label %done
done:
  ret ptr %base
}
; Release base only when it is the most recently acquired active owner. Reject
; missing, repeated, or out-of-order cleanup before invoking the real host free.
define internal void @test.free(ptr %base) {
entry:
  %count = load i64, ptr @test.count
  %nonempty = icmp ugt i64 %count, 0
  call void @holo.assert(i1 %nonempty)
  %next = sub i64 %count, 1
  %slot = getelementptr [16 x ptr], ptr @test.owners, i64 0, i64 %next
  %expected = load ptr, ptr %slot
  %matches = icmp eq ptr %base, %expected
  call void @holo.assert(i1 %matches)
  store i64 %next, ptr @test.count
  call void @free(ptr %base)
  ret void
}
; Complete with status only after all tracked owners were released. This guard
; never returns and preserves ordinary host exit output flushing.
define internal void @test.exit(i32 %status) noreturn {
entry:
  %count = load i64, ptr @test.count
  %empty = icmp eq i64 %count, 0
  call void @holo.assert(i1 %empty)
  call void @exit(i32 %status)
  unreachable
}
"#,
    );
    tracked
}
