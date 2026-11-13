//! Parsed-source and programmatic placement checks through the real LLVM backend.
//!
//! Expected values are independently derived and checked against the safe model
//! before native O0/O3/ASan runs. Source and lowered IR share the same checked path;
//! no host-language implementation is substituted for a Holo workload. Failed
//! native cases retain source, LLVM, observations, commands, and tool identities.

#[path = "../../support/model/mod.rs"]
mod model;

use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::parse_experimental;
use holo::ir::{Expr, Op, Program};
use holo::layout::LayoutMode;
use holo::placement::{ScopedPlacement, place_scoped_aggregate};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Source workload with nested destination construction and three distinct failed appends.
/// Retained aliases witness stable addresses; byte scans observe published contents.
const NESTED: &str = r#"
// Inline payload and outer record; neither has independent object reclamation.
struct Payload { data Bytes }
struct Entry { id u64, payload Payload }
// Explicit inputs construct each byte from a seed and its contextual index.
func payload
    with noblock
    arg length u64 as readonly copy
    arg seed u64 as readonly copy
    returns Payload
{
    return Payload { data: core.bytes(length, core.add_wrapping(seed, core.index())) }
}
// This nested producer rejects selected input before publication of its outer record.
func rejected_payload
    with noblock
    arg length u64 as readonly copy
    arg seed u64 as readonly copy
    returns Payload
{
    core.reject_if(length == 7)
    return Payload { data: core.bytes(length, core.add_wrapping(seed, core.index())) }
}
// Ordinary constructor calls target the receiving nested field directly.
func entry
    with noblock
    arg id u64 as readonly copy
    arg length u64 as readonly copy
    arg seed u64 as readonly copy
    returns Entry
{
    return Entry { id: id, payload: payload(length, seed) }
}
// The outer id has begun construction when its nested producer rejects.
func rejected_entry
    with noblock
    arg id u64 as readonly copy
    arg length u64 as readonly copy
    arg seed u64 as readonly copy
    returns Entry
{
    return Entry { id: id, payload: rejected_payload(length, seed) }
}
// Observe successful growth, rollback, alias visibility, scanning, reset, and reuse.
func main
    returns u64
{
    let memory = core.region(512) else { core.print(999); return 0 }
    {
        let items = core.batch(memory, Entry)
        let initial = core.append(items, entry(7, 4, 10))
        core.assert(initial == 0)
        {
            let first = core.borrow_readwrite(items, 0)
            let reader = core.alias_readonly(first)
            let original = core.address(first)
            // Existing addresses and aliases remain valid through each successful extension.
            for index in core.range(3) {
                let status = core.append(items, entry(core.add_wrapping(index, 2), core.add_wrapping(index, 1), 20))
                core.assert(status == 0)
            }
            first.id = 9
            first.payload.data[1] = 77
            let published = core.used(memory)
            let count = core.len(items)

            let full = core.append(items, entry(99, 512, 0))
            core.print(full)
            core.assert(full == 1)
            core.assert(core.used(memory) == published)
            core.assert(core.len(items) == count)

            let rejected = core.append(items, rejected_entry(99, 7, 0))
            core.print(rejected)
            core.assert(rejected == 3)
            core.assert(core.used(memory) == published)
            core.assert(core.len(items) == count)

            let overflow = core.append(items, entry(99, 18446744073709551615, 0))
            core.print(overflow)
            core.assert(overflow == 2)
            core.assert(core.used(memory) == published)
            core.assert(core.len(items) == count)
            core.assert(core.address(reader) == original)

            var checksum = 0
            // The sequential scan observes only fully published records and frozen byte lengths.
            for readonly item in items {
                checksum = core.add_wrapping(checksum, item.id)
                for byte in core.range(core.len(item.payload.data)) {
                    checksum = core.add_wrapping(checksum, item.payload.data[byte])
                }
            }
            core.print(checksum)
            core.print(core.len(items))
            core.print(core.used(memory))
            core.print(reader.id)
        }
        core.reset(items)
        core.print(core.len(items))
        core.print(core.used(memory))
        let reused = core.append(items, entry(42, 0, 0))
        core.assert(reused == 0)
        let again = core.borrow_readonly(items, 0)
        core.print(again.id)
    }
    core.print(core.used(memory))
    return 0
}
"#;

/// Successful early completion from nested owners, batch iteration, and counted control flow.
/// Unreachable prints detect accidental fallthrough without allocating additional data.
const EARLY_EXIT: &str = r#"
// Even empty records carry the current fixture's explicit size header.
struct Empty {}
// Construct an empty record directly in its receiving slot.
func empty with noblock returns Empty { return Empty {} }
// Both acquired owners must be released before entry completion reaches host exit.
func main returns u64 {
    let outer = core.region(64) else { core.print(998); return 0 }
    let items = core.batch(outer, Empty)
    let initial = core.append(items, empty())
    core.assert(initial == 0)
    // Entry completion leaves every enclosing storage/control scope immediately.
    for readonly item in items {
        let inner = core.region(64) else { core.print(997); return 0 }
        let inner_items = core.batch(inner, Empty)
        let nested = core.append(inner_items, empty())
        core.assert(nested == 0)
        for index in core.range(18446744073709551615) {
            // The first iteration ends successfully; the huge remaining range is never visited.
            if index == 0 { core.print(11); return 0 }
            core.print(996)
        }
        core.print(995)
    }
    core.print(994)
    return 0
}
"#;

#[test]
/// The maintained readable example executes unchanged through O0, O3, and ASan.
fn source_example_matches_model_and_native() {
    compare(
        "source-example",
        include_str!("../../fixtures/region.holo"),
        &[98, 0, 0],
        false,
    );
}

#[test]
/// Nested failures preserve published bytes/counts and aliases across all native modes.
fn nested_source_construction_rollback_and_reuse_match_model() {
    // Four records occupy (32+4)+(32+1)+(32+2)+(32+3)=138 bytes.
    // Their independent scalar/byte sums are 121+22+44+67=254 after mutation.
    compare(
        "source-nested",
        NESTED,
        &[1, 3, 2, 254, 4, 138, 9, 0, 0, 42, 0],
        false,
    );
}

#[test]
/// Early source returns free acquired backing before exit and stop remaining work.
fn source_early_completion_cleans_enclosing_regions() {
    compare("source-early-exit", EARLY_EXIT, &[11], true);
}

/// Propagation returns the exact failure, skipping work that would trap on a
/// zero-length borrow. The success continuation observes the admitted length.
#[test]
fn source_try_propagates_before_a_success_only_read() {
    let source = r#"
// Construct the two available bytes in owned input outside the typed helpers.
struct Input { data Bytes }
func bytes returns Input { return Input { data: core.bytes(2, 7) } }
error FrameError { Empty; Excess }
// Each failure remains distinguishable after it propagates out of a helper.
func verify arg count u64 as readonly copy returns Result<u64, FrameError> {
    if count == 0 { return error FrameError.Empty }
    if count > 1 { return error FrameError.Excess }
    return ok count
}
// Success proves a nonempty extent for the following checked byte access.
func read arg data Bytes as readonly reference returns Result<u64, FrameError> {
    let count = try verify(core.len(data))
    return ok core.add_wrapping(count, data[0])
}
// Evaluate the same body on failure and success against the independent oracle.
func main returns u64 {
    let memory = core.region(64) else { return 0 }
    let values = core.batch(memory, Input)
    let appended = core.append(values, bytes())
    core.assert(appended == 0)
    let input = core.borrow_readonly(values, 0)
    let empty = read(core.prefix(input.data, 0))
        on ok(value) then value
        on Empty then 17
        on Excess then 22
    let excess = read(input.data)
        on ok(value) then value
        on Empty then 17
        on Excess then 22
    let success = read(core.prefix(input.data, 1))
        on ok(value) then value
        on error then 0
    core.print(empty)
    core.print(excess)
    core.print(success)
    return 0
}
"#;
    compare("source-try", source, &[17, 22, 8], false);
}

/// Lift failures through two nonzero tag offsets. Colliding leaf names keep
/// separate arms, and assertions witness that later try calls are skipped.
#[test]
fn source_nested_error_union_preserves_payload_and_short_circuits() {
    let source = r#"
error LeafError { Failed(code u64) }
error MiddleError { Failed; from LeafError as Leaf }
error OuterError { Failed; from MiddleError as Middle }

// Preserve all payload bits across both lifts.
func first arg scenario u64 as readonly copy returns Result<u64, LeafError> {
    if scenario == 1 { return error LeafError.Failed(18446744073709551615) }
    return ok scenario
}

// A failure of first must skip this ordinary call entirely.
func second arg scenario u64 as readonly copy returns Result<u64, MiddleError> {
    core.assert(scenario != 1)
    if scenario == 2 { return error MiddleError.Failed }
    return ok scenario
}

func middle arg scenario u64 as readonly copy returns Result<u64, MiddleError> {
    let value = try first(scenario)
    let result = try second(value)
    return ok result
}

// Use an explicit qualified construction on a separate edge too.
func outer arg scenario u64 as readonly copy returns Result<u64, OuterError> {
    if scenario == 4 { return error OuterError.Middle.Leaf.Failed(99) }
    let value = try middle(scenario)
    core.assert(value != 1)
    core.assert(value != 2)
    if value == 3 { return error OuterError.Failed }
    return ok 42
}

func show arg scenario u64 as readonly copy returns u64 {
    return outer(scenario)
        on ok(value) then value
        on Middle.Leaf.Failed(code) then code
        on Middle.Failed then 2
        on Failed then 3
}

func main returns u64 {
    core.print(show(0))
    core.print(show(1))
    core.print(show(2))
    core.print(show(3))
    core.print(show(4))
    return 0
}
"#;
    compare(
        "source-nested-error-unions",
        source,
        &[42, u64::MAX, 2, 3, 99],
        false,
    );
}

/// Check/lower real source, compare independent expected values, and run three native modes.
/// `track_cleanup` adds test-only backing accounting after compilation, never a data kernel.
fn compare(label: &str, source: &str, expected: &[u64], track_cleanup: bool) {
    compare_layout(label, source, expected, track_cleanup, LayoutMode::Wide);
}

/// Replays typed publication across the physical encodings, including a partial reserve.
/// Derived64 can reserve the parent word and reach the child with no remaining capacity.
#[test]
fn source_typed_append_preserves_prefix_and_failure_order() {
    let source = include_str!("../../fixtures/typed_append.holo");
    // The original fixture tests an empty tail before entering the nested producer.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        compare_layout(
            "source-typed-append",
            source,
            &[u64::MAX, 8, 1],
            false,
            layout,
        );
    }
    let partial = source
        .replace(
            "core.region(saved)",
            "core.region(core.add_wrapping(saved, 8))",
        )
        .replace("on Full  then 1", "on Rejected BadChild(value) then value")
        .replace(
            "core.assert(failure == 1)",
            "core.assert(failure == 18446744073709551615)",
        )
        .replace(
            "core.print(reason)",
            r#"
    // With valid input, the same remaining capacity fails on the child reservation.
    let valid = core.append(bounded, make_parent(8, 4))
        on ok    then 0
        on Full  then 1
        on error then 2
    core.assert(valid == 1)
    core.assert(core.len(bounded) == 1)
    core.assert(core.used(limited) == saved)
    core.print(reason)"#,
        );
    compare_layout(
        "source-typed-append-partial",
        &partial,
        &[u64::MAX, 8, u64::MAX],
        false,
        LayoutMode::Derived64,
    );
    // The last child now returns a different family. Its specialized fixed
    // definition retags the domain failure and preserves Full.
    let union = source
        .replace(
            "struct Child {",
            "error ParentError { BadSequence; from BuildError as ChildError }\nstruct Child {",
        )
        .replace("Result<Parent, BuildError>", "Result<Parent, ParentError>")
        .replace(
            "on Rejected BadChild(value) then value",
            "on Rejected ChildError.BadChild(value) then value\n        on Rejected BadSequence then 3",
        );
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        compare_layout(
            "source-fixed-child-union",
            &union,
            &[u64::MAX, 8, 1],
            false,
            layout,
        );
    }
}

/// Verify a shared producer used under several different nominal shifts.
/// The last two uses exceed the specialization budget and still lift on failure.
#[test]
fn source_terminal_lifts_preserve_all_checked_callers() {
    let mut source = String::from(
        r#"
error LeafError { Rejected(value u64) }
struct Leaf { value u64 }
struct Box { leaf Leaf }
struct Envelope { item Box }

func leaf arg value u64 as readonly copy returns Result<Leaf, LeafError> {
    if value > 100 { return error LeafError.Rejected(value) }
    return ok Leaf:
        value = value
}
"#,
    );
    // Each wrapper includes the leaf at another offset, while all share
    // the exact same child. The final wrappers exercise caller-side fallback.
    for index in 0..6 {
        let local = (0..=index)
            .map(|case| format!("Local{case}; "))
            .collect::<String>();
        source.push_str(&format!(
            "
error BoxError{index} {{ {local}from LeafError as Child }}
func box{index} arg value u64 as readonly copy returns Result<Box, BoxError{index}> {{
    return ok Box:
        leaf = try leaf(value)
}}
"
        ));
    }
    source.push_str(
        "
error EnvelopeError { First; Second; from BoxError0 as Boxed }
func envelope arg value u64 as readonly copy returns Result<Envelope, EnvelopeError> {
    return ok Envelope:
        item = try box0(value)
}

func main returns u64 {
    let region = core.region(512) else { core.assert(false); return 0 }
    {
        let outer = core.batch(region, Envelope)
        let rejected = core.append(outer, envelope(18446744073709551615))
            on ok then 0
            on Rejected Boxed.Child.Rejected(value) then value
            on error then 0
        core.print(rejected)
        core.assert(core.len(outer) == 0)
    }
    let boxes = core.batch(region, Box)
",
    );
    // Verify the full-width payload reaches every caller and leaves the same
    // receiving marker untouched. Each handler independently witnesses its tag.
    for index in 0..6 {
        source.push_str(&format!(
            "
    let failed{index} = core.append(boxes, box{index}(18446744073709551615))
        on ok then 0
        on Rejected Child.Rejected(value) then value
        on error then 0
    core.print(failed{index})
    core.assert(core.len(boxes) == 0)
"
        ));
    }
    source.push_str("    return 0\n}\n");
    compare_layout(
        "fixed-terminal-union-encodings",
        &source,
        &[u64::MAX; 7],
        false,
        LayoutMode::Derived64,
    );
}

/// Check static field offsets and resumed reservations across multiple children.
/// The independent model and all native modes observe writes only after publication.
#[test]
fn source_typed_fixed_children_preserve_offsets_and_resume_reservations() {
    let source = r#"
// Reject with the original full-width scalar after a previous child succeeded.
error BuildError { BadChild(value u64) }
struct Child { value u64 }
struct Empty {}
struct Parent {
    sequence u64
    first Child
    empty Empty
    middle u64
    second Child
    last u64
}

// Validate before reserving the receiving field.
func child with noblock arg value u64 as readonly copy returns Result<Child, BuildError> {
    if value >= 100 {
        return error BuildError.BadChild(value)
    }
    return ok Child:
        value = value
}

// Headerless layouts reserve and initialize one byte for an empty record.
func empty with noblock returns Result<Empty, BuildError> {
    return ok Empty:
}

// The middle and last reservations begin after a successful nested call.
func parent
    with noblock
    arg sequence u64 as readonly copy
    arg value u64 as readonly copy
    returns Result<Parent, BuildError>
{
    return ok Parent:
        sequence = sequence
        first    = try child(11)
        empty    = try empty()
        middle   = core.add_wrapping(sequence, 100)
        second   = try child(value)
        last     = 233
}

// Verify retained data, a failed partial append, and a later retry.
func main returns u64 {
    let memory = core.region(512) else { core.assert(false); return 0 }
    let rows = core.batch(memory, Parent)
    let seeded = core.append(rows, parent(7, 12))
        on ok    then true
        on error then false
    core.assert(seeded)
    let first = core.borrow_readonly(rows, 0)
    let saved = core.used(memory)
    let address = core.address(first)

    // The final child rejects after four fields of its parent were initialized.
    let reason = core.append(rows, parent(8, 18446744073709551615))
        on ok                       then 0
        on Rejected BadChild(value) then value
        on error                    then 1
    core.assert(reason == 18446744073709551615)
    core.assert(core.len(rows) == 1)
    core.assert(core.used(memory) == saved)
    core.assert(core.address(first) == address)
    core.assert(first.sequence == 7)
    core.assert(first.first.value == 11)
    core.assert(first.middle == 107)
    core.assert(first.second.value == 12)
    core.assert(first.last == 233)

    // Retry in the same suffix. Each nonzero field exposes its actual offset.
    let retried = core.append(rows, parent(8, 13))
        on ok    then true
        on error then false
    core.assert(retried)
    core.assert(core.len(rows) == 2)
    core.assert(core.used(memory) == core.mul_wrapping(saved, 2))
    core.assert(core.address(first) == address)
    let second = core.borrow_readonly(rows, 1)
    core.assert(second.sequence == 8)
    core.assert(second.first.value == 11)
    core.assert(second.middle == 108)
    core.assert(second.second.value == 13)
    core.assert(second.last == 233)

    // Every child fits, but the scalar after the final child does not.
    let limit = core.sub_wrapping(saved, 8)
    let limited = core.region(limit) else { core.assert(false); return 0 }
    let bounded = core.batch(limited, Parent)
    let outcome = core.append(bounded, parent(8, 13))
        on ok    then 0
        on Full  then 1
        on error then 2
    core.assert(outcome == 1)
    core.assert(core.used(limited) == 0)
    core.assert(core.len(bounded) == 0)
    core.print(saved)
    core.print(reason)
    core.print(second.middle)
    core.print(outcome)
    return 0
}
"#;
    // Wide carries four headers. Derived and compact use five words and an empty unit.
    for (layout, stride) in [
        (LayoutMode::Wide, 72),
        (LayoutMode::Derived64, 41),
        (LayoutMode::Compact32, 41),
    ] {
        compare_layout(
            "source-typed-fixed-children",
            source,
            &[stride, u64::MAX, 108, 1],
            false,
            layout,
        );
    }
}

/// A variable parent resumes reservation after a fixed child at a dynamic offset.
/// Byte checks distinguish the committed child extent from an overwritten suffix.
#[test]
fn source_typed_variable_parent_commits_fixed_child_before_trailing_bytes() {
    let source = r#"
error BuildError { BadCode(value u64) }
struct Code { value u64 }
struct Message { leading Bytes, code Code, trailing Bytes }

// The child's rejection must escape with its full scalar before any child write.
func checked_code with noblock arg value u64 as readonly copy returns Result<Code, BuildError> {
    if value >= 100 { return error BuildError.BadCode(value) }
    return ok Code:
        value = value
}

// The leading length moves the child's destination. The trailing field starts
// at the first byte after the child only if its completed extent is recorded.
func message
    with noblock
    arg before u64 as readonly copy
    arg value u64 as readonly copy
    arg after u64 as readonly copy
    returns Result<Message, BuildError>
{
    return ok Message:
        leading  = core.bytes(before, core.add_wrapping(10, core.index()))
        code     = try checked_code(value)
        trailing = core.bytes(after, core.add_wrapping(40, core.index()))
}

func main returns u64 {
    // Measure one record to give the second owner exact capacity in every layout.
    let sizing = core.region(128) else { core.assert(false); return 0 }
    let sample = core.batch(sizing, Message)
    let sized = core.append(sample, message(3, 11, 2))
        on ok    then true
        on error then false
    core.assert(sized)
    let stride = core.used(sizing)
    let limit = core.mul_wrapping(stride, 2)
    let memory = core.region(limit) else { core.assert(false); return 0 }
    let rows = core.batch(memory, Message)
    let seeded = core.append(rows, message(3, 11, 2))
        on ok    then true
        on error then false
    core.assert(seeded)
    let first = core.borrow_readonly(rows, 0)
    let address = core.address(first)

    // The failed child must leave the initial message and its published end intact.
    let reason = core.append(rows, message(2, 18446744073709551615, 3))
        on ok                      then 0
        on Rejected BadCode(value) then value
        on error                   then 1
    core.assert(reason == 18446744073709551615)
    core.assert(core.used(memory) == stride)
    core.assert(core.len(rows) == 1)

    // The child succeeds but the trailing field needs one byte beyond capacity.
    let full = core.append(rows, message(2, 22, 4))
        on ok    then false
        on Full  then true
        on error then false
    core.assert(full)
    core.assert(core.used(memory) == stride)
    core.assert(core.len(rows) == 1)

    // A shorter trailing field fits exactly, with the child at a new offset.
    let retried = core.append(rows, message(2, 22, 3))
        on ok    then true
        on error then false
    core.assert(retried)
    core.assert(core.used(memory) == limit)
    core.assert(core.len(rows) == 2)
    core.assert(core.address(first) == address)
    core.assert(core.len(first.leading) == 3)
    core.assert(first.leading[2] == 12)
    core.assert(first.code.value == 11)
    core.assert(core.len(first.trailing) == 2)
    core.assert(first.trailing[1] == 41)
    let second = core.borrow_readonly(rows, 1)
    core.assert(core.len(second.leading) == 2)
    core.assert(second.leading[1] == 11)
    core.assert(second.code.value == 22)
    core.assert(core.len(second.trailing) == 3)
    core.assert(second.trailing[0] == 40)
    core.print(stride)
    core.print(reason)
    core.print(second.trailing[2])
    return 0
}
"#;
    // Give the parent a direct leaf before the child block. A child error now
    // retags after earlier byte writes, while Full from the trailing field
    // still restores the original prefix. Run both family configurations.
    let union = source
        .replace(
            "struct Code {",
            "error MessageError { BadLength; from BuildError as CodeError }\nstruct Code {",
        )
        .replace(
            "Result<Message, BuildError>",
            "Result<Message, MessageError>",
        )
        .replace(
            "on Rejected BadCode(value)",
            "on Rejected CodeError.BadCode(value)",
        );
    // Give the child its own runtime extent. Success must advance the parent
    // before the final field. A later overflow preserves the published end.
    let variable_child = union
        .replace("struct Code { value u64 }", "struct Code { value u64; note Bytes }")
        .replace(
            "        value = value\n}",
            "        value = value\n        note = core.bytes(3, core.add_wrapping(50, core.index()))\n}",
        )
        .replace(
            "    // A shorter trailing field fits exactly",
            r#"    // Width or arithmetic overflow in the final field leaves the prefix intact.
    let overflow = core.append(rows, message(2, 22, 18446744073709551615))
        on ok then false
        on SizeOverflow then true
        on error then false
    core.assert(overflow)
    core.assert(core.used(memory) == stride)
    core.assert(core.len(rows) == 1)

    // A shorter trailing field fits exactly"#,
        )
        .replace(
            "    core.assert(second.code.value == 22)",
            "    core.assert(second.code.value == 22)\n    core.assert(core.len(second.code.note) == 3)\n    core.assert(second.code.note[2] == 52)\n    core.assert(first.code.note[1] == 51)",
        );
    // Every parent has a retained header. Wide also stores the child's header,
    // while Compact32 saves four bytes at each of the two length fields.
    for (layout, stride) in [
        (LayoutMode::Wide, 45),
        (LayoutMode::Derived64, 37),
        (LayoutMode::Compact32, 29),
    ] {
        compare_layout(
            "source-typed-variable-parent-fixed-child",
            source,
            &[stride, u64::MAX, 42],
            false,
            layout,
        );
        compare_layout(
            "source-variable-parent-child-union",
            &union,
            &[stride, u64::MAX, 42],
            false,
            layout,
        );
        // The extra child field has a three-byte payload and one length word.
        let length_bytes = if layout == LayoutMode::Compact32 {
            4
        } else {
            8
        };
        compare_layout(
            "source-variable-child-then-variable-field",
            &variable_child,
            &[stride + length_bytes + 3, u64::MAX, 42],
            false,
            layout,
        );
    }
}

/// Force an eligible pair on success, named rejection, and a full tail.
/// Every native mode must retain the earlier record and its exact marker.
#[test]
fn bounded_pair_inlining_preserves_checked_publication() {
    let source = r#"
error DecodeError { Short(required u64) }
struct Wire { bytes Bytes }
struct Frame { data Bytes }
struct Received { channel u64, frame Frame }

func wire arg length u64 as readonly copy returns Wire {
    return Wire { bytes: core.bytes(length, core.add_wrapping(170, core.index())) }
}
func decode arg input Bytes as readonly reference returns Result<Frame, DecodeError> {
    // The guard runs before the checked copy.
    if core.len(input) < 2 { return error DecodeError.Short(2) }
    return ok Frame:
        data = core.bytes(1, input[core.add_wrapping(1, core.index())])
}
func receive arg input Bytes as readonly reference returns Result<Received, DecodeError> {
    return ok Received:
        channel = 9
        frame = try decode(input)
}
func main returns u64 {
    let source = core.region(128) else { core.assert(false); return 0 }
    let wires = core.batch(source, Wire)
    let valid = core.append(wires, wire(2))
    let short = core.append(wires, wire(1))
    core.assert(valid == 0)
    core.assert(short == 0)
    let complete = core.borrow_readonly(wires, 0)
    let partial = core.borrow_readonly(wires, 1)

    let sizing = core.region(128) else { core.assert(false); return 0 }
    let sample = core.batch(sizing, Received)
    let sized = core.append(sample, receive(complete.bytes))
        on ok then true
        on error then false
    core.assert(sized)
    let stride = core.used(sizing)
    let destination = core.region(stride) else { core.assert(false); return 0 }
    let frames = core.batch(destination, Received)

    // The parent writes privately, then the child rejects. Retry uses
    // the same suffix and fills the region exactly.
    let needed = core.append(frames, receive(partial.bytes))
        on ok then 0
        on Rejected Short(required) then required
        on error then 1
    core.assert(needed == 2)
    core.assert(core.used(destination) == 0)
    let added = core.append(frames, receive(complete.bytes))
        on ok then true
        on error then false
    core.assert(added)
    let retained = core.borrow_readonly(frames, 0)
    let address = core.address(retained)
    let full = core.append(frames, receive(partial.bytes))
        on ok then false
        on Full then true
        on error then false
    core.assert(full)
    core.assert(core.address(retained) == address)
    core.assert(core.len(frames) == 1)
    core.assert(core.used(destination) == stride)
    core.assert(retained.channel == 9)
    core.print(retained.frame.data[0])
    return 0
}
"#;
    // The measured layout selects both definitions. Compact32 supplies
    // a behavioral control with the existing cost model.
    for (mode, attributes) in [(LayoutMode::Derived64, 2), (LayoutMode::Compact32, 0)] {
        let input = parse_experimental(source)
            .unwrap_or_else(|error| panic!("{}", error.render("inline-publication.holo", source)));
        let checked = input.check().unwrap();
        let llvm =
            holo::llvm::emit_with_layout(&checked, "arm64-apple-macosx11.0.0", mode).unwrap();
        assert_eq!(llvm.matches(" alwaysinline {").count(), attributes);
        compare_layout("bounded-inline-publication", source, &[171], false, mode);
    }
}

/// Exact whole-result forwards share one destination with the leaf producer.
/// Retained headers, empty units, and typed failures all use that single extent.
#[test]
fn aggregate_tail_forward_preserves_extent_and_rollback() {
    let source = r#"
error DecodeError { Short(required u64) }
error StoreError { Unavailable; from DecodeError as Input }
error TicketError { Denied(value u64) }
error IssueError { Closed; from TicketError as Ticket }
struct Empty {}
struct Wire { data Bytes }
struct Frame { channel u64, data Bytes }
struct Entry { sequence u64, frame Frame, stamp u64 }
struct Ticket { number u64 }
struct Issued { sequence u64, ticket Ticket }

// The wrapper must leave the empty record's only unit or header to its callee.
func empty returns Empty { return Empty {} }
func blank returns Empty { return empty() }

// Keep a distinct input owner and forward its readonly bytes through two calls.
func wire arg length u64 as readonly copy returns Wire {
    return Wire { data: core.bytes(length, core.add_wrapping(170, core.index())) }
}
func decode arg input Bytes as readonly reference arg count u64 as readonly copy
    returns Result<Frame, DecodeError> {
    if core.len(input) < count { return error DecodeError.Short(count) }
    return ok Frame:
        channel = core.len(input)
        data = core.bytes(count, input[core.index()])
}
func decoded arg input Bytes as readonly reference arg count u64 as readonly copy
    returns Result<Frame, DecodeError> {
    return decode(input, count)
}

// The nested variable field precedes a scalar, so every layout retains a
// size header. The parent's own scalar has already been written on rejection.
func entry arg input Bytes as readonly reference arg count u64 as readonly copy
    arg sequence u64 as readonly copy returns Result<Entry, StoreError> {
    return ok Entry:
        sequence = sequence
        frame = try decoded(input, count)
        stamp = core.add_wrapping(sequence, 100)
}
func route arg input Bytes as readonly reference arg count u64 as readonly copy
    arg sequence u64 as readonly copy returns Result<Entry, StoreError> {
    return entry(input, count, sequence)
}
func reroute arg input Bytes as readonly reference arg count u64 as readonly copy
    arg sequence u64 as readonly copy returns Result<Entry, StoreError> {
    return route(input, count, sequence)
}

// A fixed terminal child can inherit a lifted error range from its parent.
// The whole-result wrapper relays the exact family before the parent's lift.
func ticket arg number u64 as readonly copy returns Result<Ticket, TicketError> {
    if number >= 100 { return error TicketError.Denied(number) }
    return ok Ticket:
        number = number
}
func ticket_forward arg number u64 as readonly copy returns Result<Ticket, TicketError> {
    return ticket(number)
}
func issued arg number u64 as readonly copy returns Result<Issued, IssueError> {
    return ok Issued:
        sequence = 11
        ticket = try ticket_forward(number)
}

func main returns u64 {
    let input = core.region(128) else { core.assert(false); return 0 }
    let wires = core.batch(input, Wire)
    let complete_status = core.append(wires, wire(3))
    let partial_status = core.append(wires, wire(1))
    core.assert(complete_status == 0)
    core.assert(partial_status == 0)
    let complete = core.borrow_readwrite(wires, 0)
    let partial = core.borrow_readonly(wires, 1)

    // Size a nonforwarded outer call, then require two exact forwarded slots.
    let sizing = core.region(128) else { core.assert(false); return 0 }
    let sample = core.batch(sizing, Entry)
    let sized = core.append(sample, entry(complete.data, 2, 7))
        on ok then true
        on error then false
    core.assert(sized)
    let stride = core.used(sizing)
    let capacity = core.mul_wrapping(stride, 2)
    let memory = core.region(capacity) else { core.assert(false); return 0 }
    let entries = core.batch(memory, Entry)
    let seeded = core.append(entries, reroute(complete.data, 2, 7))
        on ok then true
        on error then false
    core.assert(seeded)
    let retained = core.borrow_readonly(entries, 0)
    let address = core.address(retained)
    core.assert(core.used(memory) == stride)

    // Roll back the private prefix before handling the lifted, full-width error.
    let reason = core.append(entries, reroute(partial.data, 18446744073709551615, 8))
        on ok then 0
        on Rejected Input.Short(required) then required
        on error then 1
    core.assert(reason == 18446744073709551615)
    core.assert(core.len(entries) == 1)
    core.assert(core.used(memory) == stride)

    // The larger child fits, but the trailing word crosses the exact capacity.
    let exhausted = core.append(entries, reroute(complete.data, 3, 8))
        on ok then false
        on Full then true
        on error then false
    core.assert(exhausted)
    core.assert(core.len(entries) == 1)
    core.assert(core.used(memory) == stride)

    // Retry with the intended extent and observe the returned child end.
    let retried = core.append(entries, reroute(complete.data, 2, 8))
        on ok then true
        on error then false
    core.assert(retried)
    core.assert(core.len(entries) == 2)
    core.assert(core.used(memory) == capacity)
    core.assert(core.address(retained) == address)
    let second = core.borrow_readonly(entries, 1)
    core.assert(core.sub_wrapping(core.address(second), address) == stride)
    complete.data[0] = 99
    core.assert(retained.sequence == 7)
    core.assert(retained.frame.channel == 3)
    core.assert(retained.frame.data[0] == 170)
    core.assert(retained.stamp == 107)
    core.assert(second.sequence == 8)
    core.assert(core.len(second.frame.data) == 2)
    core.assert(second.frame.data[1] == 171)
    core.assert(second.stamp == 108)
    core.print(stride)
    core.print(reason)
    core.print(second.stamp)

    // Empty forwarding accounts for one Wide header or one headerless unit.
    let unit_memory = core.region(8) else { core.assert(false); return 0 }
    let units = core.batch(unit_memory, Empty)
    let unit = core.append(units, blank())
    core.assert(unit == 0)
    core.assert(core.len(units) == 1)
    core.print(core.used(unit_memory))

    // Check the fixed terminal child's shifted tag and its unshifted storage failure.
    let ticket_memory = core.region(64) else { core.assert(false); return 0 }
    let tickets = core.batch(ticket_memory, Issued)
    let denied = core.append(tickets, issued(18446744073709551615))
        on ok then 0
        on Rejected Ticket.Denied(value) then value
        on error then 1
    core.assert(denied == 18446744073709551615)
    core.assert(core.len(tickets) == 0)
    core.assert(core.used(ticket_memory) == 0)
    let allowed = core.append(tickets, issued(42))
        on ok then true
        on error then false
    core.assert(allowed)
    let published = core.borrow_readonly(tickets, 0)
    core.assert(published.sequence == 11)
    core.print(published.ticket.number)
    let limit = core.sub_wrapping(core.used(ticket_memory), 8)
    let bounded_memory = core.region(limit) else { core.assert(false); return 0 }
    let bounded = core.batch(bounded_memory, Issued)
    let full = core.append(bounded, issued(42))
        on ok then false
        on Full then true
        on error then false
    core.assert(full)
    core.assert(core.len(bounded) == 0)
    core.assert(core.used(bounded_memory) == 0)
    return 0
}
"#;
    let input = parse_experimental(source).unwrap();
    // The empty wrapper, dynamic child, two outer wrappers, and fixed child
    // each exercise the checked whole-result operation.
    assert_eq!(
        input
            .program()
            .producers
            .iter()
            .filter(|producer| matches!(
                producer.body.as_slice(),
                [holo::ir::Build::Forward { .. }]
            ))
            .count(),
        5
    );
    // The outer header stays eight bytes. Wide also has a nested header,
    // while Compact32 saves four bytes in the nested payload length.
    for (layout, stride, unit) in [
        (LayoutMode::Wide, 50, 8),
        (LayoutMode::Derived64, 42, 1),
        (LayoutMode::Compact32, 38, 1),
    ] {
        compare_layout(
            "aggregate-tail-forward",
            source,
            &[stride, u64::MAX, 108, unit, 42],
            false,
            layout,
        );
    }
}

/// A private byte-bearing suffix can fail after a parent write, then be reused
/// by its handler. Input mutation after construction cannot affect owned bytes.
#[test]
fn scoped_byte_result_rolls_back_and_reuses_one_region_tail() {
    let source = r#"
struct Wire { data Bytes }
struct Payload { data Bytes }
struct Frame { id u64, payload Payload, stamp u64 }
error InputError { Truncated(needed u64) }
error FrameError { from InputError as Input }

func wire returns Wire {
    return Wire { data: core.bytes(3, core.add_wrapping(40, core.index())) }
}
func payload arg input Bytes as readonly reference arg n u64 as readonly copy
    returns Result<Payload, InputError> {
    if n == 7 { return error InputError.Truncated(n) }
    return ok Payload:
        data = core.bytes(n, input[core.index()])
}
func frame arg input Bytes as readonly reference arg n u64 as readonly copy
    returns Result<Frame, FrameError> {
    return ok Frame:
        id = 11
        payload = try payload(input, n)
        stamp = 13
}
func sum_pair arg data Bytes as readonly reference returns u64 {
    return core.add_wrapping(data[0], data[1])
}

func main returns u64 {
    let source = core.region(128) else { core.assert(false); return 0 }
    let wires = core.batch(source, Wire)
    let status = core.append(wires, wire())
    core.assert(status == 0)
    let input = core.borrow_readwrite(wires, 0)
    let sizing = core.region(128) else { core.assert(false); return 0 }
    var extent = 0
    core.construct(sizing, frame(input.data, 2))
        on ok(sample) then {
            extent = core.used(sizing)
            core.assert(core.len(sample.payload.data) == 2)
        }
        on error then { core.assert(false) }
    core.assert(core.used(sizing) == 0)
    core.print(extent)

    // Named rejection after writing id must release the tail before this retry.
    let exact = core.region(extent) else { core.assert(false); return 0 }
    var recovered = 0
    core.construct(exact, frame(input.data, 7))
        on ok then { core.assert(false) }
        on Rejected Input.Truncated(needed) then {
            core.assert(core.used(exact) == 0)
            core.print(needed)
            core.construct(exact, frame(input.data, 2))
                on ok(frame) then {
                    core.assert(core.used(exact) == extent)
                    let alias = core.alias_readonly(frame)
                    input.data[0] = 99
                    recovered = sum_pair(alias.payload.data)
                    core.assert(alias.id == 11)
                    core.assert(alias.stamp == 13)
                }
                on error then { core.assert(false) }
        }
        on error then { core.assert(false) }
    core.assert(core.used(exact) == 0)
    core.print(recovered)

    // A one-byte shortage and a mathematical overflow remain distinct.
    let small = core.region(core.sub_wrapping(extent, 1)) else { core.assert(false); return 0 }
    core.construct(small, frame(input.data, 2))
        on ok(_) then { core.assert(false) }
        on Full then {
            core.assert(core.used(small) == 0)
            core.construct(small, frame(input.data, 1))
                on ok(retry) then { core.print(retry.payload.data[0]) }
                on error then { core.assert(false) }
        }
        on error then { core.assert(false) }
    core.assert(core.used(small) == 0)
    core.construct(exact, frame(input.data, 18446744073709551615))
        on ok then { core.assert(false) }
        on SizeOverflow then { core.print(core.used(exact)) }
        on error then { core.assert(false) }
    return 0
}
"#;
    // Only the Wide parent retains the nested header as well as its own.
    for (layout, extent) in [
        (LayoutMode::Wide, 42),
        (LayoutMode::Derived64, 34),
        (LayoutMode::Compact32, 30),
    ] {
        compare_layout(
            "scoped-byte-result",
            source,
            &[extent, 7, 81, 99, 0],
            true,
            layout,
        );
    }
}

/// A fixed record borrows the producer's actual destination, releases exactly
/// one marker on the selected path, and leaves no batch or aggregate temporary.
#[test]
fn scoped_fixed_result_returns_from_either_arm_without_a_batch() {
    let source = r#"
struct Ticket { id u64 }
error IssueError { Denied(id u64) }
func issue arg id u64 as readonly copy returns Result<Ticket, IssueError> {
    if id > 20 { return error IssueError.Denied(id) }
    return ok Ticket:
        id = id
}
func main returns u64 {
    let region = core.region(16) else { core.assert(false); return 0 }
    core.construct(region, issue(9))
        on ok(ticket) then {
            core.print(ticket.id)
            return 0
        }
        on Rejected Denied(id) then {
            core.assert(core.used(region) == 0)
            core.print(id)
            return 0
        }
        on Full then { core.assert(false) }
        on SizeOverflow then { core.assert(false) }
    core.assert(false)
    return 0
}
"#;
    let failed = source.replace("issue(9)", "issue(99)");
    let program = parse_experimental(source).unwrap();
    let checked = program.check().unwrap();
    // Both marker and cursor conventions should pass the final address directly.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let llvm =
            holo::llvm::emit_with_layout(&checked, "arm64-apple-macosx11.0.0", layout).unwrap();
        let entry = llvm.split_once("define i32 @main()").unwrap().1;
        assert_eq!(entry.matches("call ptr @malloc(").count(), 1);
        assert_eq!(entry.matches(" @holo.producer.").count(), 1);
        assert!(!entry.contains("alloca %holo.batch"));
        assert!(!entry.contains("call ptr @holo.record_at."));
        assert!(!entry.contains("call void @llvm.mem"));
        compare_layout("scoped-fixed-success", source, &[9], true, layout);
        compare_layout("scoped-fixed-failure", &failed, &[99], true, layout);
    }
}

/// Both actuals mutate separate initialized state. Even a full destination
/// evaluates them once in written order before entering the producer.
#[test]
fn scoped_result_captures_effectful_actuals_once_and_in_order() {
    let source = r#"
struct Counter { count u64 }
struct Pair { value u64 }
error PairError { Invalid }
func zero returns Counter { return Counter { count: 0 } }
func next arg state Counter as readwrite reference returns u64 {
    state.count = core.add_wrapping(state.count, 1)
    return state.count
}
func pair arg first u64 as readonly copy arg second u64 as readonly copy
    returns Result<Pair, PairError> {
    return ok Pair:
        value = core.add_wrapping(core.mul_wrapping(first, 10), second)
}
func main returns u64 {
    let source = core.region(32) else { core.assert(false); return 0 }
    let counters = core.batch(source, Counter)
    let added = core.append(counters, zero())
    core.assert(added == 0)
    let state = core.borrow_readwrite(counters, 0)
    let empty = core.region(0) else { core.assert(false); return 0 }
    core.construct(empty, pair(next(state), next(state)))
        on ok then { core.assert(false) }
        on Full then { core.print(state.count) }
        on error then { core.assert(false) }
    let memory = core.region(16) else { core.assert(false); return 0 }
    core.construct(memory, pair(next(state), next(state)))
        on ok(pair) then { core.print(pair.value) }
        on error then { core.assert(false) }
    core.print(state.count)
    return 0
}
"#;
    compare_layout(
        "scoped-ordered-actuals",
        source,
        &[2, 34, 4],
        true,
        LayoutMode::Derived64,
    );
}

/// Producers shared by the programmatic and source placement controls.
/// Failures after a private field write distinguish rollback from rejection
/// before a destination is touched.
const SCOPED_PLACEMENT_DEFINITIONS: &str = r#"
// Inline payload and enclosing schema. Failure after the id write must release
// the private record before the next visit without affecting older roots.
struct Payload { data Bytes }
struct Frame { id u64, payload Payload }
struct Counter { count u64 }
struct Wire { data Bytes }
error DecodeError { Skipped(attempt u64) }
error FrameError { from DecodeError as Decode }

// Initialize separate backing for the mutable actuals and borrowed wire bytes.
func zero returns Counter { return Counter { count: 0 } }
func wire returns Wire { return Wire { data: core.bytes(2, 41) } }

// Side effects expose both the number and order of actual evaluations.
func next arg state Counter as readwrite reference returns u64 {
    state.count = core.add_wrapping(state.count, 1)
    return state.count
}

// Reject the named input before reserving child storage. Other lengths use the
// normal checked capacity and overflow paths before any payload reads.
func payload arg input Bytes as readonly reference arg length u64 as readonly copy
    returns Result<Payload, DecodeError> {
    if length == 7 { return error DecodeError.Skipped(length) }
    return ok Payload:
        data = core.bytes(length, input[core.index()])
}

// Store the ordered actuals in the receiving record before the child runs.
func frame
    arg first u64 as readonly copy
    arg second u64 as readonly copy
    arg length u64 as readonly copy
    arg input Bytes as readonly reference
    returns Result<Frame, FrameError>
{
    return ok Frame:
        id = core.add_wrapping(core.mul_wrapping(first, 10), second)
        payload = try payload(input, length)
}
"#;

/// Explicit control for the programmatic placement prototype, which replaces
/// a region and its one site after source lowering.
const SCOPED_PLACEMENT_CONTROL: &str = r#"
// The budgeted scope encloses the loop. On acquisition failure, the recovery
// runs before acquiring either of the separate roots used by the producer.
func main returns u64 {
    core.print(88)
    let memory = core.region(64) else {
        core.print(900)
        return 0
    }
    let counts = core.region(32) else { core.assert(false); return 0 }
    let counters = core.batch(counts, Counter)
    let count_status = core.append(counters, zero())
    core.assert(count_status == 0)
    let state = core.borrow_readwrite(counters, 0)
    let inputs = core.region(64) else { core.assert(false); return 0 }
    let wires = core.batch(inputs, Wire)
    let input_status = core.append(wires, wire())
    core.assert(input_status == 0)
    let input = core.borrow_readwrite(wires, 0)
    var first_address = 0
    // One static site sees each outcome. The final empty record must fit
    // when the first two-byte record is one byte over the budget.
    for attempt in core.range(5) {
        var length = 2
        if attempt == 1 { length = 7 }
        if attempt == 2 { length = 64 }
        if attempt == 3 { length = 18446744073709551615 }
        if attempt == 4 { length = 0 }
        core.construct(memory, frame(next(state), next(state), length, input.data))
            on ok(record) then {
                core.print(record.id)
                if attempt == 0 {
                    first_address = core.address(record)
                    input.data[0] = 99
                    core.assert(record.payload.data[0] == 41)
                }
                if attempt == 4 {
                    if first_address != 0 {
                        core.assert(core.address(record) == first_address)
                    }
                    core.assert(core.len(record.payload.data) == 0)
                    return 0
                }
            }
            on Rejected Decode.Skipped(needed) then {
                core.print(core.add_wrapping(100, needed))
            }
            on Full then {
                core.print(core.add_wrapping(200, state.count))
            }
            on SizeOverflow then {
                core.print(core.add_wrapping(300, state.count))
            }
    }
    core.print(state.count)
    return 0
}
"#;

/// Prove that automatic selection supplies the same checked storage as the
/// explicit name. A single verified native module suffices after exact
/// equality. Distinct inputs cover fit, shortage, zero, and setup failure.
#[test]
fn automatic_scoped_placement_matches_explicit_region() {
    let source = format!("{SCOPED_PLACEMENT_DEFINITIONS}{SCOPED_PLACEMENT_CONTROL}");
    let input = parse_experimental(&source).unwrap();
    let tools =
        NativeToolchain::discover().expect("placement/native tests require matched LLVM tools");
    // The layouts retain different nested headers and length widths, so an
    // otherwise identical two-byte record needs a different exact capacity.
    for (layout, exact_fit) in [
        (LayoutMode::Wide, 34),
        (LayoutMode::Derived64, 18),
        (LayoutMode::Compact32, 14),
    ] {
        // The order witness produces 12 and 100 on successful visits. The
        // full and overflow arms observe the side effects from failed visits.
        for (case, capacity, fail_acquire, expected) in [
            ("fit", exact_fit, false, &[88, 12, 107, 206, 308, 100][..]),
            (
                "short",
                exact_fit - 1,
                false,
                &[88, 202, 107, 206, 308, 100][..],
            ),
            ("zero", 0, false, &[88, 202, 204, 206, 208, 210, 10][..]),
            ("acquisition-fault", exact_fit, true, &[88, 900][..]),
        ] {
            let mut control = input.program().clone();
            // The first backing owns only the scoped site. Both ordinary
            // regions live in its success body, and leave no aliases to it.
            let Op::Region {
                capacity: budget,
                fail_acquire: injected,
                ..
            } = control.body.last_mut().expect("explicit control owner")
            else {
                panic!("explicit control must end in its outer region");
            };
            *budget = Expr::Const(capacity);
            *injected = fail_acquire;
            let candidate = select_scoped_site(control.clone());
            let control_checked = holo::check::check(&control).unwrap();
            let candidate_checked = holo::check::check(&candidate).unwrap();
            let label = format!("automatic-scoped-placement-{layout:?}-{case}");
            assert_eq!(
                model::run_with_layout(&control, layout).unwrap(),
                expected,
                "{label} explicit model"
            );
            assert_eq!(
                model::run_with_layout(&candidate, layout).unwrap(),
                expected,
                "{label} placement model"
            );
            let control_ir =
                holo::llvm::emit_with_layout(&control_checked, tools.target_triple(), layout)
                    .unwrap();
            let candidate_ir =
                holo::llvm::emit_with_layout(&candidate_checked, tools.target_triple(), layout)
                    .unwrap();
            assert!(
                candidate_ir == control_ir,
                "{label} changed the emitted LLVM"
            );
            let (_, entry) = candidate_ir
                .split_once("define i32 @main()")
                .expect("placement fixture has a direct entry");
            // The scoped owner, counter, and wire each have one setup site.
            // The repeated constructor adds neither a backing nor a batch.
            assert_eq!(entry.matches("call ptr @malloc(").count(), 3, "{label}");
            assert_eq!(entry.matches("alloca %holo.region").count(), 3, "{label}");
            assert_eq!(entry.matches("alloca %holo.batch").count(), 2, "{label}");
            // The matched ordinary modules are identical. Trace backing
            // cleanup in a separate copy when executing the common module.
            compare_native(&label, &source, expected, true, &tools, &candidate_ir);
        }
    }
}

/// Acquired body shared by the automatic source and its explicit control.
/// The wire owner surrounds repeated visits to one site, with both successful
/// visits checking the same address when the first frame fit.
const SOURCE_SCOPED_PLACEMENT_BODY: &str = r#"
        let inputs = core.region(64) else { core.assert(false); return 0 }
        let wires = core.batch(inputs, Wire)
        let input_status = core.append(wires, wire())
        core.assert(input_status == 0)
        let input = core.borrow_readwrite(wires, 0)
        var first_address = 0
        // Each visit reaches a different checked outcome or reuses the released tail.
        for attempt in core.range(5) {
            var length = 2
            if attempt == 1 { length = 7 }
            if attempt == 2 { length = 64 }
            if attempt == 3 { length = 18446744073709551615 }
            if attempt == 4 { length = 0 }
            core.construct(frame(next(state), next(state), length, input.data))
                on ok(record) then {
                    core.print(record.id)
                    if attempt == 0 {
                        first_address = core.address(record)
                        input.data[0] = 99
                        core.assert(record.payload.data[0] == 41)
                    }
                    if attempt == 4 {
                        if first_address != 0 {
                            core.assert(core.address(record) == first_address)
                        }
                        core.assert(core.len(record.payload.data) == 0)
                        // The early variant must clean up every owner before process exit.
                        if early == 1 { return 0 }
                    }
                }
                on Rejected Decode.Skipped(needed) then {
                    core.print(core.add_wrapping(100, needed))
                }
                on Full then {
                    core.print(core.add_wrapping(200, state.count))
                }
                on SizeOverflow then {
                    core.print(core.add_wrapping(300, state.count))
                }
        }
"#;

/// Create matched source forms with the same effectful budget and success body.
/// The explicit owner lives in braces so its lifetime matches the automatic
/// owner, including the post-owner continuation. This allocates fixture text only.
fn source_scoped_placement_fixture(capacity: u64, early_exit: bool) -> (String, String) {
    let prefix = format!(
        "{SCOPED_PLACEMENT_DEFINITIONS}{}\n    let early = {}\n",
        r#"
// Preserve the budget effect even when backing acquisition fails.
func budget
    arg state Counter as readwrite reference
    arg capacity u64 as readonly copy
    returns u64
{
    state.count = core.add_wrapping(state.count, 1)
    return capacity
}

func main returns u64 {
    core.print(88)
    let counts = core.region(32) else { core.assert(false); return 0 }
    let counters = core.batch(counts, Counter)
    let count_status = core.append(counters, zero())
    core.assert(count_status == 0)
    let state = core.borrow_readwrite(counters, 0)"#,
        u64::from(early_exit),
    );
    let recovery = "{ core.print(900); core.print(state.count) }";
    let continuation = "\n    core.print(701)\n    core.print(state.count)\n    return 0\n}\n";
    let automatic = format!(
        "{prefix}    with core.region(budget(state, {capacity})) {{{SOURCE_SCOPED_PLACEMENT_BODY}    }} else {recovery}{continuation}"
    );
    let explicit_body = SOURCE_SCOPED_PLACEMENT_BODY
        .replace("core.construct(frame(", "core.construct(scratch, frame(");
    let explicit = format!(
        "{prefix}    {{\n        let scratch = core.region(budget(state, {capacity})) else {recovery}{explicit_body}    }}{continuation}"
    );
    (automatic, explicit)
}

/// Inject failure at the owner whose capacity calls the fixture's budget
/// helper. Return a match count so changing the fixture cannot silently
/// turn this into an ordinary success test.
fn fail_source_scoped_acquisition(operations: &mut [Op]) -> usize {
    let mut matches = 0;
    // Descend only through the enclosing lexical owners and ordinary scope.
    for operation in operations {
        match operation {
            // The effectful budget uniquely identifies the candidate owner.
            Op::Region {
                capacity: Expr::Call { function, .. },
                fail_acquire,
                ..
            } if function == "budget" => {
                *fail_acquire = true;
                matches += 1;
            }
            Op::Region { body, .. } | Op::Batch { body, .. } | Op::Scope(body) => {
                matches += fail_source_scoped_acquisition(body);
            }
            _ => {}
        }
    }
    matches
}

/// Observe the remaining owner count at the post-block continuation and
/// at process exit. The instrumented native copy is built after LLVM equality.
fn observe_source_scoped_continuation(llvm: &str) -> String {
    let continuation = "call i32 (ptr, ...) @printf(ptr @holo.number_format, i64 701)";
    assert_eq!(llvm.matches(continuation).count(), 1);
    let offset = llvm.find(continuation).expect("one continuation call");
    // The call returns an SSA value. Insert before its entire instruction to
    // preserve the result assignment and native output.
    let instruction_start = llvm[..offset].rfind('\n').expect("complete LLVM line") + 1;
    let mut with_probe = llvm.to_owned();
    with_probe.insert_str(
        instruction_start,
        "  call void @source.test.scoped_continuation()\n",
    );
    let mut observed = count_backing(&with_probe);
    observed.push_str(
        r#"
; Only the older counter backing remains live after the lexical owner ends.
define internal void @source.test.scoped_continuation() {
entry:
  %live = load i64, ptr @source.test.live
  %released = icmp eq i64 %live, 1
  call void @holo.assert(i1 %released)
  ret void
}
"#,
    );
    observed
}

/// Source elaboration preserves exact LLVM, oracle outcomes, and cleanup of
/// both acquisition branches. Once equal, a common verified module covers
/// the identical native paths at O0, O3, and under AddressSanitizer.
#[test]
fn source_scoped_placement_matches_explicit_region() {
    let tools =
        NativeToolchain::discover().expect("source placement tests require matched LLVM tools");
    // Existing inline lengths and headers determine the exact two-byte frame fits.
    for (layout, exact_fit) in [
        (LayoutMode::Wide, 34),
        (LayoutMode::Derived64, 18),
        (LayoutMode::Compact32, 14),
    ] {
        // Actuals run in written order after the budget advances the counter.
        // The injected failure sees just the budget effect and reaches the continuation.
        for (case, capacity, early, expected) in [
            (
                "fit",
                exact_fit,
                false,
                &[88, 23, 107, 207, 309, 111, 701, 11][..],
            ),
            (
                "short",
                exact_fit - 1,
                false,
                &[88, 203, 107, 207, 309, 111, 701, 11][..],
            ),
            (
                "zero",
                0,
                false,
                &[88, 203, 205, 207, 209, 211, 701, 11][..],
            ),
            (
                "early-exit",
                exact_fit,
                true,
                &[88, 23, 107, 207, 309, 111][..],
            ),
            ("acquire-fault", exact_fit, false, &[88, 900, 1, 701, 1][..]),
        ] {
            let (automatic, explicit) = source_scoped_placement_fixture(capacity, early);
            let label = format!("source-scoped-placement-{layout:?}-{case}");
            let emit = |source: &str| {
                let input = parse_experimental(source)
                    .unwrap_or_else(|error| panic!("{}", error.render(&label, source)));
                input
                    .check()
                    .unwrap_or_else(|error| panic!("{}", error.render(&label, source)));
                let mut program = input.program().clone();
                // Fault injection changes only this test's acquisition edge after normal source checking.
                if case == "acquire-fault" {
                    assert_eq!(fail_source_scoped_acquisition(&mut program.body), 1);
                }
                let checked = holo::check::check(&program).unwrap();
                assert_eq!(model::run_with_layout(&program, layout).unwrap(), expected);
                holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap()
            };
            let candidate_ir = emit(&automatic);
            let control_ir = emit(&explicit);
            assert!(
                candidate_ir == control_ir,
                "{label} changed the emitted LLVM"
            );
            let observed = observe_source_scoped_continuation(&candidate_ir);
            compare_native(&label, &automatic, expected, false, &tools, &observed);
        }
    }
}

/// Turn the explicit fixture into a programmatic request. Move the owner
/// body and site operands out of the cloned program, preserving sibling order
/// and every argument/handler. Missing fixture nodes fail the test directly.
fn select_scoped_site(mut program: Program) -> Program {
    let Op::Region {
        name,
        capacity,
        fail_acquire,
        mut body,
        on_failure,
    } = program.body.pop().expect("explicit control owner")
    else {
        panic!("expected explicit control owner");
    };
    let mut site = Vec::new();
    let Op::ConstructHandled {
        region,
        producer,
        args,
        arms,
    } = take_scoped_site(&mut body, &mut site).expect("one scoped construction site")
    else {
        unreachable!("the path finder removes only scoped construction sites");
    };
    assert_eq!(region, name);
    let acquire_at = program.body.len();
    program.body.extend(body);
    place_scoped_aggregate(
        program,
        ScopedPlacement {
            acquire_at,
            capacity,
            fail_acquire,
            on_failure,
            site,
            producer,
            args,
            arms,
        },
    )
    .expect("place the fixture's single site in the private region")
}

/// Move the fixture's first scoped construction out and record its old path.
/// Only success bodies on the path can dominate the constructor visit. A
/// branch without a matching site restores the prefix before trying siblings.
fn take_scoped_site(operations: &mut Vec<Op>, site: &mut Vec<usize>) -> Option<Op> {
    // Walk the finite success-body tree in source order. Each frame records
    // its insertion index, which remains valid after the target is removed.
    for index in 0..operations.len() {
        site.push(index);
        match &mut operations[index] {
            Op::ConstructHandled { .. } => return Some(operations.remove(index)),
            Op::Region { body, .. }
            | Op::Batch { body, .. }
            | Op::Repeat { body, .. }
            | Op::Scope(body) => {
                // Keep the path when this child contains the single site.
                if let Some(operation) = take_scoped_site(body, site) {
                    return Some(operation);
                }
            }
            // Ordinary siblings and multiway branches are outside the path.
            _ => {}
        }
        site.pop();
    }
    None
}

/// Checks the chosen physical layout with the independent model and native modes.
/// The source and expected observations stay borrowed. Failed runs retain replay artifacts.
fn compare_layout(
    label: &str,
    source: &str,
    expected: &[u64],
    track_cleanup: bool,
    layout: LayoutMode,
) {
    let input = parse_experimental(source)
        .unwrap_or_else(|error| panic!("{}", error.render(label, source)));
    let checked = input
        .check()
        .unwrap_or_else(|error| panic!("{}", error.render(label, source)));
    // Preserve the standard Wide entry point and select the matching oracle for other layouts.
    let modeled = match layout {
        LayoutMode::Wide => model::run(input.program()),
        _ => model::run_with_layout(input.program(), layout),
    };
    assert_eq!(modeled.unwrap(), expected);
    let tools =
        NativeToolchain::discover().expect("source/native tests require matched LLVM tools");
    let emitted = holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
    compare_native(label, source, expected, track_cleanup, &tools, &emitted);
}

/// Verify and execute a checked module in all native modes against exact output.
/// Cleanup tracing may own an instrumented copy of the borrowed LLVM. Failure
/// retains replay inputs, while successful build artifacts are temporary.
fn compare_native(
    label: &str,
    source: &str,
    expected: &[u64],
    track_cleanup: bool,
    tools: &NativeToolchain,
    emitted: &str,
) {
    assert!(!emitted.contains("noalias"));
    assert!(!emitted.contains("@realloc"));
    // Cleanup accounting changes test observation only; ordinary fixture runs use untouched LLVM.
    let instrumented;
    let llvm = if track_cleanup {
        instrumented = count_backing(emitted);
        &instrumented
    } else {
        emitted
    };
    let expected_text = expected
        .iter()
        .map(|value| format!("{value}\n"))
        .collect::<String>();
    let scratch = TempDir::new().unwrap();
    // Each mode independently verifies/code-generates identical checked semantics.
    for mode in ["o0", "o3", "asan"] {
        let binary = scratch.path().join(mode);
        let build = match mode {
            "asan" => tools.build_sanitized(llvm, &binary),
            "o3" => tools.build(llvm, &binary, true),
            _ => tools.build(llvm, &binary, false),
        }
        .unwrap_or_else(|error| {
            failure(label, source, llvm, tools, mode, expected, &error, &binary)
        });
        let output = Command::new(&binary)
            .env("ASAN_OPTIONS", "detect_leaks=0")
            .output()
            .unwrap_or_else(|error| {
                failure(
                    label,
                    source,
                    llvm,
                    tools,
                    mode,
                    expected,
                    &error.to_string(),
                    &binary,
                )
            });
        // Exact text includes every decimal line and newline; a trap or extra output is a failure.
        if !output.status.success() || output.stdout != expected_text.as_bytes() {
            let detail = format!(
                "commands: {:#?}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
                build.commands,
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            failure(label, source, llvm, tools, mode, expected, &detail, &binary);
        }
    }
}

/// Retain deterministic replay inputs and available native output on failure, then panic.
/// Files stay under generated build output; failed evidence I/O cannot hide the original error.
#[allow(clippy::too_many_arguments)]
fn failure(
    label: &str,
    source: &str,
    llvm: &str,
    tools: &NativeToolchain,
    mode: &str,
    expected: &[u64],
    detail: &str,
    binary: &Path,
) -> ! {
    let save = || -> Result<std::path::PathBuf, String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CLI package must live directly under the workspace root")
            .join("build/failures")
            .join(format!("{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        fs::write(path.join("input.holo"), source).map_err(|error| error.to_string())?;
        fs::write(path.join("module.ll"), llvm).map_err(|error| error.to_string())?;
        fs::write(path.join("expected.txt"), format!("{expected:?}\n"))
            .map_err(|error| error.to_string())?;
        fs::write(path.join("observed.txt"), detail).map_err(|error| error.to_string())?;
        fs::write(
            path.join("toolchain.txt"),
            format!(
                "mode: {mode}\ntarget: {}\nopt: {}\nclang: {}\n{}",
                tools.target_triple(),
                tools.opt.display(),
                tools.clang.display(),
                tools.identity
            ),
        )
        .map_err(|error| error.to_string())?;
        // Build failures may have no binary; a completed executable is copied without overwriting source.
        if binary.is_file() {
            fs::copy(binary, path.join("native")).map_err(|error| error.to_string())?;
        }
        Ok(path)
    };
    // Preserve both discrepancies if retaining replay files also fails.
    match save() {
        Ok(path) => panic!(
            "{label}/{mode}: {detail}\nreplay evidence: {}",
            path.display()
        ),
        Err(error) => panic!("{label}/{mode}: {detail}\nevidence write failed: {error}"),
    }
}

/// Add test-only successful-allocation accounting and require zero owners at host exit.
/// This tests source early-exit cleanup, complementing the core's reverse-order cleanup tests.
fn count_backing(llvm: &str) -> String {
    let mut tracked = llvm
        .replace("call ptr @malloc(", "call ptr @source.test.malloc(")
        .replace("call void @free(", "call void @source.test.free(")
        .replace("call void @exit(", "call void @source.test.exit(");
    tracked.push_str(
        r#"
; Test-only ownership counter; no workload algorithm or allocation is hidden here.
@source.test.live = internal global i64 0
; Count only acquired backing; a null allocation contributes no cleanup obligation.
define internal ptr @source.test.malloc(i64 %size) {
entry:
  %base = call ptr @malloc(i64 %size)
  %present = icmp ne ptr %base, null
  %increment = zext i1 %present to i64
  %live = load i64, ptr @source.test.live
  %next = add i64 %live, %increment
  store i64 %next, ptr @source.test.live
  ret ptr %base
}
; Release one existing owner; underflow exposes repeated or unowned cleanup.
define internal void @source.test.free(ptr %base) {
entry:
  %live = load i64, ptr @source.test.live
  %present = icmp ugt i64 %live, 0
  call void @holo.assert(i1 %present)
  %next = sub i64 %live, 1
  store i64 %next, ptr @source.test.live
  call void @free(ptr %base)
  ret void
}
; Exit is allowed only after every successfully acquired source owner was released.
define internal void @source.test.exit(i32 %status) noreturn {
entry:
  %live = load i64, ptr @source.test.live
  %empty = icmp eq i64 %live, 0
  call void @holo.assert(i1 %empty)
  call void @exit(i32 %status)
  unreachable
}
"#,
    );
    tracked
}
