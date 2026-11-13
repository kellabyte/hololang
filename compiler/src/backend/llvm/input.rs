//! Typed producer argument capture and synchronous readonly byte descriptors.
//!
//! Checked scalar actuals evaluate once. Byte actuals capture the location and
//! frozen extent of an initialized field or a fresh input view. Descriptors are
//! ordinary native operands. The checker establishes live roots and destination
//! effects, and this module emits no alias promises.

use crate::ir::{Argument, BytesSource};

use super::{
    context::{Binding, Lowering},
    storage,
};

/// Native operands for one checked formal, retaining source parameter order.
#[derive(Clone)]
pub(super) enum Input {
    /// One captured unsigned scalar SSA value or argument name.
    Scalar(String),
    /// Readonly payload extent valid within its synchronous frame.
    Bytes {
        /// Address of the first payload byte, possibly an undereferenced empty boundary.
        contents: String,
        /// Captured full-width extent, potentially narrowed by a checked prefix.
        len: String,
    },
    /// Borrowed fixed-record address in an ordinary helper's typed input frame.
    State(String),
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
                Input::State(_) => unreachable!("checked producers cannot borrow mutable state"),
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

    /// Capture a complete initialized field or forward an existing logical extent.
    /// Field metadata is loaded once, and the capture performs no payload access.
    pub(super) fn capture_bytes(&mut self, source: &BytesSource) -> Input {
        // Forwarding keeps the already checked bound even when the caller's
        // original field contained more initialized bytes.
        match source {
            BytesSource::Param(index) => self.params[*index].clone(),
            BytesSource::Field { reference, path } => self.capture_byte_field(reference, path),
        }
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
            Input::Scalar(_) | Input::State(_) => {
                panic!("checked byte formal has wrong lowering kind")
            }
        }
    }
}
