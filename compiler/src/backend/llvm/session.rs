//! Optional host-managed loopback session lowering.
//!
//! Only a checked session-aware entry resolves its current client through the
//! host. Region cleanup, application state, and existing I/O range checks stay
//! in the ordinary lowering. A host session transition introduces no byte copy.

use super::context::{Binding, Lowering};

/// Internal ABI used only by checked entries with next-session operations.
pub(super) const DECLARATIONS: &str = "declare i64 @holo_session_fd_experimental(i64, i64)\n\
     declare i64 @holo_next_session_experimental(i64, i64)\n\n";

impl Lowering<'_> {
    /// Normalize listener mode to the connected byte-stream adapter kind.
    /// The entry guard has already established the original kind range.
    pub(super) fn bind_session_io_kind(&mut self) {
        self.f.line("%io.stream = icmp ne i64 %refill_kind, 0");
        self.f.line("%io.kind = zext i1 %io.stream to i64");
    }

    /// Select the native ABI kind operand for file or connected stream.
    /// Ordinary profiles keep their original entry operand.
    pub(super) fn io_kind(&self) -> &'static str {
        // Only session-aware entries can receive the listener's third kind.
        if self.hosted_session {
            "%io.kind"
        } else {
            "%refill_kind"
        }
    }

    /// Return the descriptor for the synchronous attempt about to execute.
    /// The optional call observes the current session without lending it to Holo.
    pub(super) fn io_descriptor(&mut self) -> String {
        // Retain the original operand and avoid a runtime call outside this profile.
        if self.hosted_session {
            self.f
                .value("call i64 @holo_session_fd_experimental(i64 %refill_kind, i64 %refill_fd)")
        } else {
            "%refill_fd".to_owned()
        }
    }

    /// Emit one explicit transition and validate its immutable scalar status.
    /// Listener mode may accept, be interrupted, or return a tagged errno.
    /// The one-descriptor profiles return the distinct terminal status.
    pub(super) fn next_session(&mut self, name: &str) {
        let result = self
            .f
            .value("call i64 @holo_next_session_experimental(i64 %refill_kind, i64 %refill_fd)");
        let listener = self.f.value("icmp eq i64 %refill_kind, 2");
        let accepted = self.f.value(format!("icmp eq i64 {result}, 0"));
        let error = self.valid_io_status(&result, false, false);
        let allowed = self.f.value(format!("or i1 {accepted}, {error}"));
        let end = self.f.value(format!("icmp eq i64 {result}, 1"));
        let valid = self
            .f
            .value(format!("select i1 {listener}, i1 {allowed}, i1 {end}"));
        self.f.line(format!("call void @holo.assert(i1 {valid})"));
        self.bind(name, Binding::ScalarValue(result));
    }
}
