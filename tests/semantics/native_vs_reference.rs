//! Differential execution of bounded generated IR against an independent model.
//!
//! Real LLVM executables run at O0 and O3 under all three explicit layout modes.
//! Expected values come from safe test-only
//! semantics, never checker/emitter helpers. Failure artifacts retain complete IR,
//! emitted LLVM, flags, tool identity, expected outputs, and native observations.
//! Native address observations are checked inside each run rather than compared
//! as host-dependent printed integers. These are correctness tests, not timings.

#[path = "../support/edges.rs"]
mod edges;
#[path = "../support/generator.rs"]
mod generator;
#[path = "../support/model/mod.rs"]
mod model;

use holo::driver::{NativeToolchain, TempDir};
use holo::ir::Program;
use holo::layout::LayoutMode;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Enum fields retain their named cases through packed construction and writes.
#[test]
fn enum_record_fields_match_independent_model() {
    let source = r#"
enum Command { Unset; Get; Set }
struct Request { count u64; command Command; payload u64 }

func request returns Request {
    return Request { count: 17, command: Command.Unset, payload: 12345 }
}

func update arg request Request as readwrite reference returns bool {
    var next = Command.Get
    if request.command != Command.Unset { next = Command.Set }
    request.command = next
    let saved = request.command
    next = Command.Unset
    return saved == Command.Set and next != saved
}

func main returns u64 {
    let memory = core.region(64) else { return 0 }
    let rows = core.batch(memory, Request)
    let added = core.append(rows, request())
    core.assert(added == 0)
    let row = core.borrow_readwrite(rows, 0)
    core.assert(row.command == Command.Unset)
    core.assert(not update(row))
    core.assert(row.command == Command.Get)
    core.assert(update(row))
    core.assert(row.command == Command.Set)
    core.print(row.count)
    core.print(row.payload)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("enum-record.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [17, 12345]);
    let checked = parsed.check().unwrap();
    for (mode, expected) in [
        (LayoutMode::Wide, 25),
        (LayoutMode::Derived64, 17),
        (LayoutMode::Compact32, 17),
    ] {
        let plan = holo::layout::LayoutPlan::new(&checked, mode).unwrap();
        assert_eq!(plan.record("Request").fixed_size, Some(expected));
    }
    let toolchain = NativeToolchain::discover().expect("enum tests require LLVM tools");
    compare(&toolchain, "enum-record", program);
}

/// Compare multiple complete generated inputs/layouts at both optimization levels.
#[test]
fn generated_programs_match_independent_model_in_native_builds() {
    let toolchain =
        NativeToolchain::discover().expect("model/native tests require matched LLVM tools");
    // Seeds vary values, finite capacities, and failure ordering. Complete input
    // is retained on failure because a seed cannot identify a future generator.
    for seed in [0, 1, 7, 19, 41, 255, 65_537, u64::MAX] {
        compare(
            &toolchain,
            &format!("seed-{seed}"),
            &generator::program(seed),
        );
    }
}

/// Explicit nested failures preserve published values, marker, and references.
#[test]
fn construction_boundaries_match_independent_model() {
    let toolchain = NativeToolchain::discover().expect("boundary tests require matched LLVM tools");
    compare(
        &toolchain,
        "construction-boundaries",
        &generator::boundaries(),
    );
}

/// Unit successes carry no value, and failure propagation preserves payloads.
#[test]
fn unit_results_match_independent_model() {
    let source = r#"
error ReadError { Invalid(code u64) }
error TaskError { Busy; from ReadError as Read }

func validate arg code u64 as readonly copy returns Result<Unit, ReadError> {
    if code != 0 { return error ReadError.Invalid(code) }
    return ok
}

func saved_forward arg code u64 as readonly copy returns Result<Unit, ReadError> {
    let ok = validate(code)
    return ok
}

func run_task arg code u64 as readonly copy returns Result<u64, TaskError> {
    try saved_forward(0)
    let checked = saved_forward(code)
    try checked
    return ok 17
}

func main returns u64 {
    let direct = validate(0)
        on ok then 5
        on Invalid(code) then code
    core.print(direct)
    let direct_failure = validate(7)
        on ok then 0
        on Invalid(code) then code
    core.print(direct_failure)
    let status = saved_forward(0)
    let ready = status
        on ok then 11
        on Invalid(code) then code
    core.print(ready)
    run_task(0)
        on ok(value) then { core.print(value) }
        on error then { core.assert(false) }
    run_task(9)
        on ok then { core.assert(false) }
        on Read.Invalid(code) then { core.print(code) }
        on Busy then { core.assert(false) }
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("unit-result.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [5, 7, 11, 17, 9]);
    let toolchain = NativeToolchain::discover().expect("Unit result tests require LLVM tools");
    compare(&toolchain, "unit-result", program);
}

/// Scoped writes reach their final record, and both outcomes release the tail.
#[test]
fn writable_scoped_construction_matches_independent_model() {
    let source = r#"
struct Cursor { position u64 }
struct Buffer { data Bytes }
error BufferError { BadSize }

func cursor arg position u64 as readonly copy returns Cursor {
    core.reject_if(position == 9)
    return Cursor { position: position }
}

func buffer returns Result<Buffer, BufferError> {
    return ok Buffer:
        data = core.bytes(4, 0)
}

func write_one
    arg cursor Cursor as readwrite reference
    arg destination Bytes as readwrite reference
    returns u64
{
    destination[cursor.position] = 25
    cursor.position = core.add_wrapping(cursor.position, 1)
    return cursor.position
}

func main returns u64 {
    let cursor_memory = core.region(64) else { return 0 }
    let bytes_memory = core.region(128) else { return 0 }
    core.construct(cursor_memory, cursor(1))
        on ok(readwrite position) then {
            core.construct(bytes_memory, buffer())
                on ok(readwrite output) then {
                    let written = write_one(position, output.data)
                    core.print(written)
                    core.print(output.data[1])
                }
                on error then { core.assert(false) }
            core.print(position.position)
        }
        on error then { core.assert(false) }
    core.construct(cursor_memory, cursor(9))
        on ok then { core.assert(false) }
        on Full then { core.assert(false) }
        on SizeOverflow then { core.assert(false) }
        on error then {
            core.construct(cursor_memory, cursor(3))
                on ok(readwrite retry) then {
                    retry.position = 4
                    core.print(retry.position)
                }
                on error then { core.assert(false) }
        }
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("writable-construct.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [2, 25, 2, 4]);
    let toolchain = NativeToolchain::discover().expect("scoped construction requires LLVM tools");
    compare(&toolchain, "writable-construct", program);
}

/// Empty values, exact capacities, cursor rebinding, and captured prefixes agree.
#[test]
fn empty_records_and_prefix_iteration_match_independent_model() {
    let toolchain = NativeToolchain::discover().expect("edge tests require matched LLVM tools");
    compare(&toolchain, "empty-and-prefix", &edges::program());
}

/// One subject call mutates caller state once, even when its success arm
/// invokes another handler. Both helper-arm returns and entry fallthrough
/// agree with the independent model at each layout and optimization level.
#[test]
fn ordinary_result_blocks_match_independent_model() {
    let source = r#"
struct State { calls u64 }
error ReadError { Missing(required u64); Closed }

func state returns State {
    return State { calls: 0 }
}

func read
    arg state State as readwrite reference
    arg mode u64 as readonly copy
    returns Result<u64, ReadError>
{
    state.calls = core.add_wrapping(state.calls, 1)
    if mode == 0 { return error ReadError.Missing(23) }
    if mode == 1 { return error ReadError.Closed }
    return ok state.calls
}

func normalize arg value u64 as readonly copy returns Result<u64, ReadError> {
    if value == 0 { return error ReadError.Closed }
    return ok core.add_wrapping(value, 10)
}

func read_or_limit
    arg state State as readwrite reference
    arg mode u64 as readonly copy
    returns u64
{
    read(state, mode)
        on ok(count) then {
            normalize(count)
                on ok(value) then { return value }
                on error then { return 90 }
        }
        on Missing(required) then { return required }
        on error then { return 99 }
}

func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let records = core.batch(memory, State)
    let appended = core.append(records, state())
    core.assert(appended == 0)
    let slot = core.borrow_readwrite(records, 0)
    var observed = 0
    read(slot, 2)
        on ok(count) then { observed = count }
        on error then { core.assert(false) }
    core.print(observed)
    core.print(read_or_limit(slot, 0))
    core.print(read_or_limit(slot, 1))
    core.print(read_or_limit(slot, 2))
    core.print(slot.calls)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("result-blocks.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [1, 23, 99, 14, 4]);
    let toolchain = NativeToolchain::discover().expect("result blocks require matched LLVM tools");
    compare(&toolchain, "ordinary-result-blocks", program);
}

/// Returning and handling a saved result preserve its case and payload after input mutation.
#[test]
fn saved_scalar_results_match_independent_model() {
    let source = r#"
struct State { calls u64; value u64 }
error ReadError { Short(required u64); Closed }
error DecodeError { Local; from ReadError as Read }

func initial returns State {
    return State { calls: 0, value: 18446744073709551615 }
}

func read
    arg state State as readwrite reference
    arg mode u64 as readonly copy
    returns Result<u64, ReadError>
{
    state.calls = core.add_wrapping(state.calls, 1)
    if mode == 0 { return error ReadError.Short(state.value) }
    if mode == 1 { return error ReadError.Closed }
    return ok state.value
}

func decode
    arg state State as readwrite reference
    arg mode u64 as readonly copy
    returns Result<u64, DecodeError>
{
    let outcome = read(state, mode)
    state.value = 11
    let value = try outcome
    state.value = 99
    return ok value
}

func decode_and_record
    arg state State as readwrite reference
    arg mode u64 as readonly copy
    returns Result<u64, DecodeError>
{
    let outcome = decode(state, mode)
    state.value = core.add_wrapping(state.value, 1)
    return outcome
}

func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let states = core.batch(memory, State)
    let ready = core.append(states, initial())
    core.assert(ready == 0)
    let state = core.borrow_readwrite(states, 0)
    let failure = decode_and_record(state, 0)
    core.assert(state.value == 12)
    state.value = 99
    let success = decode_and_record(state, 2)
    core.assert(state.value == 100)
    let closed = decode_and_record(state, 1)
    core.assert(state.value == 12)
    state.value = 12
    failure
        on ok then { core.assert(false) }
        on Read.Short(required) then { core.print(required) }
        on error then { core.assert(false) }
    let value = failure
        on ok then 0
        on Read.Short(required) then required
        on error then 1
    core.print(value)
    success
        on ok(value) then { core.print(value) }
        on error then { core.assert(false) }
    closed
        on ok then { core.assert(false) }
        on Read.Short(_) then { core.assert(false) }
        on error then { core.print(17) }
    core.print(state.calls)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("saved-result.holo", source)));
    let program = parsed.program();
    assert_eq!(
        model::run(program).unwrap(),
        [u64::MAX, u64::MAX, 99, 17, 3]
    );
    let toolchain = NativeToolchain::discover().expect("saved results require LLVM tools");
    compare(&toolchain, "saved-scalar-results", program);
}

/// Saved optional returns preserve both ends of the u64 range and call once.
#[test]
fn optional_scalar_returns_match_independent_model() {
    let source = r#"
struct State { calls u64 }

func state returns State {
    return State { calls: 0 }
}

func lookup
    arg state State as readwrite reference
    arg mode u64 as readonly copy
    returns Option<u64>
{
    state.calls = core.add_wrapping(state.calls, 1)
    if mode == 0 { return Some(0) }
    if mode == 1 { return Some(18446744073709551615) }
    return None
}

func forward
    arg state State as readwrite reference
    arg mode u64 as readonly copy
    returns Option<u64>
{
    let outcome = lookup(state, mode)
    return outcome
}

func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let records = core.batch(memory, State)
    let appended = core.append(records, state())
    core.assert(appended == 0)
    let slot = core.borrow_readwrite(records, 0)
    forward(slot, 0)
        on Some(value) then { core.print(value) }
        on None then { core.assert(false) }
    let max = forward(slot, 1)
        on None then 17
        on Some(value) then value
    core.print(max)
    forward(slot, 2)
        on Some(_) then { core.assert(false) }
        on None then { core.print(23) }
    core.print(slot.calls)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("option.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [0, u64::MAX, 23, 3]);
    let toolchain = NativeToolchain::discover().expect("optional returns require LLVM tools");
    compare(&toolchain, "optional-scalar-returns", program);
}

/// Saving a lookup copies its result before its source is edited. Repeated
/// handlers observe the original value, even after a nested option shadows it.
#[test]
fn saved_optional_scalars_match_independent_model() {
    let source = r#"
struct State { calls u64 }
struct Slot { data Bytes }

func state returns State { return State { calls: 0 } }
func slot returns Slot { return Slot { data: core.bytes(1, 7) } }

func lookup
    arg state State as readwrite reference
    arg input Bytes as readonly reference
    returns Option<u64>
{
    state.calls = core.add_wrapping(state.calls, 1)
    if core.len(input) == 0 { return None }
    return Some(input[0])
}

func absent returns Option<u64> { return None }

func default_value returns u64 {
    let saved = absent()
    return saved
        on Some(value) then value
        on None then 23
}

func main returns u64 {
    let state_memory = core.region(128) else { return 0 }
    let states = core.batch(state_memory, State)
    let state_ready = core.append(states, state())
    core.assert(state_ready == 0)
    let tracker = core.borrow_readwrite(states, 0)
    let memory = core.region(128) else { return 0 }
    let slots = core.batch(memory, Slot)
    let slot_ready = core.append(slots, slot())
    core.assert(slot_ready == 0)
    {
        let source = core.borrow_readwrite(slots, 0)
        let saved = lookup(tracker, source.data)
        source.data[0] = 19
        {
            let saved = lookup(tracker, core.prefix(source.data, 0))
            saved
                on Some(_) then { core.assert(false) }
                on None then { core.print(13) }
        }
        let observed = saved
            on None then 17
            on Some(value) then value
        core.print(observed)
        saved
            on Some(value) then { core.print(value) }
            on None then { core.assert(false) }
    }
    core.reset(slots)
    core.print(default_value())
    core.print(tracker.calls)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("saved-option.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [13, 7, 7, 23, 2]);
    let toolchain = NativeToolchain::discover().expect("saved options require LLVM tools");
    compare(&toolchain, "saved-optional-scalars", program);
}

/// An ordinary helper borrows a published prefix by reference. A nested call
/// updates one row, and the same readonly alias sees the update on its next scan.
/// The two differently sized payloads force traversal to respect frozen extents.
#[test]
fn ordinary_batch_borrows_match_independent_model() {
    let source = r#"
struct Row { value u64; data Bytes }

func row
    with noblock
    arg value u64 as readonly copy
    arg length u64 as readonly copy
    returns Row
{
    return Row { value: value, data: core.bytes(length, value) }
}

func sum
    with noalloc, noblock
    arg rows Batch<Row> as readonly reference
    returns u64
{
    var total = core.len(rows)
    for readonly entry in rows {
        total = core.add_wrapping(total, entry.value)
        total = core.add_wrapping(total, core.len(entry.data))
        total = core.add_wrapping(total, entry.data[0])
    }
    return total
}

func update
    with noalloc, noblock
    arg rows Batch<Row> as readwrite reference
    returns u64
{
    let second = core.borrow_readwrite(rows, 1)
    second.value = 11
    second.data[0] = 13
    return core.len(rows)
}

func observe_update
    with noalloc, noblock
    arg view Batch<Row> as readonly reference
    arg writable Batch<Row> as readwrite reference
    returns u64
{
    let before = sum(view)
    let count = update(writable)
    core.assert(count == core.len(view))
    let second = core.borrow_readonly(view, 1)
    core.assert(second.value == 11)
    return core.add_wrapping(before, sum(view))
}

func main returns u64 {
    let memory = core.region(256) else { return 0 }
    let rows = core.batch(memory, Row)
    core.print(sum(rows))
    let first = core.append(rows, row(2, 2))
    let second = core.append(rows, row(3, 5))
    core.assert(first == 0)
    core.assert(second == 0)
    core.print(observe_update(rows, rows))
    let observed = core.borrow_readonly(rows, 1)
    core.print(observed.value)
    core.print(observed.data[0])
    core.print(observed.data[4])
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("batch-borrows.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [0, 56, 11, 13, 3]);
    let toolchain = NativeToolchain::discover().expect("batch borrows require matched LLVM tools");
    compare(&toolchain, "ordinary-batch-borrows", program);
}

/// Compile and run one input at both levels, checking deterministic scalar output.
///
/// `label` identifies replay artifacts. Every error stores complete context before
/// panicking; a missing tool or failed native build is not silently skipped.
fn compare(toolchain: &NativeToolchain, label: &str, program: &Program) {
    // Explicit modes share logical operations/checking while the independent oracle
    // derives each mode's own sizes, failure points, and stable record offsets.
    for mode in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        compare_layout(toolchain, &format!("{label}-{mode:?}"), program, mode);
    }
}

/// A narrowed view stays exact through a whole-result forward and scoped retry.
#[test]
fn producer_view_carries_exact_bounds_through_scoped_construction() {
    let source = r#"
struct Wire { data Bytes }
struct Frame { payload Bytes }
error DecodeError { NeedMore(required u64) }

func wire returns Wire {
    return Wire { data: core.bytes(6, core.index()) }
}

func decode arg input Bytes as readonly reference returns Result<Frame, DecodeError> {
    if core.len(input) < 2 { return error DecodeError.NeedMore(2) }
    return ok Frame:
        payload = core.bytes(core.len(input), input[core.index()])
}

func forward arg input Bytes as readonly reference returns Result<Frame, DecodeError> {
    return decode(core.subrange(input, 1, core.sub_wrapping(core.len(input), 1)))
}

func main returns u64 {
    let source_memory = core.region(128) else { return 0 }
    let wires = core.batch(source_memory, Wire)
    let initialized = core.append(wires, wire())
    core.assert(initialized == 0)
    let source = core.borrow_readonly(wires, 0)
    let frames = core.region(128) else { return 0 }

    core.construct(frames, forward(core.prefix(source.data, 2)))
        on ok then { core.assert(false) }
        on Rejected NeedMore(required) then {
            core.assert(core.used(frames) == 0)
            core.print(required)
        }
        on error then { core.assert(false) }

    core.construct(frames, forward(core.subrange(source.data, 2, 3)))
        on ok(frame) then {
            core.print(core.len(frame.payload))
            core.print(frame.payload[0])
            core.print(frame.payload[1])
        }
        on error then { core.assert(false) }
    core.assert(core.used(frames) == 0)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source).unwrap();
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [2, 2, 3, 4]);
    let toolchain = NativeToolchain::discover().expect("producer view tests require LLVM tools");
    compare(&toolchain, "producer-byte-view", program);
}

/// A local view keeps its captured origin and bound after source rebinding.
/// It still observes writes through a live alias and forwards the exact range.
#[test]
fn local_byte_views_match_independent_model() {
    let source = r#"
struct Wire { data Bytes }

func wire arg first u64 as readonly copy returns Wire {
    return Wire { data: core.bytes(6, core.add_wrapping(first, core.index())) }
}

func inspect arg input Bytes as readonly reference returns u64 {
    let tail = core.subrange(input, 1, 2)
    core.assert(core.len(tail) == 2)
    return tail[1]
}

func copy_wire arg input Bytes as readonly reference returns Wire {
    return Wire { data: core.bytes(core.len(input), input[core.index()]) }
}

func main returns u64 {
    let memory = core.region(256) else { return 0 }
    let wires = core.batch(memory, Wire)
    let first_status = core.append(wires, wire(10))
    let second_status = core.append(wires, wire(20))
    core.assert(first_status == 0)
    core.assert(second_status == 0)
    let writable = core.borrow_readwrite(wires, 0)
    let second = core.borrow_readonly(wires, 1)
    var selected = core.borrow_readonly(wires, 0)
    var count = 3
    let view = core.subrange(selected.data, 1, count)
    selected = second
    count = 1
    writable.data[2] = 99
    core.print(core.len(view))
    core.print(view[0])
    core.print(view[1])
    core.print(inspect(view))
    let empty = core.subrange(view, 3, 0)
    core.print(core.len(empty))
    let copied = core.append(wires, copy_wire(view))
    core.assert(copied == 0)
    let copy = core.borrow_readonly(wires, 2)
    core.print(core.len(copy.data))
    core.print(copy.data[1])
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("named-byte-view.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [3, 11, 99, 13, 0, 3, 99]);
    let toolchain = NativeToolchain::discover().expect("local byte views require LLVM tools");
    compare(&toolchain, "local-byte-views", program);
}

/// Writable views keep their original address after reference rebinding and
/// lend only their captured extent to both writable and readonly callees.
#[test]
fn writable_byte_views_match_independent_model() {
    let source = r#"
struct Wire { data Bytes }

func wire arg first u64 as readonly copy returns Wire {
    return Wire { data: core.bytes(6, core.add_wrapping(first, core.index())) }
}

func inspect arg input Bytes as readonly reference returns u64 {
    core.assert(core.len(input) == 1)
    return input[0]
}

func replace arg input Bytes as readwrite reference returns u64 {
    core.assert(core.len(input) == 2)
    let readwrite suffix = core.subrange(input, 1, 1)
    suffix[0] = core.add_wrapping(suffix[0], 256)
    suffix[0] = core.add_wrapping(suffix[0], 5)
    return inspect(suffix)
}

func main returns u64 {
    let memory = core.region(256) else { return 0 }
    let wires = core.batch(memory, Wire)
    let first_status = core.append(wires, wire(10))
    let second_status = core.append(wires, wire(20))
    core.assert(first_status == 0)
    core.assert(second_status == 0)
    let first = core.borrow_readonly(wires, 0)
    let second = core.borrow_readwrite(wires, 1)
    var selected = core.borrow_readwrite(wires, 0)
    var count = 3
    let readwrite view = core.subrange(selected.data, 1, count)
    let observed = core.prefix(view, 1)
    selected = second
    count = 1
    view[0] = 257
    core.print(observed[0])
    let readwrite prefix = core.prefix(view, 2)
    core.print(replace(prefix))
    core.print(first.data[2])
    core.print(selected.data[2])
    core.print(core.len(view))
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("writable-byte-view.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [1, 17, 17, 22, 3]);
    let toolchain = NativeToolchain::discover().expect("local writable views require LLVM tools");
    compare(&toolchain, "writable-byte-views", program);
}

/// Direct and forwarded boolean calls each mutate caller state once. A saved
/// boolean still controls the branch after the original state has changed.
#[test]
fn ordinary_booleans_match_independent_model() {
    let source = r#"
struct State { calls u64 }

func state returns State { return State { calls: 0 } }

func next_is
    arg state State as readwrite reference
    arg expected u64 as readonly copy
    returns bool
{
    state.calls = core.add_wrapping(state.calls, 1)
    if expected == 0 { return false }
    return state.calls == expected
}

func forward arg state State as readwrite reference returns bool {
    return next_is(state, 1)
}

func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let records = core.batch(memory, State)
    let appended = core.append(records, state())
    core.assert(appended == 0)
    let slot = core.borrow_readwrite(records, 0)
    let saved = forward(slot)
    if next_is(slot, 2) {
        core.print(7)
    }
    if next_is(slot, 0) {
        core.assert(false)
    }
    if saved {
        core.print(slot.calls)
    }
    core.assert(saved)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("ordinary-boolean.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [7, 3]);
    let toolchain = NativeToolchain::discover().expect("boolean returns require LLVM tools");
    compare(&toolchain, "ordinary-booleans", program);
}

/// Logical operators guard trapping reads and run selected calls once in order.
#[test]
fn short_circuit_booleans_match_independent_model() {
    let source = r#"
error DecodeError { Bad }
struct State { calls u64 }
struct Wire { data Bytes }
struct Decoded { value u64 }

func state returns State { return State { calls: 0 } }
func wire returns Wire { return Wire { data: core.bytes(1, 11) } }

func identity arg value u64 as readonly copy returns u64 {
    return value
}

func has_accepted_start arg input Bytes as readonly reference returns bool {
    return core.len(input) > 0 and (identity(input[0]) == 11 or input[0] == 22)
}

func decode arg input Bytes as readonly reference returns Result<Decoded, DecodeError> {
    if core.len(input) == 0 or not input[0] == 11 { return error DecodeError.Bad }
    return ok Decoded:
        value = input[0]
}

func next_is
    arg state State as readwrite reference
    arg expected u64 as readonly copy
    returns bool
{
    state.calls = core.add_wrapping(state.calls, 1)
    return state.calls == expected
}

func main returns u64 {
    let state_memory = core.region(128) else { return 0 }
    let records = core.batch(state_memory, State)
    let state_status = core.append(records, state())
    core.assert(state_status == 0)
    let slot = core.borrow_readwrite(records, 0)
    let wire_memory = core.region(128) else { return 0 }
    let wires = core.batch(wire_memory, Wire)
    let wire_status = core.append(wires, wire())
    core.assert(wire_status == 0)
    let item = core.borrow_readonly(wires, 0)
    if not has_accepted_start(core.prefix(item.data, 0)) and has_accepted_start(item.data) {
        core.print(5)
    }
    let result_memory = core.region(128) else { return 0 }
    let decoded = core.batch(result_memory, Decoded)
    let empty_status = core.append(decoded, decode(core.prefix(item.data, 0)))
        on ok then 1
        on Rejected DecodeError.Bad then 2
        on error then 3
    core.assert(empty_status == 2 and core.len(decoded) == 0)
    let full_status = core.append(decoded, decode(item.data))
        on ok then 0
        on error then 1
    core.assert(full_status == 0 and core.len(decoded) == 1)
    let saved = next_is(slot, 1) and (next_is(slot, 2) or next_is(slot, 0))
    core.assert(saved)
    core.assert(next_is(slot, 0) or next_is(slot, 4))
    core.assert(not next_is(slot, 0) and not next_is(slot, 0))
    core.assert(not slot.calls == 5 and slot.calls == 6 or false)
    core.print(slot.calls)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("short-circuit.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [5, 6]);
    let toolchain =
        NativeToolchain::discover().expect("short-circuit conditions require LLVM tools");
    compare(&toolchain, "short-circuit-booleans", program);
}

/// Static literal contents survive narrowing, forwarding, and producer capture.
#[test]
fn byte_literals_match_independent_model() {
    let source = r#"
struct Row { data Bytes }

func copy arg input Bytes as readonly reference returns Row {
    return Row { data: core.bytes(core.len(input), input[core.index()]) }
}

func from_literal returns Row {
    return copy(core.subrange(b"a\0\xFF", 1, 2))
}

func literal_byte arg index u64 as readonly copy returns u64 {
    return b"a\0\xFF"[index]
}

func inspect arg input Bytes as readonly reference returns u64 {
    if core.len(input) == 0 { return 77 }
    return input[0]
}

func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let rows = core.batch(memory, Row)
    let status = core.append(rows, from_literal())
    core.assert(status == 0)
    let row = core.borrow_readonly(rows, 0)
    core.print(row.data[1])
    let named = b"a\0\xFF"
    let window = core.subrange(core.subrange(named, 1, 2), 1, 1)
    core.print(inspect(window))
    core.print(inspect(b""))
    core.print(inspect(core.subrange(named, 3, 0)))
    core.print(literal_byte(1))
    core.print(literal_byte(0))
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("byte-literals.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [255, 255, 77, 77, 0, 97]);
    let toolchain = NativeToolchain::discover().expect("byte literals require LLVM tools");
    let checked = holo::check::check(program).unwrap();
    let llvm = holo::llvm::emit(&checked, toolchain.target_triple());
    assert_eq!(
        llvm.matches("private unnamed_addr constant [3 x i8] c\"\\61\\00\\FF\"")
            .count(),
        1
    );
    assert_eq!(
        llvm.matches("private unnamed_addr constant [1 x i8] zeroinitializer")
            .count(),
        1
    );
    compare(&toolchain, "byte-literals", program);
}

/// Boolean actuals evaluate once in written order alongside other actual kinds.
#[test]
fn boolean_parameters_match_independent_model() {
    let source = r#"
struct State { calls u64 }
func state returns State { return State { calls: 0 } }

func count arg state State as readwrite reference returns u64 {
    state.calls = core.add_wrapping(state.calls, 1)
    return state.calls
}

func next_is
    arg state State as readwrite reference
    arg expected u64 as readonly copy
    returns bool
{
    return count(state) == expected
}

func both
    arg first bool as readonly copy
    arg middle u64 as readonly copy
    arg last bool as readonly copy
    returns bool
{
    return first and middle == 2 and last
}

func forward
    arg first bool as readonly copy
    arg middle u64 as readonly copy
    arg last bool as readonly copy
    returns bool
{
    return both(first, middle, last)
}

func unused
    arg enabled bool as readonly copy
    returns u64
{
    return 11
}

func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let records = core.batch(memory, State)
    let status = core.append(records, state())
    core.assert(status == 0)
    let slot = core.borrow_readwrite(records, 0)
    let forwarded = forward(next_is(slot, 1), count(slot), next_is(slot, 3) or next_is(slot, 0))
    core.assert(forwarded)
    core.print(unused(next_is(slot, 0)))
    core.print(slot.calls)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("boolean-parameters.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [11, 4]);
    let toolchain = NativeToolchain::discover().expect("boolean parameters require LLVM tools");
    let checked = holo::check::check(program).unwrap();
    let llvm = holo::llvm::emit(&checked, toolchain.target_triple());
    assert!(llvm.contains("(i1 %arg0, i64 %arg1, i1 %arg2)"));
    compare(&toolchain, "boolean-parameters", program);
}

/// Rebinding in branches and loops preserves boolean values and evaluates the
/// right-hand side once, including short-circuit calls with observable effects.
#[test]
fn mutable_booleans_match_independent_model() {
    let source = r#"
struct State { calls u64 }
func state returns State { return State { calls: 0 } }

func next_is
    arg state State as readwrite reference
    arg expected u64 as readonly copy
    returns bool
{
    state.calls = core.add_wrapping(state.calls, 1)
    return state.calls == expected
}

func toggle arg initial bool as readonly copy returns bool {
    var flag = initial
    if flag {
        flag = false
    } else {
        flag = true
    }
    return flag
}

func choice arg flag bool as readonly copy returns Option<u64> {
    if flag { return Some(1) }
    return None
}

func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let records = core.batch(memory, State)
    let status = core.append(records, state())
    core.assert(status == 0)
    let slot = core.borrow_readwrite(records, 0)
    var pending = next_is(slot, 1)
    let saved = pending
    for index in core.range(3) {
        if pending { core.print(index) }
        pending = toggle(pending)
    }
    core.assert(not pending and saved)
    pending = pending and next_is(slot, 0) or next_is(slot, 2)
    core.assert(pending)
    var answered = choice(pending)
        on Some(_) then true
        on None then false
    core.assert(answered)
    answered = toggle(answered)
    core.assert(not answered)
    core.print(slot.calls)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("mutable-booleans.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [0, 2, 2]);
    let toolchain = NativeToolchain::discover().expect("mutable booleans require LLVM tools");
    compare(&toolchain, "mutable-booleans", program);
}

/// Boolean fields use one byte and keep neighboring packed fields intact.
#[test]
fn boolean_record_fields_match_independent_model() {
    let source = r#"
struct State {
    enabled bool
    count u64
}

struct Row {
    state State
    marker bool
    payload Bytes
}

func state arg count u64 as readonly copy returns State {
    return State {
        enabled: count > 0 and not false,
        count: count
    }
}

func row arg count u64 as readonly copy returns Row {
    return Row {
        state: state(count),
        marker: true,
        payload: core.bytes(2, 42)
    }
}

func toggle arg state State as readwrite reference returns bool {
    state.enabled = not state.enabled
    return state.enabled
}

func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let rows = core.batch(memory, Row)
    for count in core.range(2) {
        let status = core.append(rows, row(count))
        core.assert(status == 0)
    }
    let first = core.borrow_readwrite(rows, 0)
    let second = core.borrow_readonly(rows, 1)
    let initially_enabled = first.state.enabled
    core.assert(not initially_enabled and second.state.enabled)
    first.state.enabled = second.state.enabled
    first.marker = not second.marker
    core.assert(first.state.enabled and not first.marker)
    core.print(core.used(memory))
    for readonly item in rows {
        core.print(item.state.count)
        core.print(item.payload[0])
    }
    let scratch = core.region(32) else { return 0 }
    core.construct(scratch, state(5))
        on ok(readwrite item) then {
            core.assert(not toggle(item))
            core.print(item.count)
        }
        on error then { core.assert(false) }
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("boolean-fields.holo", source)));
    let program = parsed.program();
    let toolchain = NativeToolchain::discover().expect("boolean fields require LLVM tools");
    // Two rows include one nested bool and one top-level bool per row.
    for (mode, used) in [
        (LayoutMode::Wide, 72),
        (LayoutMode::Derived64, 40),
        (LayoutMode::Compact32, 32),
    ] {
        assert_eq!(
            model::run_with_layout(program, mode).unwrap(),
            [used, 0, 42, 1, 42, 5]
        );
        compare_layout(
            &toolchain,
            &format!("boolean-fields-{mode:?}"),
            program,
            mode,
        );
    }
}

/// Each transfer targets the nearest loop and restores the storage it leaves.
#[test]
fn loop_transfers_restore_scoped_storage() {
    let source = r#"
struct Row { value u64 }
error RowError { Invalid }

func row arg value u64 as readonly copy returns Row {
    return Row { value: value }
}

func scoped_row arg value u64 as readonly copy returns Result<Row, RowError> {
    return ok Row:
        value = value
}

func counted arg stop u64 as readonly copy returns bool {
    var total = 0
    var visits = 0
    for index in core.range(6) {
        visits = core.add_wrapping(visits, 1)
        core.assert(visits <= 6)
        if index == 1 { continue }
        if index == stop { break }
        total = core.add_wrapping(total, index)
    }
    return total == 5
}

func main returns u64 {
    core.assert(counted(4))
    let batch_memory = core.region(64) else { return 0 }
    let record_memory = core.region(64) else { return 0 }
    var iteration = 0
    loop {
        iteration = core.add_wrapping(iteration, 1)
        core.assert(iteration <= 2)
        core.assert(core.used(batch_memory) == 0)
        core.assert(core.used(record_memory) == 0)
        let scratch = core.region(16) else { break }
        core.assert(core.used(scratch) == 0)
        let rows = core.batch(batch_memory, Row)
        for index in core.range(4) {
            let status = core.append(rows, row(index))
            core.assert(status == 0)
        }
        var visits = 0
        for readonly entry in rows {
            visits = core.add_wrapping(visits, 1)
            core.assert(visits <= 4)
            if entry.value == 1 { continue }
            if entry.value == 3 { break }
            core.print(entry.value)
        }
        core.construct(record_memory, scoped_row(iteration))
            on ok(record) then {
                core.print(record.value)
                if iteration == 1 { continue }
                break
            }
            on error then { core.assert(false) }
        core.assert(false)
    }
    core.assert(iteration == 2)
    core.assert(core.used(batch_memory) == 0)
    core.assert(core.used(record_memory) == 0)
    return 0
}
"#;
    let parsed = holo::frontend::parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render("loop-transfers.holo", source)));
    let program = parsed.program();
    assert_eq!(model::run(program).unwrap(), [0, 2, 1, 0, 2, 2]);
    let toolchain = NativeToolchain::discover().expect("loop transfers require LLVM tools");
    compare(&toolchain, "loop-transfers", program);
}

/// Compare one explicit layout at both optimization levels without sharing its plan.
/// Complete layout-labeled replay evidence is retained by the existing failure path.
fn compare_layout(toolchain: &NativeToolchain, label: &str, program: &Program, mode: LayoutMode) {
    let model_result = if mode == LayoutMode::Wide {
        model::run(program)
    } else {
        model::run_with_layout(program, mode)
    };
    let expected = model_result.unwrap_or_else(|error| {
        fail(
            label,
            program,
            "",
            toolchain,
            false,
            &[],
            &format!("model failed: {error}"),
        )
    });
    let checked = holo::check::check(program).unwrap_or_else(|error| {
        fail(
            label,
            program,
            "",
            toolchain,
            false,
            &expected,
            &format!("valid generated input rejected: {error}"),
        )
    });
    let llvm = holo::llvm::emit_with_layout(&checked, toolchain.target_triple(), mode)
        .unwrap_or_else(|error| {
            fail(
                label,
                program,
                "",
                toolchain,
                false,
                &expected,
                &format!("layout failed: {error}"),
            )
        });
    // The input and expectations are identical across optimization levels; a
    // backend-dependent numeric or alias interpretation is a correctness failure.
    for optimized in [false, true] {
        let scratch = TempDir::new().expect("create native model-test scratch directory");
        let executable = scratch.path().join("model native executable");
        toolchain
            .build(&llvm, &executable, optimized)
            .unwrap_or_else(|error| {
                fail(
                    label,
                    program,
                    &llvm,
                    toolchain,
                    optimized,
                    &expected,
                    &format!("native build failed: {error}"),
                )
            });
        let output = Command::new(&executable).output().unwrap_or_else(|error| {
            fail(
                label,
                program,
                &llvm,
                toolchain,
                optimized,
                &expected,
                &format!("native launch failed: {error}"),
            )
        });
        let observations = format!(
            "status: {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // A trap cannot be mistaken for a successful empty/matching output. The
        // generated positive fixtures are defined and expected to finish normally.
        if !output.status.success() {
            fail(
                label,
                program,
                &llvm,
                toolchain,
                optimized,
                &expected,
                &observations,
            );
        }
        let actual = parse_output(&output.stdout).unwrap_or_else(|error| {
            fail(
                label,
                program,
                &llvm,
                toolchain,
                optimized,
                &expected,
                &format!("{error}\n{observations}"),
            )
        });
        // Exact ordered values expose status, mutation, byte decoding, and scan
        // mismatches that a single final checksum could accidentally conceal.
        if actual != expected {
            fail(
                label,
                program,
                &llvm,
                toolchain,
                optimized,
                &expected,
                &observations,
            );
        }
    }
}

/// Parse only unsigned ASCII decimal lines produced by native Print operations.
fn parse_output(bytes: &[u8]) -> Result<Vec<u64>, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("non-UTF8 native output: {error}"))?;
    // A Print operation includes its newline; incomplete output must not compare
    // equal after a parser silently accepts a truncated final record.
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err("native output ended in an incomplete decimal line".into());
    }
    let mut values = Vec::new();
    // Reject malformed records rather than filtering unexpected diagnostics or
    // converting signed/overflowed values into apparently valid expected output.
    for line in text.split_terminator('\n') {
        // Each print is exactly one unsigned decimal value; whitespace, signs,
        // and blank lines would conceal a different native-output contract.
        if line.is_empty() || !line.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("malformed native output line: {line:?}"));
        }
        values.push(
            line.parse::<u64>()
                .map_err(|error| format!("native value out of range: {error}"))?,
        );
    }
    Ok(values)
}

/// Preserve replay evidence for a failing case, then stop the test explicitly.
///
/// Artifact I/O is test-only and failure paths never modify the original input.
/// A failed artifact write is reported rather than pretending evidence was saved.
fn fail(
    label: &str,
    program: &Program,
    llvm: &str,
    toolchain: &NativeToolchain,
    optimized: bool,
    expected: &[u64],
    detail: &str,
) -> ! {
    let saved = save_failure(label, program, llvm, toolchain, optimized, expected, detail);
    // Keep the original discrepancy in the panic even when the filesystem cannot
    // retain evidence; otherwise an I/O error would mask the compiler/model bug.
    match saved {
        Ok(path) => panic!("{label}: {detail}\nreplay evidence: {}", path.display()),
        Err(error) => panic!("{label}: {detail}\nfailed to save replay evidence: {error}"),
    }
}

/// Write a unique generated-artifact directory containing full replay inputs.
fn save_failure(
    label: &str,
    program: &Program,
    llvm: &str,
    toolchain: &NativeToolchain,
    optimized: bool,
    expected: &[u64],
    detail: &str,
) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CLI package must live directly under the workspace root")
        .to_path_buf()
        .join("build/failures")
        .join(format!("{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    fs::write(path.join("input.debug"), format!("{program:#?}\n"))
        .map_err(|error| error.to_string())?;
    fs::write(path.join("module.ll"), llvm).map_err(|error| error.to_string())?;
    fs::write(path.join("expected.txt"), format!("{expected:?}\n"))
        .map_err(|error| error.to_string())?;
    fs::write(path.join("observed.txt"), detail).map_err(|error| error.to_string())?;
    fs::write(
        path.join("toolchain.txt"),
        format!(
            "crate: {} {}\noptimized: {optimized}\ntarget: {}\nopt: {}\nclang: {}\n{}\n",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            toolchain.target_triple(),
            toolchain.opt.display(),
            toolchain.clang.display(),
            toolchain.identity
        ),
    )
    .map_err(|error| error.to_string())?;
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CLI package must live directly under the workspace root")
        .to_path_buf();
    // Preserve actual workspace-relative ownership so both manifests and their
    // embedded runtime/example inputs and Cargo configuration can be replayed.
    for component in ["compiler", "cli", "runtime", "tests", "examples", ".cargo"] {
        copy_tree(&manifest.join(component), &path.join(component))?;
    }
    // Version labels cannot identify dirty compiler code; retain build inputs
    // alongside complete source snapshots for a self-contained failing case.
    for name in ["Cargo.toml", "Cargo.lock"] {
        fs::copy(manifest.join(name), path.join(name)).map_err(|error| error.to_string())?;
    }
    Ok(path)
}

/// Copy maintained source bytes into a generated replay directory on failure.
///
/// Input directories are maintained workspace components, never build output.
/// Symlinks and special files are rejected rather than traversed implicitly.
fn copy_tree(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    fs::create_dir(destination).map_err(|error| error.to_string())?;
    // Snapshot complete source only for a failing case; normal validation and
    // native execution carry no filesystem-copy or provenance overhead.
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        // Preserve module boundaries and READMEs, copying only regular files.
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
        } else {
            return Err(format!(
                "cannot snapshot non-regular source entry {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}
