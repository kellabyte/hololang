//! LLVM boundary for one checked lexical TCP descriptor.
//!
//! Native admission transfers the descriptor as an entry operand. A direct
//! explicit close removes the cleanup at compile time. Early exits observe
//! the active lexical cleanup without mutating it for other emitted paths.
//! This profile emits no owner slot, handle table, or runtime liveness check.

use crate::ir::Op;

use super::{
    context::{Binding, Cleanup, Lowering},
    refill::MAX_NATIVE,
};

/// Native close and readiness adapters staged only for the dedicated TCP entry.
pub(super) const DECLARATIONS: &str = "declare i64 @holo_tcp_close_experimental(i64)\n\
     declare i64 @holo_tcp_wait_experimental(i64, i64, i64)\n";

impl Lowering<'_> {
    /// Run the body while it owns the descriptor already admitted by its host.
    /// Scope exit releases a still-live owner and keeps the ordinary return
    /// status. The checker admits only one outermost owner and restricts
    /// explicit close to its immediate sequence.
    pub(super) fn tcp_scope(&mut self, body: &[Op]) {
        let stream = self.f.value("icmp eq i64 %refill_kind, 1");
        let native_fd = self
            .f
            .value(format!("icmp ule i64 %refill_fd, {MAX_NATIVE}"));
        // The top two unsigned words select direct poll or the concurrent
        // host broker. Every other selector must be a native queue fd.
        let mode = self.f.value("icmp uge i64 %tcp_queue, -2");
        let native_queue = self
            .f
            .value(format!("icmp ule i64 %tcp_queue, {MAX_NATIVE}"));
        let valid_queue = self.f.value(format!("or i1 {mode}, {native_queue}"));
        let valid_stream = self.f.value(format!("and i1 {stream}, {native_fd}"));
        let valid = self
            .f
            .value(format!("and i1 {valid_stream}, {valid_queue}"));
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
        self.push_scope();
        self.cleanups.push(Cleanup::Tcp);
        self.operations(body);
        // Only a direct explicit close removes this cleanup before fallthrough.
        if matches!(self.cleanups.last(), Some(Cleanup::Tcp)) {
            self.cleanups.pop();
            self.release_tcp();
        }
        self.pop_scope();
    }

    /// Consume the live descriptor before issuing its terminal native close.
    /// Bind a checked immutable status, including tagged EINTR on failure,
    /// without granting any further operation access to the old descriptor.
    pub(super) fn close_tcp(&mut self, name: &str) {
        assert!(
            matches!(self.cleanups.pop(), Some(Cleanup::Tcp)),
            "checked explicit TCP close must consume the active outer owner"
        );
        let result = self
            .f
            .value("call i64 @holo_tcp_close_experimental(i64 %refill_fd)");
        let success = self.f.value(format!("icmp eq i64 {result}, 0"));
        let terminal_error = self.valid_native_errno(&result);
        let valid = self.f.value(format!("or i1 {success}, {terminal_error}"));
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
        self.bind(name, Binding::ScalarValue(result));
    }

    /// Wait once through the host's selected readiness backend.
    /// Both directions share the same validated immutable status. Readiness
    /// lends no payload, retains no region storage, and can block explicitly.
    pub(super) fn wait_tcp(&mut self, name: &str, writing: bool) {
        let direction = u8::from(writing);
        let result = self.f.value(format!(
            "call i64 @holo_tcp_wait_experimental(i64 %refill_fd, i64 %tcp_queue, i64 {direction})"
        ));
        let valid = self.valid_io_status(&result, true, false);
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
        self.bind(name, Binding::ScalarValue(result));
    }

    /// Issue the normal-exit close without changing the selected return value.
    /// The native adapter attempts terminal release once. Its status is unused.
    pub(super) fn release_tcp(&mut self) {
        self.f
            .line("call i64 @holo_tcp_close_experimental(i64 %refill_fd)");
    }
}
