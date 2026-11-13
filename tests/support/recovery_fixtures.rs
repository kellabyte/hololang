//! Actual source cases for fallible acquisition recovery and later continuation.
//!
//! Cases share only schema/producer text and the owning fixture type with the
//! established owner suite. Their outputs/acquisition counts are hand derived;
//! source records remain Holo operations, and NULL injection stays in the native
//! observer. Host strings are test inputs, never a runtime allocation strategy.

use super::owner_fixtures::{DEFINITIONS, Fixture};

/// Return the exact maintained recovery example with its ordinary success outcome.
/// Native failure/retry coverage is supplied by the distinct injected fixtures;
/// this source is included directly so documentation cannot drift from execution.
pub fn maintained_recovery() -> Fixture {
    Fixture {
        label: "owner-maintained-recovery-example",
        source: include_str!("../fixtures/recovery.holo").into(),
        expected: &[0],
        acquisitions: 1,
        failed_capacity: None,
        expected_faults: 0,
    }
}

/// Return identical source for normal acquisition and an injected recoverable failure.
/// `fail` selects only the test allocator fault and independently expected output;
/// the handler's replacement owner ends before later enclosing work continues.
pub fn acquire_retry(fail: bool) -> Fixture {
    // Success and recovery create two actual owners each, but their second owner
    // differs. Neither outcome may run both the handler and success-only tail.
    let (label, expected, failed_capacity): (_, &'static [u64], _) = if fail {
        ("owner-acquire-retry", &[70, 71, 1, 2, 62], Some(4096))
    } else {
        ("owner-acquire-success-tail", &[80, 2, 2, 62], None)
    };
    Fixture {
        label,
        source: format!("{DEFINITIONS}\n{ACQUIRE_RETRY}"),
        expected,
        acquisitions: 2,
        failed_capacity,
        expected_faults: u64::from(fail),
    }
}

/// Recovery borrows only the live outer prefix, constructs a replacement in its
/// own owner, then rejoins after the failed owner's complete lexical block.
const ACQUIRE_RETRY: &str = r#"
// A NULL acquisition skips its success tail; the handler may recover normally.
func main returns u64 {
    let outer = core.region(256) else { core.print(990); return 0 }
    let outer_items = core.batch(outer, Item)
    let seeded = core.append(outer_items, item(61, 2))
    core.assert(seeded == 0)
    let reader = core.borrow_readonly(outer_items, 0)
    let original = core.address(reader)
    var recovered = 0
    {
        let unavailable = core.region(4096) else {
            core.assert(reader.id == 61)
            core.assert(core.len(reader.data) == 2)
            core.assert(core.address(reader) == original)
            // Check every published outer byte while the failed owner is absent.
            for index in core.range(2) {
                core.assert(reader.data[index] == core.add_wrapping(61, index))
            }
            {
                let replacement = core.region(64) else { core.assert(false); return 0 }
                let retry_items = core.batch(replacement, Item)
                let retry = core.append(retry_items, item(70, 2))
                core.assert(retry == 0)
                let result = core.borrow_readonly(retry_items, 0)
                core.assert(core.len(result.data) == 2)
                core.assert(result.data[0] == 70)
                core.print(result.id)
                core.print(result.data[1])
            }
            // Only an enclosing scalar is changed; the replacement has ended.
            recovered = 1
        }
        // This entire tail executes only after actual successful acquisition.
        let successful_items = core.batch(unavailable, Item)
        let success = core.append(successful_items, item(80, 1))
        core.assert(success == 0)
        let result = core.borrow_readonly(successful_items, 0)
        core.assert(core.len(result.data) == 1)
        core.assert(result.data[0] == 80)
        core.print(result.id)
        recovered = 2
    }
    // Both acquisition paths have released their inner owners before continuing.
    core.print(recovered)
    core.assert(core.address(reader) == original)
    core.assert(reader.id == 61)
    let appended = core.append(outer_items, item(90, 1))
    core.assert(appended == 0)
    core.print(core.len(outer_items))
    core.print(reader.data[1])
    return 0
}
"#;

/// Return the two actual outcomes of a recovery handler with a conditional exit.
/// Both cases inject the same one acquisition fault; the live outer record's id
/// chooses early completion versus normal handler fallthrough and later work.
pub fn acquire_branch(early: bool) -> Fixture {
    // Both source paths retain one outer owner and must never acquire the failed
    // inner owner. Exact outputs distinguish an exit from accidental continuation.
    let (label, id, expected): (_, _, &'static [u64]) = if early {
        ("owner-acquire-handler-exit", "71", &[71, 72])
    } else {
        ("owner-acquire-handler-continue", "72", &[72, 1, 73])
    };
    Fixture {
        label,
        source: format!(
            "{DEFINITIONS}\n{}",
            ACQUIRE_BRANCH.replace("INITIAL_ID", id)
        ),
        expected,
        acquisitions: 1,
        failed_capacity: Some(4096),
        expected_faults: 1,
    }
}

/// One handler branch exits through active outer cleanup; the other returns to
/// the enclosing block with its initialized outer prefix and reference intact.
const ACQUIRE_BRANCH: &str = r#"
// Partial termination in recovery does not require every branch to end entry.
func main returns u64 {
    let outer = core.region(256) else { core.print(990); return 0 }
    let outer_items = core.batch(outer, Item)
    let seeded = core.append(outer_items, item(INITIAL_ID, 2))
    core.assert(seeded == 0)
    let reader = core.borrow_readonly(outer_items, 0)
    let original = core.address(reader)
    var recovered = 0
    {
        let unavailable = core.region(4096) else {
            core.assert(core.address(reader) == original)
            core.assert(core.len(reader.data) == 2)
            // Validate both initialized bytes before either handler control path.
            for index in core.range(2) {
                core.assert(reader.data[index] == core.add_wrapping(reader.id, index))
            }
            core.print(reader.id)
            if reader.id == 71 {
                core.print(reader.data[1])
                return 0
            }
            recovered = 1
        }
        // Injection makes this success-only sentinel unreachable in both fixtures.
        core.print(999)
    }
    core.print(recovered)
    core.assert(core.address(reader) == original)
    core.print(reader.data[1])
    return 0
}
"#;

/// Return source whose original acquisition and handler's retry both receive NULL.
/// Neither failed owner is installed; the second handler reads live outer bytes
/// and exits through only the one successfully acquired owner's cleanup.
pub fn acquire_retry_failure() -> Fixture {
    Fixture {
        label: "owner-acquire-retry-failure",
        source: format!("{DEFINITIONS}\n{ACQUIRE_RETRY_FAILURE}"),
        expected: &[61, 62],
        acquisitions: 1,
        failed_capacity: Some(4096),
        expected_faults: 2,
    }
}

/// Two failed acquisitions are distinct from live owners. The exact two injected
/// requests reach nested recovery, whose early exit must not double-free outer.
const ACQUIRE_RETRY_FAILURE: &str = r#"
// A recovery allocation may fail too; cleanup still visits only acquired owners.
func main returns u64 {
    let outer = core.region(256) else { core.print(990); return 0 }
    let outer_items = core.batch(outer, Item)
    let seeded = core.append(outer_items, item(61, 2))
    core.assert(seeded == 0)
    let reader = core.borrow_readonly(outer_items, 0)
    let original = core.address(reader)
    {
        let unavailable = core.region(4096) else {
            core.assert(core.address(reader) == original)
            core.assert(reader.id == 61)
            core.assert(core.len(reader.data) == 2)
            core.print(reader.data[0])
            let unavailable_retry = core.region(4096) else {
                core.assert(core.address(reader) == original)
                core.assert(reader.id == 61)
                core.assert(reader.data[0] == 61)
                core.print(reader.data[1])
                return 0
            }
            // Both injected requests must take recovery, never this success tail.
            core.print(998)
        }
        core.print(999)
    }
    core.print(997)
    return 0
}
"#;
