//! Typed producer argument capture and synchronous byte descriptors.
//!
//! Checked scalar actuals evaluate once. Byte actuals capture the location and
//! frozen extent of an initialized field or a fresh input view. Descriptors are
//! ordinary native operands. The checker establishes live roots and destination
//! effects, and this module emits no alias promises.

use crate::ir::{Argument, ByteRange, BytesSource};

use super::{
    context::{BatchPrefix, Binding, Lowering},
    storage,
};

/// Native operands for one checked formal, retaining source parameter order.
#[derive(Clone)]
pub(super) enum Input {
    /// One captured unsigned scalar SSA value or argument name.
    Scalar(String),
    /// One copied boolean formal in its native i1 representation.
    Boolean(String),
    /// Captured payload extent valid within its synchronous frame.
    Bytes {
        /// Address of the first payload byte, possibly an undereferenced empty boundary.
        contents: String,
        /// Captured full-width extent, potentially narrowed by a checked view.
        len: String,
    },
    /// Borrowed fixed-record address in an ordinary helper's typed input frame.
    State(String),
    /// Address, byte extent, and record count of an ordinary borrowed batch.
    Batch(BatchPrefix),
}

impl Lowering<'_> {
    /// Capture all typed `args` once and return comma-prefixed native call operands.
    /// Evaluation preserves written order. Field projections load shape metadata,
    /// not payload bytes; host strings allocate only compiler memory. Missing kinds
    /// are internal failures because this path accepts only checked programs.
    pub(super) fn call_arguments(&mut self, args: &[Argument]) -> String {
        let mut result = String::new();
        // Capture each argument fully before beginning the next, including unused formals.
        for argument in args {
            let input = match argument {
                Argument::Scalar(value) => Input::Scalar(self.expression(value)),
                Argument::Bytes(source) => self.capture_bytes(source),
                Argument::ByteView { source, ranges } => self.capture_byte_view(source, ranges),
                Argument::Received(name) => {
                    let (contents, len) = self.received_bytes(name);
                    Input::Bytes { contents, len }
                }
            };
            // Flatten each typed formal only at the native ABI boundary; no runtime tag exists.
            match input {
                Input::Scalar(value) => result.push_str(&format!(", i64 {value}")),
                Input::Bytes { contents, len } => {
                    result.push_str(&format!(", ptr {contents}, i64 {len}"));
                }
                Input::Boolean(_) | Input::State(_) | Input::Batch(_) => {
                    unreachable!("checked producers accept only unsigned and byte inputs")
                }
            }
        }
        result
    }

    /// Lend the original SSA pointer and count of the active refill continuation.
    /// The checker established the lexical capability and the refill already
    /// checked the count. Capturing the operands adds no native loads or checks.
    pub(super) fn received_bytes(&self, name: &str) -> (String, String) {
        // Every accepted received actual must resolve to the original view.
        let Binding::Received { contents, len } = self.binding(name) else {
            unreachable!("checked received argument has a live view");
        };
        (contents, len)
    }

    /// Reuse the captured pointer and extent of a checked lexical byte view.
    pub(super) fn local_bytes(&self, name: &str) -> (String, String) {
        let Binding::ByteView { contents, len } = self.binding(name) else {
            unreachable!("checked local byte argument has a live view");
        };
        (contents, len)
    }

    /// Capture a complete initialized field or forward an existing logical extent.
    /// Field metadata is loaded once, and the capture performs no payload access.
    pub(super) fn capture_bytes(&mut self, source: &BytesSource) -> Input {
        // Forwarding keeps the already checked bound even when the caller's
        // original field contained more initialized bytes.
        match source {
            BytesSource::Literal(bytes) => {
                let (contents, len) = self.byte_literals.capture(bytes);
                Input::Bytes { contents, len }
            }
            BytesSource::Param(index) => self.params[*index].clone(),
            BytesSource::Field { reference, path } => self.capture_byte_field(reference, path),
            BytesSource::Received(name) => {
                let (contents, len) = self.received_bytes(name);
                Input::Bytes { contents, len }
            }
            BytesSource::Local(name) => {
                let (contents, len) = self.local_bytes(name);
                Input::Bytes { contents, len }
            }
        }
    }

    /// Apply each view to its captured input before evaluating the next window.
    /// Subtraction-based guards avoid overflowing `offset + length`.
    pub(super) fn capture_byte_view(
        &mut self,
        source: &BytesSource,
        ranges: &[ByteRange],
    ) -> Input {
        let Input::Bytes {
            mut contents,
            mut len,
        } = self.capture_bytes(source)
        else {
            unreachable!("checked byte view has a byte source");
        };
        for range in ranges {
            let offset = range.offset.as_ref().map(|offset| self.expression(offset));
            let requested = self.expression(&range.length);
            if let Some(offset) = offset {
                let starts_in_range = self.f.value(format!("icmp ule i64 {offset}, {len}"));
                let remaining = self.f.value(format!("sub i64 {len}, {offset}"));
                let ends_in_range = self
                    .f
                    .value(format!("icmp ule i64 {requested}, {remaining}"));
                let valid = self
                    .f
                    .value(format!("and i1 {starts_in_range}, {ends_in_range}"));
                self.f.line(format!("call void @holo.assert(i1 {valid})"));
                contents = storage::advance(&mut self.f, &contents, &offset);
            } else {
                let valid = self.f.value(format!("icmp ule i64 {requested}, {len}"));
                self.f.line(format!("call void @holo.assert(i1 {valid})"));
            }
            len = requested;
        }
        Input::Bytes { contents, len }
    }

    /// Capture the payload pointer and frozen physical extent of a checked field.
    /// The caller has already established root and permissions. A proved cursor
    /// extent can replace the metadata load. No payload is read.
    pub(super) fn capture_byte_field(&mut self, reference: &str, path: &[String]) -> Input {
        let header = self.field_pointer(reference, path);
        let len = match self.known_byte_extent(reference) {
            Some(extent) => extent.to_string(),
            None => self.byte_length(&header),
        };
        let width = self.layout.length_bytes().to_string();
        let contents = storage::advance(&mut self.f, &header, &width);
        Input::Bytes { contents, len }
    }

    /// Return a byte formal's captured location/extent without accessing payload memory.
    /// `param` is type-checked in the full formal list. Cloning operands affects
    /// compiler strings only; a wrong kind indicates a compiler invariant failure.
    pub(super) fn byte_input(&self, param: usize) -> (String, String) {
        // Only Bytes formals can be observed with InputLen/InputByte expressions.
        match &self.params[param] {
            Input::Bytes { contents, len } => (contents.clone(), len.clone()),
            Input::Scalar(_) | Input::Boolean(_) | Input::State(_) | Input::Batch(_) => {
                panic!("checked byte formal has wrong lowering kind")
            }
        }
    }
}
