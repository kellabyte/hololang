//! Actual experimental Holo source for independent region-owner lifetime checks.
//!
//! Fixture expectations are hand derived from source values, never compiler
//! layout calculations. Addresses are compared only within one execution. Small
//! capacities fit every supported layout; deliberate failures exceed all three.

/// Parsed source and independently expected observations for one lifecycle path.
pub struct Fixture {
    /// Stable identifier used in generated evidence paths.
    pub label: &'static str,
    /// Complete source accepted by the experimental frontend.
    pub source: String,
    /// Exact decimal output, independent of physical record metadata width.
    pub expected: &'static [u64],
    /// Number of successful real backing acquisitions and matching frees.
    pub acquisitions: u64,
    /// Request size to fail at every matching real allocator boundary, or no injection.
    pub failed_capacity: Option<u64>,
    /// Exact number of expected NULL injections, separate from successful owners.
    pub expected_faults: u64,
}

/// Shared producers construct initialized bytes in their receiving destination.
pub(super) const DEFINITIONS: &str = r#"
// Each record has an inline scalar and variable-length byte payload.
struct Item {
    // Application value used to observe alias mutation and published record identity.
    id u64
    // Inline initialized bytes whose extent contributes to finite backing use.
    data Bytes
}
// Copy id/length scalars and construct the Item directly in its receiving slot.
// Byte values wrap from index+id; append owns reservation failure and publication.
func item
    with noblock
    arg id u64 as readonly copy
    arg length u64 as readonly copy
    returns Item
{
    return Item {
        id: id
        data: core.bytes(length, core.add_wrapping(id, core.index()))
    }
}
// The same scalar inputs produce Item unless id99 rejects before construction.
// Rejection must not publish any new record or marker; no source reference escapes.
func rejected
    with noblock
    arg id u64 as readonly copy
    arg length u64 as readonly copy
    returns Item
{
    core.reject_if(id == 99)
    return Item {
        id: id
        data: core.bytes(length, core.add_wrapping(id, core.index()))
    }
}
"#;

/// Return the continuation path: inner failure/reset/reuse leaves outer aliases live.
/// The outer cursor captures one record; its later successful append is excluded.
pub fn continuation() -> Fixture {
    Fixture {
        label: "owner-continuation",
        source: format!("{DEFINITIONS}\n{CONTINUATION}"),
        // Inner results: rejection, capacity, explicit reuse, lexical reuse.
        // Outer records sum to (90+10+11+12)+(11+11+12)=157 after mutation.
        expected: &[3, 1, 200, 300, 1, 2, 90, 157, 0, 7, 0],
        acquisitions: 2,
        failed_capacity: None,
        expected_faults: 0,
    }
}

/// Inner resets are legal despite unrelated outer aliases and a captured cursor.
const CONTINUATION: &str = r#"
// Continue after every recoverable inner failure and normal inner cleanup.
func main returns u64 {
    let outer = core.region(256) else { core.print(990); return 0 }
    {
        let outer_items = core.batch(outer, Item)
        let initial = core.append(outer_items, item(10, 3))
        core.assert(initial == 0)
        {
            let writer = core.borrow_readwrite(outer_items, 0)
            let reader = core.alias_readonly(writer)
            let original = core.address(reader)
            var visits = 0
            // The one-record entry prefix stays fixed while its owner grows below.
            for readonly visible in outer_items {
                visits = core.add_wrapping(visits, 1)
                let inner = core.region(64) else { core.print(991); return 0 }
                var saved = 0
                {
                    let inner_items = core.batch(inner, Item)
                    let first = core.append(inner_items, item(100, 4))
                    core.assert(first == 0)
                    let published = core.used(inner)
                    {
                        let retained = core.borrow_readonly(inner_items, 0)
                        saved = core.address(retained)
                        let rejected_status = core.append(inner_items, rejected(99, 1))
                        core.assert(rejected_status == 3)
                        core.print(rejected_status)
                        let full_status = core.append(inner_items, item(8, 128))
                        core.assert(full_status == 1)
                        core.print(full_status)
                        core.assert(core.used(inner) == published)
                        core.assert(core.len(inner_items) == 1)
                        core.assert(retained.id == 100)
                        core.assert(retained.data[3] == 103)
                        core.assert(core.address(retained) == saved)
                    }
                    // Only inner references ended; outer aliases and cursor remain usable.
                    core.reset(inner_items)
                    core.assert(core.used(inner) == 0)
                    core.assert(core.len(inner_items) == 0)
                    core.assert(reader.id == 10)
                    core.assert(core.address(visible) == original)
                    let reuse = core.append(inner_items, item(200, 2))
                    core.assert(reuse == 0)
                    {
                        let reused = core.borrow_readonly(inner_items, 0)
                        core.assert(core.address(reused) == saved)
                        core.assert(reused.data[1] == 201)
                        core.print(reused.id)
                    }
                    core.reset(inner_items)
                    core.assert(core.used(inner) == 0)
                }
                core.assert(core.used(inner) == 0)
                {
                    // A sibling batch reuses the same region after lexical tail cleanup.
                    let sibling = core.batch(inner, Item)
                    let appended = core.append(sibling, item(300, 1))
                    core.assert(appended == 0)
                    let again = core.borrow_readonly(sibling, 0)
                    core.assert(core.address(again) == saved)
                    core.print(again.id)
                }
                core.assert(core.used(inner) == 0)
                writer.id = 90
                core.assert(reader.id == 90)
                core.assert(visible.id == 90)
                core.assert(core.address(reader) == original)
                let extended = core.append(outer_items, item(11, 2))
                core.assert(extended == 0)
            }
            core.print(visits)
            core.print(core.len(outer_items))
            core.print(reader.id)
            core.assert(core.address(reader) == original)
            var sum = 0
            // This fresh scan includes both records and checks every published payload byte.
            for readonly record in outer_items {
                sum = core.add_wrapping(sum, record.id)
                for byte in core.range(core.len(record.data)) {
                    sum = core.add_wrapping(sum, record.data[byte])
                }
            }
            core.print(sum)
        }
        core.reset(outer_items)
        core.print(core.used(outer))
        let reused_outer = core.append(outer_items, item(7, 0))
        core.assert(reused_outer == 0)
        let final_record = core.borrow_readonly(outer_items, 0)
        core.print(final_record.id)
    }
    core.print(core.used(outer))
    return 0
}
"#;

/// Return an early exit after rollback, with an already-closed sibling owner.
/// `rejection` selects status 3 versus finite-capacity status 1, without changing safety.
pub fn append_exit(rejection: bool) -> Fixture {
    // Both cases exercise the same source control flow and independent expected values.
    let (label, call, status, expected): (_, _, _, &'static [u64]) = if rejection {
        (
            "owner-rejection-exit",
            "rejected(99, 1)",
            "3",
            &[3, 41, 1, 17],
        )
    } else {
        ("owner-capacity-exit", "item(99, 128)", "1", &[1, 41, 1, 17])
    };
    Fixture {
        label,
        source: format!(
            "{DEFINITIONS}\n{}",
            APPEND_EXIT
                .replace("FAIL_CALL", call)
                .replace("FAIL_STATUS", status)
        ),
        expected,
        acquisitions: 4,
        failed_capacity: None,
        expected_faults: 0,
    }
}

/// Three active owners unwind after rollback; a previous sibling was already freed.
const APPEND_EXIT: &str = r#"
// Observe retained outer and inner references before successful entry completion.
func main returns u64 {
    let outer = core.region(256) else { core.print(990); return 0 }
    let outer_items = core.batch(outer, Item)
    let first = core.append(outer_items, item(41, 2))
    core.assert(first == 0)
    let outer_reader = core.borrow_readonly(outer_items, 0)
    let original = core.address(outer_reader)
    {
        // This owner is gone before the later early return walks active cleanups.
        let finished = core.region(32) else { core.print(991); return 0 }
        core.assert(core.used(finished) == 0)
    }
    let middle = core.region(128) else { core.print(992); return 0 }
    let middle_items = core.batch(middle, Item)
    let middle_status = core.append(middle_items, item(5, 0))
    core.assert(middle_status == 0)
    let inner = core.region(64) else { core.print(993); return 0 }
    let inner_items = core.batch(inner, Item)
    let inner_status = core.append(inner_items, item(17, 2))
    core.assert(inner_status == 0)
    let inner_reader = core.borrow_readonly(inner_items, 0)
    let published = core.used(inner)
    let failed = core.append(inner_items, FAIL_CALL)
    core.assert(failed == FAIL_STATUS)
    core.assert(core.used(inner) == published)
    core.assert(core.len(inner_items) == 1)
    core.assert(core.address(outer_reader) == original)
    core.assert(outer_reader.data[1] == 42)
    // The selected failure branch exits while old aliases still read initialized data.
    if failed == FAIL_STATUS {
        core.print(failed)
        core.print(outer_reader.id)
        core.print(core.len(inner_items))
        core.print(inner_reader.id)
        return 0
    }
    core.print(999)
    return 0
}
"#;

/// Return source whose real 4096-byte allocation request is injected to fail.
/// The model receives the same parsed program with only that named fault toggled.
pub fn acquire_exit() -> Fixture {
    Fixture {
        label: "owner-acquire-exit",
        source: format!("{DEFINITIONS}\n{ACQUIRE_EXIT}"),
        expected: &[61, 62],
        acquisitions: 3,
        failed_capacity: Some(4096),
        expected_faults: 1,
    }
}

/// A failed owner is never installed; enclosing owners still clean up after recovery.
const ACQUIRE_EXIT: &str = r#"
// Failure recovery reads an outer alias, then exits through only acquired owners.
func main returns u64 {
    let outer = core.region(256) else { core.print(990); return 0 }
    let outer_items = core.batch(outer, Item)
    let first = core.append(outer_items, item(61, 2))
    core.assert(first == 0)
    let reader = core.borrow_readonly(outer_items, 0)
    let original = core.address(reader)
    {
        // Cleanup must not retain this finished sibling in the active-owner stack.
        let finished = core.region(32) else { core.print(991); return 0 }
        core.assert(core.used(finished) == 0)
    }
    let middle = core.region(128) else { core.print(992); return 0 }
    let middle_items = core.batch(middle, Item)
    let published = core.append(middle_items, item(5, 0))
    core.assert(published == 0)
    // Only the test allocator boundary fails this request; emitted source follows NULL.
    let unavailable = core.region(4096) else {
        core.assert(core.address(reader) == original)
        core.assert(core.len(outer_items) == 1)
        core.print(reader.id)
        core.print(reader.data[1])
        return 0
    }
    core.print(999)
    return 0
}
"#;
