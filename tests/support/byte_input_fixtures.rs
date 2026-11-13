//! Actual source and hand expectations for borrowed byte-input construction.
//!
//! These fixtures use published initialized region bytes, never a foreign copy
//! kernel. They separate required owned output from the borrowed source, exercise
//! typed forwarding and failure order, and calculate expected values independently
//! of compiler layout/checking code. Host source strings are test-only allocations.

use holo::layout::LayoutMode;

/// One expected semantic completion, independent of actual native pointer values.
#[derive(Clone, Copy)]
pub enum Outcome {
    /// Exact ordered unsigned decimal values from successful execution.
    Values(&'static [u64]),
    /// A checked bounds guard must terminate before its invalid source load.
    BoundsTrap {
        /// Exact source index selecting the one intended unoptimized native guard.
        index: u64,
        /// Exact model error, excluding unrelated setup/binding failures.
        model_message: &'static str,
    },
    /// A dynamic subrange reaches the real target bounds trap after a valid read.
    OffsetBoundsTrap,
}

/// Complete parsed input and independently specified outcomes for each layout.
pub struct Fixture {
    /// Stable case identifier used in native reproduction paths.
    pub label: &'static str,
    /// Complete Holo source, including the producers used by this case.
    pub source: String,
    /// Wide, Derived64, then Compact32 expectations; capacity policy can differ.
    pub expected: [Outcome; 3],
}

impl Fixture {
    /// Select a hand expectation without consulting the compiler's layout plan.
    /// `layout` selects one explicit configuration; the fixture remains unchanged.
    pub fn outcome(&self, layout: LayoutMode) -> Outcome {
        let index = match layout {
            LayoutMode::Wide => 0,
            LayoutMode::Derived64 => 1,
            LayoutMode::Compact32 => 2,
        };
        self.expected[index]
    }
}

/// Maintained numeric input fixture used to construct independent comparison sources.
const EXAMPLE: &str = include_str!("../fixtures/byte_input.holo");

/// Maintained typed frame walkthrough with observable payload and retry results.
const TYPED_FRAME: &str = include_str!("../fixtures/typed_frame.holo");

/// Combine the maintained producers with an additional producer and entry body.
/// This copies test text only; it neither evaluates Holo nor derives an oracle.
fn source(extra: &str, body: &str) -> String {
    let (definitions, _) = EXAMPLE
        .split_once("func main\n")
        .expect("maintained byte-input example must have a main declaration");
    format!(
        "{definitions}\n{extra}\n// Execute one bounded byte-input scenario.\nfunc main returns u64 {{\n{body}\n}}\n"
    )
}

/// Construct an explicit owned subrange using a checked readonly byte input.
/// The varying length exercises both representable and impossible requests.
const TYPED_RANGE: &str = r#"
error CopyError { EmptyInput }
// Produce any requested input extent before observing a copied subrange.
func initialized arg length u64 as readonly copy returns Payload {
    return Payload { data: core.bytes(length, core.add_wrapping(40, core.index())) }
}
// Deliberately leave ordinary source bounds checks to the initializer.
func copy_with_length
    arg length u64 as readonly copy
    arg input Bytes as readonly reference
    returns Result<Payload, CopyError>
{
    return ok Payload:
        data = core.bytes(length, input[core.add_wrapping(1, core.index())])
}
"#;

/// A tiny region separates width overflow, backing exhaustion, empty copying,
/// and successful copying from a retained published source in the same batch.
const TYPED_LIMITS: &str = r#"
    let memory = core.region(96) else { core.assert(false); return 0 }
    let rows = core.batch(memory, Payload)
    let first_status = core.append(rows, initialized(2))
    core.assert(first_status == 0)
    let empty_status = core.append(rows, initialized(0))
    core.assert(empty_status == 0)
    let first = core.borrow_readwrite(rows, 0)
    let empty = core.borrow_readonly(rows, 1)
    let original = core.address(first)
    let saved = core.used(memory)

    // Neither impossible request can reach the intentionally invalid source read.
    let overflow = core.append(rows, copy_with_length(18446744073709551615, empty.data))
        on ok           then 0
        on SizeOverflow then 2
        on error        then 1
    let width = core.append(rows, copy_with_length(4294967296, empty.data))
        on ok           then 0
        on SizeOverflow then 2
        on Full         then 1
        on error        then 3
    core.assert(core.used(memory) == saved)
    core.assert(core.len(rows) == 2)
    core.print(overflow)
    core.print(width)

    // An empty destination likewise skips the element expression entirely.
    let zero = core.append(rows, copy_with_length(0, empty.data))
        on ok    then true
        on error then false
    core.assert(zero)
    // The retained prefix can also supply a real copied byte to the suffix.
    let one = core.append(rows, copy_with_length(1, first.data))
        on ok    then true
        on error then false
    core.assert(one)
    core.assert(core.address(first) == original)
    let copy = core.borrow_readonly(rows, 3)
    first.data[1] = 99
    core.print(copy.data[0])
    return 0
"#;

/// A malformed requested range passes destination checks, reads its valid
/// first byte, then traps. Recoverable outcome arms cannot catch that trap.
const TYPED_RANGE_TRAP: &str = r#"
    let memory = core.region(96) else { core.assert(false); return 0 }
    let rows = core.batch(memory, Payload)
    let seed = core.append(rows, initialized(2))
    core.assert(seed == 0)
    let first = core.borrow_readonly(rows, 0)
    let unexpected = core.append(rows, copy_with_length(2, first.data))
        on ok    then 11
        on error then 12
    core.print(unexpected)
    return 0
"#;

/// Collect independent success/rollback/alias/empty/trap paths for source and native tests.
/// Layout-dependent overflow expectations preserve the existing explicit width policy.
pub fn fixtures() -> [Fixture; 15] {
    [
        Fixture {
            label: "byte-input-example",
            source: EXAMPLE.into(),
            expected: [Outcome::Values(&[0, 1, 1, 2663, 73, 0, 2]); 3],
        },
        Fixture {
            label: "typed-frame",
            source: TYPED_FRAME.into(),
            expected: [Outcome::Values(&[6, 8, 204]); 3],
        },
        Fixture {
            label: "typed-byte-input-capacity-and-overflow",
            source: source(TYPED_RANGE, TYPED_LIMITS),
            // An unrepresentable compact width wins before backing exhaustion.
            expected: [
                Outcome::Values(&[2, 1, 41]),
                Outcome::Values(&[2, 1, 41]),
                Outcome::Values(&[2, 2, 41]),
            ],
        },
        Fixture {
            label: "typed-offset-copy-late-bounds-trap",
            source: source(TYPED_RANGE, TYPED_RANGE_TRAP),
            expected: [Outcome::OffsetBoundsTrap; 3],
        },
        Fixture {
            label: "byte-input-alias-mutation",
            source: source(MIX, ALIASES),
            // Source changes 11->50 before copy, 50->99 after. Owned bytes stay 10,50,12.
            // Equal alias bytes xor away; low byte9 initializes mixed output, full id survives.
            expected: [Outcome::Values(&[0, 50, 99, 0x8000_0000_0000_0009, 9, 9, 0, 2, 1, 79]); 3],
        },
        Fixture {
            label: "byte-input-partial-rollback",
            source: source(PARTIAL, ROLLBACK),
            expected: [
                Outcome::Values(&[0, 3, 1, 13, 1, 1, 0, 2, 0, 3, 0]),
                Outcome::Values(&[0, 3, 1, 13, 1, 1, 0, 2, 0, 3, 0]),
                Outcome::Values(&[0, 3, 1, 13, 2, 1, 0, 2, 0, 3, 0]),
            ],
        },
        Fixture {
            label: "byte-input-empty-reservation-order",
            source: source(UNREADABLE, EMPTY),
            expected: [Outcome::Values(&[1, 0, 0, 0, 0, 2]); 3],
        },
        Fixture {
            label: "byte-input-element-bounds",
            source: source(UNREADABLE, ELEMENT_TRAP),
            expected: [Outcome::BoundsTrap {
                index: 4096,
                model_message: "input byte read bounds trap",
            }; 3],
        },
        Fixture {
            label: "byte-input-unused-scalar-argument",
            source: source(IGNORE, UNUSED_TRAP),
            expected: [Outcome::BoundsTrap {
                index: 1,
                model_message: "byte read bounds trap",
            }; 3],
        },
        Fixture {
            label: "byte-input-scalar-before-reservation",
            source: source(SCALAR_READ, SCALAR_TRAP),
            expected: [Outcome::BoundsTrap {
                index: 1,
                model_message: "input byte read bounds trap",
            }; 3],
        },
        Fixture {
            label: "byte-input-inlined-helper-argument",
            source: source(HELPER_IGNORE, HELPER_TRAP),
            expected: [Outcome::BoundsTrap {
                index: 1,
                model_message: "input byte read bounds trap",
            }; 3],
        },
        Fixture {
            label: "byte-input-exact-copy-boundaries",
            source: source("", COPY_BOUNDARIES),
            // Forty independently validated lengths include every 0..34 plus
            // 63/64/65/127/4096, with two required owned copies at each length.
            expected: [Outcome::Values(&[40, 10020]); 3],
        },
        Fixture {
            label: "byte-input-identity-helper-eager-actual",
            source: source(IDENTITY_HELPER, IDENTITY_HELPER_TRAP),
            // The empty input evaluates no element/helper actual. The nonempty
            // identity helper must still evaluate its ignored invalid actual.
            expected: [Outcome::BoundsTrap {
                index: 4096,
                model_message: "input byte read bounds trap",
            }; 3],
        },
        Fixture {
            label: "offset-copy-boundaries",
            source: source(OFFSET_COPY, OFFSET_BOUNDARIES),
            expected: [Outcome::Values(&[67, 4544]); 3],
        },
        Fixture {
            label: "offset-copy-late-bounds-trap",
            source: source(OFFSET_COPY, OFFSET_TRAP),
            expected: [Outcome::OffsetBoundsTrap; 3],
        },
    ]
}

/// Mixed argument order exercises full-width scalars between two aliasing byte borrows.
const MIX: &str = r#"
// Return a scalar unchanged; its byte-read actual must survive template substitution.
func identity with noalloc, noblock arg value u64 as readonly copy returns u64 {
    return value
}
// Borrow left/right for this call; retain the full scalar id and narrow only byte stores.
func mixed
    with noblock
    arg left Bytes as readonly reference
    arg id u64 as readonly copy
    arg right Bytes as readonly reference
    returns Entry
{
    return Entry {
        id: id
        payload: Payload {
            data: core.bytes(core.len(left), core.xor(left[core.index()], core.xor(identity(right[core.index()]), id)))
        }
    }
}
"#;

/// A retained writable alias may change source before/after a synchronous call.
const ALIASES: &str = r#"
    let memory = core.region(1024) else { return 0 }
    let entries = core.batch(memory, Entry)
    let seeded = core.append(entries, seed(10, 3))
    core.assert(seeded == 0)
    let writer = core.borrow_readwrite(entries, 0)
    let reader = core.alias_readonly(writer)
    let address = core.address(reader)
    writer.payload.data[1] = 50
    let output_memory = core.region(1024) else { return 0 }
    let outputs = core.batch(output_memory, Entry)
    let copied = core.append(outputs, from_bytes(7, writer.payload.data))
    core.print(copied)
    let copy = core.borrow_readonly(outputs, 0)
    writer.payload.data[1] = 99
    core.print(copy.payload.data[1])
    core.print(reader.payload.data[1])
    let combined = core.append(outputs, mixed(writer.payload.data, 0x8000000000000009, reader.payload.data))
    core.assert(combined == 0)
    let mix = core.borrow_readonly(outputs, 1)
    core.print(mix.id)
    core.print(mix.payload.data[0])
    core.print(mix.payload.data[2])
    let same = core.append(entries, from_bytes(13, reader.payload.data))
    core.print(same)
    core.print(core.len(entries))
    // The original record's address and initialized bytes survive same-root extension.
    if core.address(reader) == address { core.print(1) } else { core.print(0) }
    var total = copy.id
    // All bytes of the earlier owned copy still reflect the pre-call source values.
    for index in core.range(core.len(copy.payload.data)) {
        total = core.add_wrapping(total, copy.payload.data[index])
    }
    core.print(total)
    return 0
"#;

/// Two inline payloads make the second child's failure discard a populated first child.
const PARTIAL: &str = r#"
// Own two independently initialized inline payloads in declaration order.
struct Pair {
    // This child finishes before the second child's deliberate failure.
    first Payload
    // This child may reject or exhaust backing without publishing the parent.
    second Payload
}
// Borrow input and either reject or initialize length receiving bytes from it.
func maybe_payload
    with noblock
    arg input Bytes as readonly reference
    arg reject u64 as readonly copy
    arg length u64 as readonly copy
    returns Payload
{
    core.reject_if(reject == 1)
    return Payload { data: core.bytes(length, input[core.index()]) }
}
// Forward the same borrowed input through named and synthetic nested construction.
func pair
    with noblock
    arg input Bytes as readonly reference
    arg reject u64 as readonly copy
    arg length u64 as readonly copy
    returns Pair
{
    return Pair {
        first: Payload { data: core.bytes(core.len(input), input[core.index()]) }
        second: maybe_payload(input, reject, length)
    }
}
"#;

/// Rejection/overflow/capacity roll back only the private suffix, then permit retries/reset.
const ROLLBACK: &str = r#"
    let source_memory = core.region(1024) else { return 0 }
    let entries = core.batch(source_memory, Entry)
    let seeded = core.append(entries, seed(10, 4))
    core.assert(seeded == 0)
    let input = core.borrow_readonly(entries, 0)
    let original = core.address(input)
    let memory = core.region(256) else { return 0 }
    let pairs = core.batch(memory, Pair)
    let first = core.append(pairs, pair(input.payload.data, 0, 4))
    core.print(first)
    let saved = core.used(memory)
    {
        let retained = core.borrow_readonly(pairs, 0)
        let failed = core.append(pairs, pair(input.payload.data, 1, 4))
        core.print(failed)
        core.print(core.len(pairs))
        core.print(retained.first.data[3])
        let huge = core.append(pairs, pair(input.payload.data, 0, 4294967296))
        core.print(huge)
        core.assert(core.used(memory) == saved)
        core.assert(core.address(input) == original)
        core.assert(input.payload.data[3] == 13)
        // Failed private children must preserve every previously published output byte.
        for index in core.range(4) {
            core.assert(retained.first.data[index] == core.add_wrapping(10, index))
            core.assert(retained.second.data[index] == core.add_wrapping(10, index))
        }
        core.print(core.len(pairs))
        let retry = core.append(pairs, pair(input.payload.data, 0, 4))
        core.print(retry)
        core.print(core.len(pairs))
        let empty = core.append(pairs, pair(input.payload.data, 0, 0))
        core.print(empty)
        core.print(core.len(pairs))
    }
    core.reset(pairs)
    // Check the complete source after all failed and successful attempts.
    for index in core.range(4) {
        core.assert(input.payload.data[index] == core.add_wrapping(10, index))
    }
    core.print(core.used(memory))
    return 0
"#;

/// Only actual element evaluation can trap; length zero skips it and failed reserve precedes it.
const UNREADABLE: &str = r#"
// Copy no bytes for length0; otherwise index4096 exceeds even the256-byte backing.
func unreadable
    with noblock
    arg input Bytes as readonly reference
    arg length u64 as readonly copy
    returns Entry
{
    return Entry {
        id: 1
        payload: Payload { data: core.bytes(length, input[4096]) }
    }
}
"#;

/// Empty source input may share a numerical boundary with the new suffix without a load.
const EMPTY: &str = r#"
    let source_memory = core.region(256) else { return 0 }
    let sources = core.batch(source_memory, Entry)
    let seeded = core.append(sources, seed(0, 0))
    core.assert(seeded == 0)
    let input = core.borrow_readonly(sources, 0)
    let memory = core.region(64) else { return 0 }
    let output = core.batch(memory, Entry)
    let full = core.append(output, unreadable(input.payload.data, 1000))
    core.print(full)
    core.print(core.len(output))
    let zero = core.append(output, unreadable(input.payload.data, 0))
    core.print(zero)
    let empty = core.borrow_readonly(output, 0)
    core.print(core.len(empty.payload.data))
    let same = core.append(sources, unreadable(input.payload.data, 0))
    core.print(same)
    core.print(core.len(sources))
    return 0
"#;

/// A reserved nonempty destination reaches a real checked out-of-range input element.
const ELEMENT_TRAP: &str = r#"
    let memory = core.region(256) else { return 0 }
    let sources = core.batch(memory, Entry)
    let seeded = core.append(sources, seed(10, 1))
    core.assert(seeded == 0)
    let input = core.borrow_readonly(sources, 0)
    let failed = core.append(sources, unreadable(input.payload.data, 1))
    core.print(failed)
    return 0
"#;

/// A constructor's rejection cannot hide an invalid unused actual argument.
const IGNORE: &str = r#"
// The copied scalar is unused, but its caller expression still executes before this rejection.
func ignores
    with noblock
    arg input Bytes as readonly reference
    arg unused u64 as readonly copy
    returns Entry
{
    core.reject_if(core.len(input) == 1)
    return Entry { id: 0, payload: Payload { data: core.bytes(0, 0) } }
}
"#;

/// An invalid copied scalar after a byte borrow must trap before entering ignores.
const UNUSED_TRAP: &str = r#"
    let memory = core.region(256) else { return 0 }
    let sources = core.batch(memory, Entry)
    let seeded = core.append(sources, seed(10, 1))
    core.assert(seeded == 0)
    let input = core.borrow_readonly(sources, 0)
    let failed = core.append(sources, ignores(input.payload.data, input.payload.data[1]))
    core.print(failed)
    return 0
"#;

/// Scalar field evaluation still reads input before reserving that scalar's storage.
const SCALAR_READ: &str = r#"
// Deliberate index1 read from a one-byte input precedes the id field reservation.
func scalar_read with noblock arg input Bytes as readonly reference returns Entry {
    return Entry { id: input[1], payload: Payload { data: core.bytes(0, 0) } }
}
"#;

/// Wide's leading header fits8 bytes; the invalid scalar read must precede later exhaustion.
const SCALAR_TRAP: &str = r#"
    let memory = core.region(256) else { return 0 }
    let sources = core.batch(memory, Entry)
    let seeded = core.append(sources, seed(10, 1))
    core.assert(seeded == 0)
    let input = core.borrow_readonly(sources, 0)
    let tiny = core.region(8) else { return 0 }
    let output = core.batch(tiny, Entry)
    let failed = core.append(output, scalar_read(input.payload.data))
    core.print(failed)
    return 0
"#;

/// A helper's constant result must not erase evaluation of its trapping scalar actual.
const HELPER_IGNORE: &str = r#"
// This scalar result ignores its copied argument, but caller evaluation still occurs.
func ignore with noalloc, noblock arg value u64 as readonly copy returns u64 {
    return 0
}
// A producer calls the pure helper with a checked read, requiring eager capture.
func helper_ignore with noblock arg input Bytes as readonly reference returns Entry {
    return Entry { id: ignore(input[1]), payload: Payload { data: core.bytes(0, 0) } }
}
"#;

/// The invalid helper actual traps even though its result is independent of that value.
const HELPER_TRAP: &str = r#"
    let memory = core.region(256) else { return 0 }
    let sources = core.batch(memory, Entry)
    let seeded = core.append(sources, seed(10, 1))
    core.assert(seeded == 0)
    let input = core.borrow_readonly(sources, 0)
    let failed = core.append(sources, helper_ignore(input.payload.data))
    core.print(failed)
    return 0
"#;

/// Dynamic lengths cross every small-copy chunk boundary and the large fallback.
/// All complete output bytes are asserted against the independent seed formula;
/// writable aliases change byte zero between calls, proving copies are owned and
/// fresh, while source address retention and same-prefix appends remain valid.
const COPY_BOUNDARIES: &str = r#"
    var completed = 0
    var copied_bytes = 0
    for case in core.range(40) {
        var length = case
        if case == 35 { length = 63 }
        if case == 36 { length = 64 }
        if case == 37 { length = 65 }
        if case == 38 { length = 127 }
        if case == 39 { length = 4096 }
        let memory = core.region(16384) else { core.assert(false); return 0 }
        let entries = core.batch(memory, Entry)
        let seeded = core.append(entries, seed(17, length))
        core.assert(seeded == 0)
        let writer = core.borrow_readwrite(entries, 0)
        let reader = core.alias_readonly(writer)
        let address = core.address(reader)
        let other_memory = core.region(8192) else { core.assert(false); return 0 }
        let others = core.batch(other_memory, Entry)
        let copied = core.append(others, from_bytes(99, reader.payload.data))
        core.assert(copied == 0)
        let first = core.borrow_readonly(others, 0)
        if length > 0 { writer.payload.data[0] = 255 }
        let appended = core.append(entries, from_bytes(101, reader.payload.data))
        core.assert(appended == 0)
        let second = core.borrow_readonly(entries, 1)
        core.assert(core.address(reader) == address)
        core.assert(core.len(first.payload.data) == length)
        core.assert(core.len(second.payload.data) == length)
        for index in core.range(length) {
            let expected = core.and(core.add_wrapping(17, index), 255)
            core.assert(first.payload.data[index] == expected)
            if index == 0 {
                core.assert(second.payload.data[index] == 255)
            } else {
                core.assert(second.payload.data[index] == expected)
            }
        }
        completed = core.add_wrapping(completed, 1)
        copied_bytes = core.add_wrapping(copied_bytes, core.add_wrapping(
            core.len(first.payload.data), core.len(second.payload.data)))
    }
    core.print(completed)
    core.print(copied_bytes)
    return 0
"#;

/// A checked constructor reads a contiguous subrange of one captured input.
/// The independent scalar length leaves the source bounds check observable.
const OFFSET_COPY: &str = r#"
// Initialize the receiving payload directly from the original frozen source.
func copy_after_header
    with noblock
    arg input Bytes as readonly reference
    arg length u64 as readonly copy
    returns Entry
{
    return Entry {
        id: 7
        payload: Payload { data: core.bytes(length, input[core.add_wrapping(8, core.index())]) }
    }
}
"#;

/// Check every short length and two bulk lengths with both backing arrangements.
/// Empty input permits zero length, and failed reservation precedes source reads.
const OFFSET_BOUNDARIES: &str = r#"
    var total = 0
    for case in core.range(67) {
        var length = case
        // Include a second extent beyond the initial inline policy.
        if case == 66 { length = 127 }
        var source_length = core.add_wrapping(length, 8)
        // Zero length skips evaluation of the otherwise invalid offset.
        if length == 0 { source_length = 0 }
        let memory = core.region(1024) else { core.assert(false); return 0 }
        let entries = core.batch(memory, Entry)
        let seeded = core.append(entries, seed(17, source_length))
        core.assert(seeded == 0)
        let input = core.borrow_readwrite(entries, 0)
        let address = core.address(input)
        let other = core.region(512) else { core.assert(false); return 0 }
        let outputs = core.batch(other, Entry)
        if length == 0 {
            // The field cannot fit, so no byte of the empty input is read.
            let rejected = core.append(outputs, copy_after_header(input.payload.data, 512))
            core.assert(rejected == 1)
            core.assert(core.used(other) == 0)
        }
        let first_status = core.append(outputs, copy_after_header(input.payload.data, length))
        core.assert(first_status == 0)
        // A caller mutation affects only the subsequent copy from the live prefix.
        if length > 0 { input.payload.data[8] = 255 }
        let second_status = core.append(entries, copy_after_header(input.payload.data, length))
        core.assert(second_status == 0)
        core.assert(core.address(input) == address)
        let first = core.borrow_readonly(outputs, 0)
        let second = core.borrow_readonly(entries, 1)
        core.assert(core.len(first.payload.data) == length)
        core.assert(core.len(second.payload.data) == length)
        for index in core.range(length) {
            let expected = core.and(core.add_wrapping(25, index), 255)
            core.assert(first.payload.data[index] == expected)
            if index == 0 {
                core.assert(second.payload.data[index] == 255)
            } else {
                core.assert(second.payload.data[index] == expected)
            }
        }
        total = core.add_wrapping(total, core.add_wrapping(length, length))
    }
    core.print(67)
    core.print(total)
    return 0
"#;

/// The second read reaches the source boundary after a valid first read.
/// The destination has room, so capacity failure cannot hide the checked trap.
const OFFSET_TRAP: &str = r#"
    let memory = core.region(128) else { core.assert(false); return 0 }
    let entries = core.batch(memory, Entry)
    let seeded = core.append(entries, seed(17, 9))
    core.assert(seeded == 0)
    let input = core.borrow_readonly(entries, 0)
    let status = core.append(entries, copy_after_header(input.payload.data, 2))
    core.print(status)
    return 0
"#;

/// Returning the indexed byte cannot erase an additional eager trapping actual.
/// This looks like a copy after helper substitution, but its extra observation
/// is required whenever an element is initialized and must prevent bulk lowering.
const IDENTITY_HELPER: &str = r#"
// Return the first copied scalar while still evaluating the caller's second actual.
func take_first
    with noalloc, noblock
    arg value u64 as readonly copy
    arg ignored u64 as readonly copy
    returns u64
{
    return value
}
// Full input length alone cannot justify replacing this element expression.
func guarded_identity
    with noblock
    arg input Bytes as readonly reference
    returns Entry
{
    return Entry {
        id: 3
        payload: Payload {
            data: core.bytes(core.len(input), take_first(input[core.index()], input[4096]))
        }
    }
}
"#;

/// Empty output skips helper evaluation; a later nonempty output reaches index4096.
/// Both sources remain published and live. The second append has ample capacity,
/// so the intended bounds guard must terminate before an out-of-backing load.
const IDENTITY_HELPER_TRAP: &str = r#"
    let memory = core.region(256) else { return 0 }
    let sources = core.batch(memory, Entry)
    let empty_status = core.append(sources, seed(10, 0))
    core.assert(empty_status == 0)
    let empty = core.borrow_readonly(sources, 0)
    let output_memory = core.region(256) else { return 0 }
    let outputs = core.batch(output_memory, Entry)
    let skipped = core.append(outputs, guarded_identity(empty.payload.data))
    core.assert(skipped == 0)
    {
        let output = core.borrow_readonly(outputs, 0)
        core.assert(output.id == 3)
        core.assert(core.len(output.payload.data) == 0)
    }
    // Nine bytes would enter an eight-byte vector prefix and leave a scalar tail.
    // The ignored invalid helper actual must still trap before any bulk shortcut.
    let seeded = core.append(sources, seed(10, 9))
    core.assert(seeded == 0)
    let input = core.borrow_readonly(sources, 1)
    let failed = core.append(outputs, guarded_identity(input.payload.data))
    core.print(failed)
    return 0
"#;
