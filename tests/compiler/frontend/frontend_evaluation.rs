//! Native semantic witnesses for once-only, ordered source expression evaluation.
//!
//! Actual parsed source passes the ordinary checker and LLVM emitter. Replacing
//! only clock calls with a test counter or a distinctive exit makes ordering
//! observable without a foreign data kernel or a new language API. These are
//! correctness checks at O0/O3/ASan, not measurements of clock or helper costs.

use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::parse_experimental;
use holo::layout::LayoutMode;
use std::process::{Command, Output};

/// Closed scalar helpers exercise ordered inputs, repeated use, and unused inputs.
/// All parameters are immutable copies; computation has explicit wrapping semantics.
const HELPERS: &str = r#"
// Subtract the second copied input from the first, modulo 2^64.
func combine
    with noalloc, noblock
    arg first u64 as readonly copy
    arg second u64 as readonly copy
    returns u64
{
    return core.sub_wrapping(first, second)
}
// Repeated parameter use must read one captured argument value twice.
func duplicate
    with noalloc, noblock
    arg value u64 as readonly copy
    returns u64
{
    return core.xor(value, value)
}
// Returning a constant must not remove evaluation of the caller's argument.
func ignore
    with noalloc, noblock
    arg value u64 as readonly copy
    returns u64
{
    return 7
}
"#;

/// Test-only monotonic observations start at one and advance on each invocation.
/// The global counter makes unused and repeated argument evaluation observable.
const COUNTING_TICK: &str = r#"
; Test-only sequencing witness; no data-processing algorithm is supplied here.
@test.counter = internal global i64 0
; Read, increment, and return one test observation without allocation or blocking.
define internal i64 @test.tick() {
entry:
  %previous = load i64, ptr @test.counter
  %next = add i64 %previous, 1
  store i64 %next, ptr @test.counter
  ret i64 %next
}
"#;

/// A test clock that makes reaching it distinguishable from the checked bounds trap.
/// Its deliberate exit is instrumentation only, never an implementation of Holo now().
const EXITING_TICK: &str = r#"
; Exit with a unique code if evaluation reaches this test-only clock observation.
define internal i64 @test.tick() noreturn {
entry:
  call void @exit(i32 99)
  unreachable
}
"#;

/// Ordinary byte formals retain a captured logical extent through nested calls.
const BORROWED_HELPERS: &str = r#"
// Consume one bounded input and one eager copied actual.
func observe
    arg input Bytes as readonly reference
    arg value u64 as readonly copy
    returns u64
{
    if value == 7 {
        return core.add_wrapping(core.len(input), value)
    }
    return 0
}
// Read the first byte with the callee's captured bound.
func first arg input Bytes as readonly reference returns u64 {
    return input[0]
}
// Forward the existing descriptor without widening it to the backing field.
func forward arg input Bytes as readonly reference returns u64 {
    return first(input)
}
"#;

/// A pure wrapping helper whose zero and two-byte calls can specialize.
/// The unused third argument still participates in eager evaluation.
const STATIC_SUM: &str = r#"
func sum arg input Bytes as readonly reference arg seed u64 as readonly copy arg unused u64 as readonly copy returns u64 {
    var total = seed
    // Walk exactly the extent that the caller checked and captured.
    for index in core.range(core.len(input)) {
        total = core.add_wrapping(total, input[index])
    }
    return total
}
"#;

#[test]
/// Empty static selection still evaluates both copied operands exactly once.
fn static_empty_sum_preserves_eager_actuals() {
    let entry = byte_fixture(
        "core.print(sum(core.prefix(item.data, 0), core.now(), core.now()))\ncore.print(core.now())",
    );
    execute_modes(
        &format!("{STATIC_SUM}\n{entry}"),
        COUNTING_TICK,
        |output, mode| {
            assert!(output.status.success(), "{mode}: {output:?}");
            assert_eq!(output.stdout, b"1\n3\n", "{mode}");
        },
    );
}

#[test]
/// A two-byte static selection must check its one-byte backing before later actuals.
fn static_tiny_sum_keeps_prefix_failure_before_later_actuals() {
    let entry = byte_fixture("core.print(sum(core.prefix(item.data, 2), core.now(), 0))");
    execute_modes(
        &format!("{STATIC_SUM}\n{entry}"),
        EXITING_TICK,
        |output, mode| {
            assert!(!output.status.success(), "{mode}: {output:?}");
            assert_ne!(output.status.code(), Some(99), "{mode}: {output:?}");
            assert!(output.stdout.is_empty(), "{mode}: {output:?}");
            assert!(output.stderr.is_empty(), "{mode}: {output:?}");
        },
    );
}

#[test]
/// Returning an empty static seed cannot erase a trap in the unused actual.
fn static_empty_sum_preserves_an_unused_bounds_failure() {
    let entry = byte_fixture("core.print(sum(core.prefix(item.data, 0), 7, item.data[1]))");
    execute_modes(&format!("{STATIC_SUM}\n{entry}"), "", |output, mode| {
        assert!(!output.status.success(), "{mode}: {output:?}");
        assert!(output.stdout.is_empty(), "{mode}: {output:?}");
        assert!(output.stderr.is_empty(), "{mode}: {output:?}");
    });
}

#[test]
fn byte_string_index_traps_even_in_an_unused_scalar_argument() {
    let source = format!(
        "{HELPERS}\n{}",
        r#"
func read arg index u64 as readonly copy returns u64 { return b"abc"[index] }
func unused arg index u64 as readonly copy returns u64 { return ignore(read(index)) }
func main returns u64 {
    core.print(unused(3))
    return 0
}
"#
    );
    execute_modes(&source, "", |output, mode| {
        assert!(!output.status.success(), "{mode}: {output:?}");
        assert!(output.stdout.is_empty(), "{mode}: {output:?}");
    });
}

#[test]
/// Arguments execute once in written order even when duplicated or ignored by a helper.
fn helper_arguments_are_ordered_once_and_not_elided() {
    let source = format!(
        "{HELPERS}\n// Five clock observations must survive helper specialization.\nfunc main returns u64 {{\ncore.print(combine(core.now(), core.now()))\ncore.print(duplicate(core.now()))\ncore.print(ignore(core.now()))\ncore.print(core.now())\nreturn 0\n}}"
    );
    execute_modes(&source, COUNTING_TICK, |output, mode| {
        assert!(output.status.success(), "{mode}: {output:?}");
        // Ticks 1 and 2 produce -1; duplicate consumes tick 3 once; ignore still consumes tick 4.
        assert_eq!(
            output.stdout,
            format!("{}\n0\n7\n5\n", u64::MAX).as_bytes(),
            "{mode}"
        );
    });
}

/// Clock observations witness a single call, lazy arm execution, and forwarding.
/// Bare `on ok` discards the success value while preserving its call and arm work.
#[test]
fn typed_results_evaluate_the_subject_and_selected_arm_once() {
    let source = r#"
error DecodeError { BadMagic; Short }
func decode arg value u64 as readonly copy returns Result<u64, DecodeError> {
    if value == 1 { return error DecodeError.BadMagic }
    return ok value
}
func forward arg value u64 as readonly copy returns Result<u64, DecodeError> {
    return decode(value)
}
func main returns u64 {
    let first = forward(core.now())
        on ok(value) then core.add_wrapping(value, core.now())
        on BadMagic then core.now()
        on error(reason) then core.now()
    let second = forward(core.now())
        on ok(value) then core.add_wrapping(value, core.now())
        on error then core.now()
    let max = decode(18446744073709551615)
        on ok(value) then value
        on error then core.now()
    let accepted = forward(1)
        on ok then true
        on error then false
    let ignored_success = forward(core.now())
        on ok then core.now()
        on error then 99
    if accepted { core.print(99) }
    core.print(first)
    core.print(second)
    core.print(max)
    core.print(ignored_success)
    core.print(core.now())
    return 0
}
"#;
    execute_modes(source, COUNTING_TICK, |output, mode| {
        assert!(output.status.success(), "{mode}: {output:?}");
        assert_eq!(
            output.stdout, b"2\n7\n18446744073709551615\n6\n7\n",
            "{mode}"
        );
    });
}

/// Direct returns share a payload arm while preserving caller scope and eager actuals.
///
/// The two failure sites carry different full-width values into the same arm.
/// Clock observations distinguish evaluating unused actuals from skipped arms.
#[test]
fn guarded_scalar_results_reach_their_handlers_without_a_runtime_tag() {
    let source = r#"
// Two guards can choose the same named payload with different values.
error LimitError { Limit(actual u64); Unused }
func guarded arg value u64 as readonly copy arg ignored u64 as readonly copy returns Result<u64, LimitError> {
    // Each failure keeps its own payload even though the caller shares an arm.
    if value == 1 { return error LimitError.Limit(0) }
    if value == 4 { return error LimitError.Limit(18446744073709551615) }
    return ok value
}
func main returns u64 {
    // An arm can read a caller binding shadowed by the callee's formal.
    let value = 100
    let first = guarded(core.now(), core.now())
        on ok then core.now()
        on Limit(actual) then core.add_wrapping(value, core.add_wrapping(actual, core.now()))
        on Unused then core.now()
    let second = guarded(core.now(), core.now())
        on ok(length) then core.now()
        on Limit(actual) then actual
        on error then core.now()
    let third = guarded(core.now(), core.now())
        on ok(value) then core.add_wrapping(value, core.now())
        on error then core.now()
    core.print(first)
    core.print(second)
    core.print(third)
    core.print(core.now())
    return 0
}
"#;
    let input = parse_experimental(source).unwrap();
    let checked = input.check().unwrap();
    let llvm = holo::llvm::emit(&checked, "arm64-apple-darwin");
    let (_, entry) = llvm.split_once("define i32 @main(").unwrap();
    let (entry, _) = entry.split_once("\n}\n").unwrap();
    assert!(!entry.contains("call { i64, i8 }"));
    assert!(!entry.contains("switch i8"));
    assert!(entry.contains("phi i64 [ 0, %") && entry.contains("[ 18446744073709551615, %"));
    execute_modes(source, COUNTING_TICK, |output, mode| {
        assert!(output.status.success(), "{mode}: {output:?}");
        assert_eq!(
            output.stdout, b"103\n18446744073709551615\n14\n9\n",
            "{mode}"
        );
    });
}

/// A named failure carries all 64 bits through exact forwarding and propagation.
/// Unrelated tag-only errors, ignored payloads and success remain disjoint.
#[test]
fn scalar_error_payloads_survive_forwarding_and_try() {
    let source = r#"
error FrameError { Truncated(required u64); BadMagic }
func read arg kind u64 as readonly copy arg n u64 as readonly copy returns Result<u64, FrameError> {
    if kind == 0 { return error FrameError.Truncated(n) }
    if kind == 1 { return error FrameError.BadMagic }
    return ok n
}
func relay arg kind u64 as readonly copy arg n u64 as readonly copy returns Result<u64, FrameError> {
    let value = try read(kind, n)
    return ok core.add_wrapping(value, 1)
}
func forward arg kind u64 as readonly copy arg n u64 as readonly copy returns Result<u64, FrameError> {
    return relay(kind, n)
}
func main returns u64 {
    // Only the named arm can see the original failure payload and read the clock.
    let observed = forward(0, core.now())
        on ok(value) then core.now()
        on Truncated(required) then core.add_wrapping(required, core.now())
        on BadMagic then core.now()
    let wide_error = forward(0, 18446744073709551615)
        on ok(_) then 99
        on FrameError.Truncated(required) then required
        on BadMagic then 99
    let zero_error = forward(0, 0)
        on ok(_) then 99
        on Truncated(required) then required
        on error then 99
    let wide_success = forward(2, 18446744073709551614)
        on ok(value) then value
        on error then 99
    let ignored = forward(0, 11)
        on ok(_) then 99
        on Truncated(_) then 7
        on error then 99
    let tag_only = forward(1, 11)
        on ok(_) then 99
        on Truncated(_) then 99
        on BadMagic then 8
    core.print(observed)
    core.print(wide_error)
    core.print(zero_error)
    core.print(wide_success)
    core.print(ignored)
    core.print(tag_only)
    core.print(core.now())
    return 0
}
"#;
    execute_modes(source, COUNTING_TICK, |output, mode| {
        assert!(output.status.success(), "{mode}: {output:?}");
        assert_eq!(
            output.stdout, b"3\n18446744073709551615\n0\n18446744073709551615\n7\n8\n3\n",
            "{mode}"
        );
    });
}

#[test]
/// An unused parameter still evaluates its invalid byte argument and traps.
fn ignored_argument_preserves_bounds_failure() {
    bounds_first("core.print(ignore(item.data[1]))");
}

#[test]
/// A complete invalid left argument must fail before evaluation of the next argument.
fn left_argument_bounds_failure_precedes_right_clock() {
    bounds_first("core.print(combine(item.data[1], core.now()))");
}

#[test]
/// A comparison's left read precedes captures introduced by a helper on its right.
fn comparison_left_failure_precedes_right_helper_argument() {
    bounds_first("core.assert(item.data[1] == ignore(core.now()))");
}

#[test]
/// An invalid byte destination must fail before evaluation of replacement-value arguments.
fn assignment_destination_failure_precedes_replacement_argument() {
    bounds_first("item.data[1] = ignore(core.now())");
}

#[test]
/// The opposite order must reach the left clock before an invalid right argument.
fn left_clock_precedes_right_argument_bounds_failure() {
    let source = byte_fixture("core.print(combine(core.now(), item.data[1]))");
    execute_modes(&source, EXITING_TICK, |output, mode| {
        assert_eq!(output.status.code(), Some(99), "{mode}: {output:?}");
        assert!(output.stdout.is_empty(), "{mode}: {output:?}");
    });
}

#[test]
/// Two real borrowed calls update aliased caller state and exclude poisoned bytes.
fn borrowed_calls_reuse_the_same_buffer() {
    let source = include_str!("../../fixtures/borrowed_buffer.holo");
    // Every layout has a different metadata policy for one or both records.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        execute_layout(source, "", layout, |output, mode| {
            assert!(output.status.success(), "{layout:?}/{mode}: {output:?}");
            assert_eq!(output.stdout, b"3\n37\n", "{layout:?}/{mode}");
        });
    }
}

#[test]
/// Prefix and scalar actual expressions execute exactly once in written order.
fn ordinary_arguments_capture_prefix_and_scalar_once() {
    let entry = byte_fixture(
        "core.print(observe(core.prefix(item.data, core.now()), ignore(core.now())))\ncore.print(core.now())",
    );
    execute_modes(
        &format!("{BORROWED_HELPERS}\n{entry}"),
        COUNTING_TICK,
        |output, mode| {
            assert!(output.status.success(), "{mode}: {output:?}");
            assert_eq!(output.stdout, b"8\n3\n", "{mode}");
        },
    );
}

#[test]
/// Invalid prefix bounds precede later scalar helper actuals and callee entry.
fn ordinary_prefix_failure_precedes_later_actual() {
    let entry = byte_fixture(
        "core.print(observe(core.prefix(item.data, core.add_wrapping(core.len(item.data), 1)), ignore(core.now())))",
    );
    execute_modes(
        &format!("{BORROWED_HELPERS}\n{entry}"),
        EXITING_TICK,
        |output, mode| {
            assert!(!output.status.success(), "{mode}: {output:?}");
            assert_ne!(output.status.code(), Some(99), "{mode}: {output:?}");
            assert!(output.stdout.is_empty(), "{mode}: {output:?}");
        },
    );
}

#[test]
/// A forwarded empty prefix traps on index zero despite initialized bytes in backing.
fn ordinary_forwarding_does_not_widen_a_prefix() {
    let entry = byte_fixture("core.print(forward(core.prefix(item.data, 0)))");
    execute_modes(
        &format!("{BORROWED_HELPERS}\n{entry}"),
        "",
        |output, mode| {
            assert!(!output.status.success(), "{mode}: {output:?}");
            assert!(output.stdout.is_empty(), "{mode}: {output:?}");
            assert!(output.stderr.is_empty(), "{mode}: {output:?}");
        },
    );
}

#[test]
/// Nested windows rebase the source and evaluate each bound once in order.
fn ordinary_subrange_preserves_window_and_actual_order() {
    let entry = byte_fixture(
        "core.print(forward(core.subrange(core.subrange(item.data, core.now(), core.now()), 1, 1)))\ncore.print(observe(core.subrange(item.data, 5, 0), ignore(core.now())))\ncore.print(core.now())",
    )
    .replace("core.bytes(1, 7)", "core.bytes(5, core.index())");
    let source = format!("{BORROWED_HELPERS}\n{entry}");
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        execute_layout(&source, COUNTING_TICK, layout, |output, mode| {
            assert!(output.status.success(), "{layout:?}/{mode}: {output:?}");
            assert_eq!(output.stdout, b"2\n7\n4\n", "{layout:?}/{mode}");
        });
    }
}

#[test]
/// A nested window must fit its input extent, even if the backing is larger.
fn ordinary_subrange_cannot_escape_parent_window() {
    let entry = byte_fixture(
        "core.print(observe(core.prefix(core.subrange(core.prefix(item.data, 0), 0, 1), core.now()), ignore(core.now())))",
    );
    execute_modes(
        &format!("{BORROWED_HELPERS}\n{entry}"),
        EXITING_TICK,
        |output, mode| {
            assert!(!output.status.success(), "{mode}: {output:?}");
            assert_ne!(output.status.code(), Some(99), "{mode}: {output:?}");
            assert!(output.stdout.is_empty(), "{mode}: {output:?}");
        },
    );
}

/// A named view evaluates its bounds at binding time, once even when unused.
#[test]
fn named_byte_view_checks_its_bounds_before_the_next_statement() {
    let valid = byte_fixture(
        "let view = core.prefix(item.data, core.now())\ncore.print(core.len(view))\ncore.print(view[0])\ncore.print(core.now())",
    );
    execute_modes(&valid, COUNTING_TICK, |output, mode| {
        assert!(output.status.success(), "{mode}: {output:?}");
        assert_eq!(output.stdout, b"1\n7\n2\n", "{mode}");
    });
    bounds_first("let view = core.prefix(item.data, 2)\ncore.print(core.now())");
}

/// A failed writable index traps before the replacement's clock call.
/// A valid write evaluates its index and eager replacement argument once each.
#[test]
fn writable_view_checks_destination_before_replacement() {
    bounds_first("let readwrite view = core.prefix(item.data, 0)\nview[0] = ignore(core.now())");
    let valid = byte_fixture(
        "let readwrite view = core.prefix(item.data, 1)\nview[core.sub_wrapping(core.now(), 1)] = duplicate(core.now())\ncore.print(view[0])\ncore.print(core.now())",
    );
    execute_modes(&valid, COUNTING_TICK, |output, mode| {
        assert!(output.status.success(), "{mode}: {output:?}");
        assert_eq!(output.stdout, b"0\n3\n", "{mode}");
    });
}

#[test]
/// A wrapped mathematical end is invalid before later actuals or pointer use.
fn ordinary_subrange_rejects_overflowing_end() {
    let entry = byte_fixture(
        "core.print(observe(core.subrange(item.data, 18446744073709551615, 1), ignore(core.now())))",
    );
    execute_modes(
        &format!("{BORROWED_HELPERS}\n{entry}"),
        EXITING_TICK,
        |output, mode| {
            assert!(!output.status.success(), "{mode}: {output:?}");
            assert_ne!(output.status.code(), Some(99), "{mode}: {output:?}");
            assert!(output.stdout.is_empty(), "{mode}: {output:?}");
        },
    );
}

/// A writable byte window keeps its bounds when forwarded into a typed helper.
/// A write through it remains visible after the helper returns a typed failure.
#[test]
fn writable_subrange_stores_at_forwarded_offset_on_typed_failure() {
    let entry = byte_fixture(
        "let outcome = forward_write(core.subrange(item.data, 1, 2))\n        on ok then 0\n        on Stop then 1\ncore.print(outcome)\ncore.print(item.data[1])\ncore.print(item.data[2])\ncore.print(item.data[3])",
    )
    .replace("core.bytes(1, 7)", "core.bytes(4, core.index())");
    let helpers = r#"
error WriteError { Stop }
func write_then_fail arg view Bytes as readwrite reference returns Result<u64, WriteError> {
    view[0] = 257
    return error WriteError.Stop
}

func forward_write arg view Bytes as readwrite reference returns Result<u64, WriteError> {
    return write_then_fail(core.subrange(view, 1, 1))
}
"#;
    execute_modes(&format!("{helpers}\n{entry}"), "", |output, mode| {
        assert!(output.status.success(), "{mode}: {output:?}");
        assert_eq!(output.stdout, b"1\n1\n1\n3\n", "{mode}");
    });
}

/// Constructor windows keep the logical extent through nested destination calls.
/// Tick values also prove scalar actuals and range operands execute in order.
#[test]
fn producer_view_preserves_range_and_actual_order() {
    let source = r#"
struct Wire { data Bytes }
struct Frame { first u64; extent u64; data Bytes }
struct Event { frame Frame }
error FrameError { Empty }

func wire returns Wire {
    return Wire { data: core.bytes(6, core.index()) }
}

func decode arg prefix Bytes as readonly reference arg first u64 as readonly copy
    returns Result<Frame, FrameError> {
    if core.len(prefix) == 0 { return error FrameError.Empty }
    return ok Frame:
        first = first
        extent = core.len(prefix)
        data = core.bytes(core.len(prefix), prefix[core.index()])
}

func event arg first u64 as readonly copy arg input Bytes as readonly reference
    arg keep u64 as readonly copy returns Result<Event, FrameError> {
    return ok Event:
        frame = try decode(core.prefix(input, keep), first)
}

func main returns u64 {
    let source_memory = core.region(256) else { return 0 }
    let wires = core.batch(source_memory, Wire)
    let initialized = core.append(wires, wire())
    core.assert(initialized == 0)
    let source = core.borrow_readonly(wires, 0)
    let destination = core.region(256) else { return 0 }
    let events = core.batch(destination, Event)
    let status = core.append(events, event(core.now(), core.subrange(source.data, core.now(), core.now()), @keep))
        on ok then 0
        on error then 1
    @observe
    return 0
}
"#;
    // Ticks 1, 2 and 3 supply the copied value and input window; tick 4
    // supplies an invalid nested prefix, so the selected error is a trap.
    let invalid = source
        .replace("@keep", "core.now()")
        .replace("@observe", "core.print(status)");
    execute_modes(&invalid, COUNTING_TICK, |output, mode| {
        assert!(!output.status.success(), "{mode}: {output:?}");
        assert!(output.stdout.is_empty(), "{mode}: {output:?}");
    });
    let success = source.replace("@keep", "core.sub_wrapping(core.now(), 2)").replace(
        "@observe",
        "core.assert(status == 0)\n    let published = core.borrow_readonly(events, 0)\n    core.print(published.frame.first)\n    core.print(published.frame.extent)\n    core.print(published.frame.data[0])\n    core.print(published.frame.data[1])\n    core.print(core.now())",
    );
    execute_modes(&success, COUNTING_TICK, |output, mode| {
        assert!(output.status.success(), "{mode}: {output:?}");
        assert_eq!(output.stdout, b"1\n2\n2\n3\n5\n", "{mode}");
    });
}

/// A trapped view prevents later scalar actuals and destination reservation.
#[test]
fn producer_view_rejects_bounds_before_later_actual_and_full_destination() {
    let source = r#"
struct Wire { data Bytes }
struct Frame { value u64 }

func wire returns Wire { return Wire { data: core.bytes(1, 7) } }
func frame arg input Bytes as readonly reference arg later u64 as readonly copy returns Frame {
    return Frame { value: later }
}

func main returns u64 {
    let source_memory = core.region(64) else { return 0 }
    let wires = core.batch(source_memory, Wire)
    let initialized = core.append(wires, wire())
    core.assert(initialized == 0)
    let source = core.borrow_readonly(wires, 0)
    let destination = core.region(0) else { return 0 }
    let frames = core.batch(destination, Frame)
    let status = core.append(frames, frame(core.prefix(source.data, 2), core.now()))
    core.print(status)
    return 0
}
"#;
    execute_modes(source, EXITING_TICK, |output, mode| {
        assert!(!output.status.success(), "{mode}: {output:?}");
        assert_ne!(output.status.code(), Some(99), "{mode}: {output:?}");
        assert!(output.stdout.is_empty(), "{mode}: {output:?}");
    });
}

/// Build one source-owned one-byte payload, then execute the supplied main-body statement.
/// The byte reference is writable so assignment failures test bounds rather than access permission.
fn byte_fixture(statement: &str) -> String {
    format!(
        r#"{HELPERS}
// Inline data has one initialized byte; index one is always out of bounds.
struct Item {{ data Bytes }}
// Construct the fixed one-byte payload directly at its append destination.
func make_item with noblock returns Item {{
    return Item {{ data: core.bytes(1, 7) }}
}}
// Storage setup is valid; only the selected expression's evaluation order can differ.
func main returns u64 {{
    let memory = core.region(64) else {{ return 0 }}
    let items = core.batch(memory, Item)
    let status = core.append(items, make_item())
    core.assert(status == 0)
    let item = core.borrow_readwrite(items, 0)
    {statement}
    return 0
}}
"#
    )
}

/// Require a guard failure before the distinctive clock exit for one source statement.
/// Exact signal numbers vary by host; successful completion or exit 99 is always incorrect.
fn bounds_first(statement: &str) {
    let source = byte_fixture(statement);
    execute_modes(&source, EXITING_TICK, |output, mode| {
        assert!(!output.status.success(), "{mode}: {output:?}");
        assert_ne!(output.status.code(), Some(99), "{mode}: {output:?}");
        assert!(output.stdout.is_empty(), "{mode}: {output:?}");
        // ASan must observe the language guard, never an actual invalid payload memory access.
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("heap-buffer-overflow"),
            "{mode}: {output:?}"
        );
    });
}

/// Parse/check source, replace only emitted clock calls, and execute three independently built modes.
/// `inspect` checks each native result; compiler allocations and temporary binaries remain test-owned.
fn execute_modes(source: &str, tick: &str, inspect: impl Fn(&Output, &str)) {
    execute_layout(source, tick, LayoutMode::Derived64, inspect);
}

/// Run the same evaluation witness under one explicit physical layout.
/// The source and caller-supplied observation hook have no layout-dependent behavior.
fn execute_layout(source: &str, tick: &str, layout: LayoutMode, inspect: impl Fn(&Output, &str)) {
    let input = parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("evaluation.holo", source)));
    let checked = input
        .check()
        .unwrap_or_else(|error| panic!("{}", error.render("evaluation.holo", source)));
    let tools = NativeToolchain::discover().expect("evaluation tests require matched LLVM tools");
    let emitted = holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout)
        .expect("source fixture must fit the selected layout");
    let mut instrumented = emitted.replace("call i64 @holo_now()", "call i64 @test.tick()");
    instrumented.push_str(tick);
    let scratch = TempDir::new().unwrap();
    // Normal and sanitized modes preserve identical source checks and instrumented observations.
    for (mode, optimized, sanitized) in [
        ("o0", false, false),
        ("o3", true, false),
        ("asan", true, true),
    ] {
        let binary = scratch.path().join(mode);
        // ASan has a distinct driver path because function attributes must accompany its link flags.
        let result = if sanitized {
            tools.build_sanitized(&instrumented, &binary)
        } else {
            tools.build(&instrumented, &binary, optimized)
        };
        result.unwrap_or_else(|error| {
            panic!("{mode}: {error}\nsource:\n{source}\nLLVM:\n{instrumented}")
        });
        let output = Command::new(binary)
            .env("ASAN_OPTIONS", "detect_leaks=0")
            .output()
            .unwrap();
        inspect(&output, mode);
    }
}
