//! Low-level storage instructions for the checked-region LLVM backend.
//!
//! These helpers know the backend metadata layout and packed field encoding. They
//! do not resolve source names or decide whether a reference is legal. All caller
//! preconditions come from the checked representation; runtime reservation and
//! indexing checks remain where a value is not statically established.

use super::function::Function;

/// Locate a fixed metadata member and return its LLVM pointer value.
///
/// `kind` is the compiler-owned region/batch LLVM type, `base` its pointer, and
/// `index` the established field index. Only compiler text is interpolated.
pub(super) fn member(f: &mut Function, kind: &str, base: &str, index: u32) -> String {
    f.value(format!(
        "getelementptr {kind}, ptr {base}, i32 0, i32 {index}"
    ))
}

/// Load a region's next-unused offset from metadata without changing ownership.
///
/// `region` points to initialized runtime metadata; the result is an SSA i64.
pub(super) fn top(f: &mut Function, region: &str) -> String {
    let slot = member(f, "%holo.region", region, 2);
    f.value(format!("load i64, ptr {slot}"))
}

/// Set a region's marker with one metadata store.
///
/// `cursor` is a previously established offset or a checked successful end.
/// A rewind requires that references into the discarded suffix are unusable.
pub(super) fn restore(f: &mut Function, region: &str, cursor: &str) {
    let slot = member(f, "%holo.region", region, 2);
    f.line(format!("store i64 {cursor}, ptr {slot}"));
}

/// Commit a fixed subtree's checked extent and return its end cursor.
/// Success proves all constituent advances fit. The fixed subtree initialized
/// its final bytes without mutating the region marker, so one store advances it.
pub(super) fn commit_fixed(f: &mut Function, region: &str, start: &str, bytes: u64) -> String {
    let end = f.value(format!("add i64 {start}, {bytes}"));
    restore(f, region, &end);
    end
}

/// Compute the unreserved tail address without dereferencing it.
///
/// `region` is initialized backing metadata. The resulting pointer may only be
/// written after a successful reservation; no inbounds or alias promise is made.
pub(super) fn tail(f: &mut Function, region: &str) -> String {
    tail_position(f, region).1
}

/// Capture the current offset and its unwritten tail pointer in one pass.
/// The caller validates the reservation before writing through the pointer.
pub(super) fn tail_position(f: &mut Function, region: &str) -> (String, String) {
    let base = load_base(f, region);
    let offset = top(f, region);
    let address = advance(f, &base, &offset);
    (offset, address)
}

/// Compute an unwritten destination from an offset the caller already captured.
/// As with tail, the pointer gains no inbounds or dereferenceable guarantee.
pub(super) fn address_at(f: &mut Function, region: &str, offset: &str) -> String {
    let base = load_base(f, region);
    advance(f, &base, offset)
}

/// Read the backing pointer from initialized, compiler-owned region metadata.
fn load_base(f: &mut Function, region: &str) -> String {
    let base_slot = member(f, "%holo.region", region, 0);
    f.value(format!("load ptr, ptr {base_slot}"))
}

/// Advance a byte pointer by a known-safe or already-validated offset.
///
/// `pointer` and `bytes` are LLVM values. This helper performs no allocation and
/// deliberately omits `inbounds`; the caller owns the relevant range proof.
pub(super) fn advance(f: &mut Function, pointer: &str, bytes: &str) -> String {
    f.value(format!("getelementptr i8, ptr {pointer}, i64 {bytes}"))
}

/// Read one packed u64 field, including compiler-maintained size metadata.
///
/// `pointer` must address eight initialized bytes of the established field type.
/// Alignment 1 preserves validity for all inline variable-sized field positions.
pub(super) fn load_u64(f: &mut Function, pointer: &str) -> String {
    f.value(format!("load i64, ptr {pointer}, align 1"))
}

/// Write one packed u64 field at its already-validated destination.
///
/// `pointer` must identify eight writable bytes and `value` is an i64 SSA value.
/// The checker, not this operation, distinguishes user fields from shape metadata.
pub(super) fn store_u64(f: &mut Function, pointer: &str, value: &str) {
    f.line(format!("store i64 {value}, ptr {pointer}, align 1"));
}

/// Read a canonical boolean byte as the native boolean type.
pub(super) fn load_bool(f: &mut Function, pointer: &str) -> String {
    let byte = f.value(format!("load i8, ptr {pointer}, align 1"));
    f.value(format!("trunc i8 {byte} to i1"))
}

/// Store an i1 boolean as a fully initialized byte containing zero or one.
pub(super) fn store_bool(f: &mut Function, pointer: &str, value: &str) {
    let byte = f.value(format!("zext i1 {value} to i8"));
    f.line(format!("store i8 {byte}, ptr {pointer}, align 1"));
}

/// Compiler-side cursor and field offsets for one checked fixed producer.
///
/// Each successful reserve supplies the next cursor in SSA. Nested success
/// advances by the child's exact fixed extent. The checked fixed subtree keeps
/// all advances in SSA. The caller updates the native marker only on success.
pub(super) struct FixedReservations {
    /// Native region cursor at `cursor_offset` in the receiving record.
    cursor: String,
    /// Static offset already represented by the cached native cursor.
    cursor_offset: u64,
    /// Static offset of the next field, including successfully built children.
    next_offset: u64,
    /// The initialized capacity operand inherited from the caller.
    capacity: String,
}

impl FixedReservations {
    /// Start at the entry cursor, where the caller has supplied the final slot.
    /// Both cursor and capacity are private call operands, never native slots.
    pub(super) fn new(cursor: String, capacity: String) -> Self {
        Self {
            cursor,
            cursor_offset: 0,
            next_offset: 0,
            capacity,
        }
    }

    /// Form the next field address relative to the caller's receiving pointer.
    /// Only an ordered successful reservation makes this address writable.
    pub(super) fn destination(&self, f: &mut Function, base: &str) -> String {
        // The first field starts at the supplied pointer without extra work.
        if self.next_offset == 0 {
            base.to_owned()
        } else {
            advance(f, base, &self.next_offset.to_string())
        }
    }

    /// Record a checked child's exact successful extent without native code.
    /// Its own reservations checked the advance. Any later reserve recovers
    /// the current cursor from the cached value and the exact extent.
    pub(super) fn complete_child(&mut self, bytes: u64) {
        self.advance_offset(bytes);
    }

    /// Return the capacity operand shared unchanged with every fixed child.
    pub(super) fn capacity(&self) -> &str {
        &self.capacity
    }

    /// Materialize the current cursor for a reserve or a nested producer.
    /// Successful children have already checked any advancement since the
    /// previous cursor operand, so the generated addition cannot wrap.
    pub(super) fn cursor(&mut self, f: &mut Function) -> &str {
        let child_bytes = self.next_offset - self.cursor_offset;
        // A later sibling can need the advanced cursor even if the parent
        // has performed no reservation of its own since its last child.
        if child_bytes != 0 {
            self.cursor = f.value(format!("add i64 {}, {child_bytes}", self.cursor));
            self.cursor_offset = self.next_offset;
        }
        &self.cursor
    }

    /// Reserve `bytes` with the ordinary overflow and exhaustion order.
    /// `base` is the record's final destination. The result may be written
    /// on success, when the cached cursor advances without a metadata store.
    pub(super) fn reserve(
        &mut self,
        f: &mut Function,
        base: &str,
        bytes: u64,
        prefer_success: bool,
        returned: super::producer::ProducerReturn,
    ) -> String {
        let saved = self.cursor(f).to_owned();
        let next = checked_end(
            f,
            &saved,
            &bytes.to_string(),
            prefer_success,
            returned,
            |_| self.capacity.clone(),
        );
        let address = self.destination(f, base);
        self.advance_offset(bytes);
        self.cursor = next;
        self.cursor_offset = self.next_offset;
        address
    }

    /// Move the compiler's field offset within the checked fixed record size.
    /// A layout violating that invariant is a compiler bug, never native wrap.
    fn advance_offset(&mut self, bytes: u64) {
        self.next_offset = self
            .next_offset
            .checked_add(bytes)
            .expect("checked fixed layout contains every field extent");
    }
}

/// A typed variable-size subtree's private progress through the receiving suffix.
///
/// The two address operands describe the same next-unused byte. Each reserve
/// checks the absolute cursor before moving the pointer. Calls inherit the
/// immutable capacity and the append owns the only published marker update.
pub(super) struct DeferredReservations {
    /// Absolute region offset of the next reservation.
    pub(super) cursor: String,
    /// Unwritten receiving pointer at that offset.
    pub(super) tail: String,
}

impl DeferredReservations {
    /// Capture the incoming cursor and final address, with no metadata reads.
    pub(super) fn new() -> Self {
        Self {
            cursor: "%initial.top".to_owned(),
            tail: "%destination".to_owned(),
        }
    }

    /// Reserve the requested bytes in order, returning the start only on success.
    /// The branch checks overflow and capacity before forming the next pointer.
    pub(super) fn reserve(
        &mut self,
        f: &mut Function,
        bytes: &str,
        prefer_success: bool,
        returned: super::producer::ProducerReturn,
    ) -> String {
        self.cursor = checked_end(f, &self.cursor, bytes, prefer_success, returned, |_| {
            "%capacity".to_owned()
        });
        let next = advance(f, &self.tail, bytes);
        std::mem::replace(&mut self.tail, next)
    }

    /// Advance past a successful child. Its own ordered checks prove the
    /// exact byte distance, so the continuation introduces no second reserve.
    pub(super) fn complete_child(
        &mut self,
        f: &mut Function,
        fixed_bytes: Option<u64>,
        outcome: &str,
    ) {
        // Fixed children preserve the original empty success lane. A variable
        // child returns its checked absolute end in that lane on success.
        let (end, bytes) = if let Some(bytes) = fixed_bytes {
            let end = f.value(format!("add i64 {}, {bytes}", self.cursor));
            (end, bytes.to_string())
        } else {
            let end = f.value(format!("extractvalue {{ i64, i64 }} {outcome}, 1"));
            let bytes = f.value(format!("sub i64 {end}, {}", self.cursor));
            (end, bytes)
        };
        self.tail = advance(f, &self.tail, &bytes);
        self.cursor = end;
    }
}

/// Reserve bytes by directly emitting ordered checked producer control flow.
///
/// `region` supplies backing and `bytes` is the requested unsigned count. Return
/// the initialized reservation's address on the continuation path. Overflow
/// returns status 2 before checking capacity; exhaustion returns status 1. Failure
/// leaves the marker unchanged. Append restores any earlier private suffix.
///
/// The checks match the former runtime helper exactly. Emitting their actual
/// branches avoids materializing a {pointer,status} pair and immediately testing
/// its status, which produced redundant native status branches after inlining.
/// `prefer_success` enables only the scalar-input producer's experimental layout
/// hint; borrowed-input callers retain their original unweighted branches.
/// `returned` selects the producer's checked status representation. It changes
/// only the failure return instruction, preserving the reservation checks.
pub(super) fn reserve(
    f: &mut Function,
    region: &str,
    bytes: &str,
    prefer_success: bool,
    returned: super::producer::ProducerReturn,
) -> String {
    let saved = top(f, region);
    let next = checked_end(f, &saved, bytes, prefer_success, returned, |f| {
        load_capacity(f, region)
    });
    let address = address_at(f, region, &saved);
    restore(f, region, &next);
    address
}

/// Capture immutable capacity from initialized, compiler-owned metadata.
/// A reservation only checks this bound on the nonoverflowing path.
pub(super) fn load_capacity(f: &mut Function, region: &str) -> String {
    let slot = member(f, "%holo.region", region, 1);
    f.value(format!("load i64, ptr {slot}"))
}

/// Check unsigned advancement before requesting the capacity operand.
/// Emit both failure returns and leave the emitter on the success continuation.
/// The returned SSA cursor is valid there and this helper never writes metadata.
fn checked_end(
    f: &mut Function,
    saved: &str,
    bytes: &str,
    prefer_success: bool,
    returned: super::producer::ProducerReturn,
    capacity: impl FnOnce(&mut Function) -> String,
) -> String {
    let addition = f.value(format!(
        "call {{ i64, i1 }} @llvm.uadd.with.overflow.i64(i64 {saved}, i64 {bytes})"
    ));
    let next = f.value(format!("extractvalue {{ i64, i1 }} {addition}, 0"));
    let overflow = f.value(format!("extractvalue {{ i64, i1 }} {addition}, 1"));
    let capacity_check = f.fresh_label();
    let overflowed = f.fresh_label();
    // Reject wrap before considering capacity or forming any destination address.
    f.construction_branch(
        &overflow,
        &overflowed,
        &capacity_check,
        prefer_success.then_some(false),
    );
    f.label(&overflowed);
    returned.return_status(f, 2);
    f.label(&capacity_check);
    let capacity = capacity(f);
    let fits = f.value(format!("icmp ule i64 {next}, {capacity}"));
    let ready = f.fresh_label();
    let exhausted = f.fresh_label();
    // Exhaustion preserves the original marker and exposes no writable address.
    f.construction_branch(&fits, &ready, &exhausted, prefer_success.then_some(true));
    f.label(&exhausted);
    returned.return_status(f, 1);
    f.label(&ready);
    next
}
