//! Native boundary for checked synchronous refill and explicit readiness wait.
//!
//! Admission establishes the sole descriptor before any attempt. Lowering
//! checks bounds before handing the existing slot to a direct write, then
//! validates the adapter result before lending the exact fresh extent to the
//! Data arm. The view is a pair of SSA operands and introduces no copy.

use crate::ir::Refill;

use super::{
    context::{Binding, Lowering},
    input::Input,
};

/// Link contract with the separately built experimental native adapter.
pub(super) const DECLARATIONS: &str = "declare i64 @holo_refill_experimental(i64, i64, ptr, i64, i64)\n\
     declare i64 @holo_wait_input_experimental(i64)\n\n";

/// Maximum request and descriptor representable by the admitted native ABI.
pub(super) const MAX_NATIVE: i32 = i32::MAX;

impl Lowering<'_> {
    /// Confirm that the trusted host supplied a recognized kind and native fd.
    ///
    /// The caller is responsible for actual handle admission and stable mode
    /// through the call. These emitted checks trap before any operation on a
    /// mismatch and do not acquire or release the descriptor.
    pub(super) fn guard_refill_host(&mut self) {
        // Listener admission is available only for explicitly session-aware code.
        let max_kind = if self.hosted_session { 2 } else { 1 };
        let valid_kind = self
            .f
            .value(format!("icmp ule i64 %refill_kind, {max_kind}"));
        let valid_fd = self
            .f
            .value(format!("icmp ule i64 %refill_fd, {MAX_NATIVE}"));
        let valid = self.f.value(format!("and i1 {valid_kind}, {valid_fd}"));
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
    }

    /// Emit a checked `refill` using the admitted descriptor and existing slot.
    ///
    /// Captures the field then evaluates its limit once. Both failure guards
    /// trap without publishing a view. On success, mutates only the adapter's
    /// destination range and emits one of two lexical outcome bodies.
    pub(super) fn refill(&mut self, refill: &Refill) {
        let Input::Bytes { contents, len } =
            self.capture_byte_field(&refill.reference, &refill.path)
        else {
            unreachable!("checked refill destination is a byte field");
        };
        let limit = self.expression(&refill.limit);
        let positive = self.f.value(format!("icmp ne i64 {limit}, 0"));
        let fits_slot = self.f.value(format!("icmp ule i64 {limit}, {len}"));
        let fits_native = self.f.value(format!("icmp ule i64 {limit}, {MAX_NATIVE}"));
        let fits = self.f.value(format!("and i1 {fits_slot}, {fits_native}"));
        let allowed = self.f.value(format!("and i1 {positive}, {fits}"));
        self.f.line(format!("call void @holo.assert(i1 {allowed})"));
        let kind = self.io_kind();
        let fd = self.io_descriptor();
        let result = self.f.value(format!(
            "call i64 @holo_refill_experimental(i64 {kind}, i64 {fd}, ptr {contents}, i64 {len}, i64 {limit})"
        ));
        // Reserved statuses and errors have the sign bit set. A positive count
        // must also fit the validated request before it can become a byte extent.
        let has_data = self.f.value(format!("icmp sgt i64 {result}, 0"));
        let data = self.f.fresh_label();
        let other = self.f.fresh_label();
        let done = self.f.fresh_label();
        self.f
            .line(format!("br i1 {has_data}, label %{data}, label %{other}"));
        self.f.label(&data);
        let fits = self.f.value(format!("icmp ule i64 {result}, {limit}"));
        self.f.line(format!("call void @holo.assert(i1 {fits})"));
        self.push_scope();
        self.bind(
            &refill.data_name,
            Binding::Received {
                contents,
                len: result.clone(),
            },
        );
        self.operations(&refill.data_body);
        self.pop_scope();
        self.f.line(format!("br label %{done}"));
        self.f.label(&other);
        self.guard_refill_status(&result, true);
        self.push_scope();
        let status = self.f.slot("i64");
        self.f.line(format!("store i64 {result}, ptr {status}"));
        self.bind(&refill.result_name, Binding::Scalar(status));
        self.operations(&refill.other_body);
        self.pop_scope();
        self.f.line(format!("br label %{done}"));
        self.f.label(&done);
    }

    /// Emit a single wait and bind its validated scalar result without retrying.
    /// Waiting does not access a byte field and retains no native state.
    pub(super) fn wait_input(&mut self, name: &str) {
        let fd = self.io_descriptor();
        let result = self
            .f
            .value(format!("call i64 @holo_wait_input_experimental(i64 {fd})"));
        self.guard_refill_status(&result, false);
        let status = self.f.slot("i64");
        self.f.line(format!("store i64 {result}, ptr {status}"));
        self.bind(name, Binding::Scalar(status));
    }

    /// Validate nondata `result` under the adapter ABI.
    ///
    /// Accept EOF, interruption, stream would-block, or the high error tag
    /// with exactly one positive signed-int errno. Any other bit pattern traps
    /// before the nondata body can observe a status. This emits scalar checks only.
    fn guard_refill_status(&mut self, result: &str, allow_stream_would_block: bool) {
        let valid = self.valid_io_status(result, true, allow_stream_would_block);
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
    }

    /// Emit the scalar validity predicate shared by native descriptor calls.
    /// The caller selects which zero/would-block outcomes belong to its ABI.
    pub(super) fn valid_io_status(
        &mut self,
        result: &str,
        allow_zero: bool,
        allow_stream_would_block: bool,
    ) -> String {
        // Reads and input waits retain their existing zero and signal checks.
        let end = allow_zero.then(|| self.f.value(format!("icmp eq i64 {result}, 0")));
        let interrupted = self.f.value(format!("icmp eq i64 {result}, -1"));
        let mut status = match end {
            Some(end) => self.f.value(format!("or i1 {end}, {interrupted}")),
            None => interrupted,
        };
        // The adapter reports EAGAIN as WouldBlock only on the admitted stream.
        if allow_stream_would_block {
            let kind = self.io_kind();
            let stream = self.f.value(format!("icmp eq i64 {kind}, 1"));
            let would_block = self.f.value(format!("icmp eq i64 {result}, -2"));
            let would_block = self.f.value(format!("and i1 {stream}, {would_block}"));
            status = self.f.value(format!("or i1 {status}, {would_block}"));
        }
        let error = self.valid_native_errno(result);
        self.f.value(format!("or i1 {status}, {error}"))
    }

    /// Verify that a status consists exactly of the native errno tag and a
    /// positive signed-int payload. The check uses SSA scalars only.
    pub(super) fn valid_native_errno(&mut self, result: &str) -> String {
        let tag = self.f.value(format!("and i64 {result}, -2147483648"));
        let error_tag = self.f.value(format!("icmp eq i64 {tag}, {}", i64::MIN));
        let payload = self.f.value(format!("and i64 {result}, {MAX_NATIVE}"));
        let has_errno = self.f.value(format!("icmp ne i64 {payload}, 0"));
        self.f.value(format!("and i1 {error_tag}, {has_errno}"))
    }
}
