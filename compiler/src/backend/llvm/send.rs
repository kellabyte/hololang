//! Checked lowering for one synchronous send or explicit writable wait.
//!
//! Each attempt borrows the admitted descriptor and a live byte source.
//! The emitted guards establish the exact byte subrange and validate the scalar
//! result before exposure. No payload copy or native owner is introduced.

use crate::ir::{BytesSource, Expr};

use super::{
    context::{Binding, Lowering},
    input::Input,
    refill::MAX_NATIVE,
};

/// Optional native output ABI, staged only for checked users of this capability.
pub(super) const DECLARATIONS: &str = "declare i64 @holo_send_experimental(i64, i64, ptr, i64, i64)\n\
     declare i64 @holo_wait_output_experimental(i64, i64)\n\n";

impl Lowering<'_> {
    /// Emit one direct send from a checked byte source's selected subrange.
    ///
    /// Captures metadata before evaluating offset and count in written order.
    /// Invalid ranges or impossible adapter results trap, with no status
    /// binding visible. The adapter borrows the bytes for the duration of send.
    pub(super) fn send(&mut self, source: &BytesSource, offset: &Expr, count: &Expr, name: &str) {
        let Input::Bytes { contents, len } = self.capture_bytes(source) else {
            unreachable!("checked send source has a byte descriptor");
        };
        let offset = self.expression(offset);
        let count = self.expression(count);
        // Subtraction without an LLVM no-wrap promise is defined even when
        // offset is invalid. Only the joint guard permits pointer formation.
        let offset_fits = self.f.value(format!("icmp ule i64 {offset}, {len}"));
        let remaining = self.f.value(format!("sub i64 {len}, {offset}"));
        let positive = self.f.value(format!("icmp ne i64 {count}, 0"));
        let fits_source = self.f.value(format!("icmp ule i64 {count}, {remaining}"));
        let fits_native = self.f.value(format!("icmp ule i64 {count}, {MAX_NATIVE}"));
        let valid_range = self.f.value(format!("and i1 {offset_fits}, {fits_source}"));
        let valid_count = self.f.value(format!("and i1 {positive}, {fits_native}"));
        let allowed = self.f.value(format!("and i1 {valid_range}, {valid_count}"));
        self.f.line(format!("call void @holo.assert(i1 {allowed})"));
        let start = self
            .f
            .value(format!("getelementptr i8, ptr {contents}, i64 {offset}"));
        let kind = self.io_kind();
        let fd = self.io_descriptor();
        let result = self.f.value(format!(
            "call i64 @holo_send_experimental(i64 {kind}, i64 {fd}, ptr {start}, i64 {remaining}, i64 {count})"
        ));
        // A stream can report a short send, but neither zero nor a positive
        // result for a file is an output outcome of the native adapter.
        let stream = self.f.value(format!("icmp eq i64 {kind}, 1"));
        let sent = self.f.value(format!("icmp sgt i64 {result}, 0"));
        let within_request = self.f.value(format!("icmp ule i64 {result}, {count}"));
        let valid_count = self.f.value(format!("and i1 {sent}, {within_request}"));
        let valid_progress = self.f.value(format!("and i1 {stream}, {valid_count}"));
        let valid_status = self.valid_io_status(&result, false, true);
        let valid = self
            .f
            .value(format!("or i1 {valid_progress}, {valid_status}"));
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
        self.bind(name, Binding::ScalarValue(result));
    }

    /// Emit one explicit writable wait and bind a checked immutable status.
    /// This can block in poll, and acquires no storage or descriptor ownership.
    pub(super) fn wait_output(&mut self, name: &str) {
        let kind = self.io_kind();
        let fd = self.io_descriptor();
        let result = self.f.value(format!(
            "call i64 @holo_wait_output_experimental(i64 {kind}, i64 {fd})"
        ));
        // Zero is ready only for the admitted stream. File mode returns errno.
        let stream = self.f.value(format!("icmp eq i64 {kind}, 1"));
        let zero = self.f.value(format!("icmp eq i64 {result}, 0"));
        let ready = self.f.value(format!("and i1 {stream}, {zero}"));
        let status = self.valid_io_status(&result, false, false);
        let valid = self.f.value(format!("or i1 {ready}, {status}"));
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
        self.bind(name, Binding::ScalarValue(result));
    }
}
