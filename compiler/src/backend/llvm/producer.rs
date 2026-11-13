//! Final-destination constructor lowering and recoverable failure propagation.
//!
//! Every producer receives its eventual address and initializes only that
//! unpublished suffix. Eligible typed subtrees check advances through private
//! operands and leave the region marker to append. Numeric producers and the
//! Wide control reserve through the region marker. Failures use ordinary calls.

use crate::ir::{Argument, Build, Expr, Parameter, Producer};
use crate::layout::{LayoutMode, LayoutPlan};

use super::{
    context::{Lowering, Module},
    copy,
    function::Function,
    input::Input,
    producer_inline::Inline,
    producer_lifts, storage,
};

/// Error encoding requested for one private constructor body.
#[derive(Clone, Copy)]
struct RejectionEncoding<'a> {
    /// Source-level nominal family for case and child lookup.
    family: Option<&'a str>,
    /// Checked shift into the caller's receiving range.
    lift: u8,
}

/// Return convention selected from the checked producer's signature.
///
/// The legacy scalar status remains unchanged. The typed pair carries status
/// and the active scalar error payload outside the unpublished suffix.
#[derive(Clone, Copy)]
pub(super) enum ProducerReturn {
    /// Numeric append's original i64 status.
    Numeric,
    /// A status and an active error payload or successful cursor in two i64 lanes.
    Typed,
}

impl ProducerReturn {
    /// Read the checked signature to choose the internal LLVM result type.
    pub(super) fn for_producer(producer: &Producer) -> Self {
        // Only a checked nominal family changes the producer call ABI.
        if producer.error.is_some() {
            Self::Typed
        } else {
            Self::Numeric
        }
    }

    /// Return the LLVM type spelling used by declarations and every caller.
    pub(super) fn llvm_type(self) -> &'static str {
        match self {
            Self::Numeric => "i64",
            Self::Typed => "{ i64, i64 }",
        }
    }

    /// Obtain the status lane of an already evaluated native call result.
    pub(super) fn status(self, function: &mut Function, value: &str) -> String {
        match self {
            Self::Numeric => value.to_owned(),
            Self::Typed => function.value(format!("extractvalue {} {value}, 0", self.llvm_type())),
        }
    }

    /// Return a statically known status with no active payload.
    pub(super) fn return_status(self, function: &mut Function, status: u64) {
        match self {
            Self::Numeric => function.line(format!("ret i64 {status}")),
            Self::Typed => function.line(format!(
                "ret {} {{ i64 {status}, i64 0 }}",
                self.llvm_type()
            )),
        }
    }
}

/// Select the experimental layout hint only for a checked scalar-input producer.
///
/// A scalar-input producer has no caller-borrowed bytes. It can observe static
/// literals and lend them to child producers. The hint is an optimization
/// preference, with no promise about purity, uniqueness, or failure frequency.
/// Unused Bytes formals remain excluded conservatively.
pub(super) fn prefers_success(producer: &Producer) -> bool {
    // Exhaustive kinds keep any future input category outside the known scalar
    // case until the optimization's scope and checked forwarding proof are reviewed.
    producer.params.iter().all(|parameter| match parameter {
        Parameter::U64 => true,
        Parameter::Bytes => false,
    })
}

/// Return the exact extent eligible for fixed cursor arithmetic and forwarding.
/// A checked typed, headerless, fixed schema also has only fixed children.
/// Variable extents use a separate success encoding.
pub(super) fn fixed_size(producer: &Producer, layout: &LayoutPlan) -> Option<u64> {
    let representation = layout.record(&producer.record);
    // Eligibility uses the planned extent and retained header, not a source hint.
    if producer.error.is_some() && representation.header_bytes == 0 {
        representation.fixed_size
    } else {
        None
    }
}

/// Private storage convention of a checked destination producer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Placement {
    /// Preserve the marker-based ABI for numeric calls and the Wide control.
    Marked,
    /// Headerless typed result whose successful extent is statically known.
    Fixed(u64),
    /// Byte-bearing typed result that returns its checked end on success.
    Deferred,
}

/// Select the same private storage ABI at every call and definition.
///
/// Checked nested producers share the typed/numeric convention. Derived and
/// compact typed subtrees therefore contain only fixed and deferred children.
pub(super) fn placement(producer: &Producer, layout: &LayoutPlan) -> Placement {
    if let Some(bytes) = fixed_size(producer, layout) {
        Placement::Fixed(bytes)
    } else if producer.error.is_some() && layout.mode() != LayoutMode::Wide {
        Placement::Deferred
    } else {
        Placement::Marked
    }
}

/// Compiler state for ordered reservations in one producer body.
enum Reservations {
    /// Read and advance the existing region marker after each reserve.
    Marked,
    /// Keep a fixed cursor and static field offsets.
    Fixed(storage::FixedReservations),
    /// Keep a variable cursor and receiving pointer in SSA.
    Deferred(storage::DeferredReservations),
}

/// Emit one checked producer as an internal native function.
///
/// The module supplies validated definitions and `index` identifies the producer.
/// The result is owned LLVM text. Scalars use u64 and byte borrows use ptr/u64, while
/// the hidden destination pointer names the final receiving slot through calls.
/// Layout changes compiler-owned metadata/limits without adding aggregate copies.
/// `target` selects the backend's short-copy policy for byte initialization.
/// `inline` is the inlining preference for the private definition.
/// `lifts` lists bounded terminal variants and `lift` selects the current encoding.
pub(super) fn emit(
    module: &Module<'_>,
    index: usize,
    target: &str,
    inline: Inline,
    lifts: &producer_lifts::ProducerLifts,
    lift: u8,
) -> String {
    let program = module.program;
    let producer = &program.producers[index];
    let layout = module.layout;
    let returned = ProducerReturn::for_producer(producer);
    let placement = placement(producer, layout);
    let mut params = Vec::with_capacity(producer.params.len());
    let mut signature = format!(
        "define internal {} {}(",
        returned.llvm_type(),
        producer_lifts::symbol(index, lift),
    );
    // Checked fixed producers inherit the caller's storage operands even when
    // outlined. No alias or dereferenceability promise is needed for reuse.
    if placement != Placement::Marked {
        signature.push_str("ptr %destination, i64 %initial.top, i64 %capacity");
    } else {
        signature.push_str("ptr %region, ptr %destination");
    }
    // The source parameters retain their native lowering. Borrows add ordinary
    // operands without a descriptor allocation, closure, or noalias attribute.
    for (index, kind) in producer.params.iter().enumerate() {
        match kind {
            Parameter::U64 => {
                let value = format!("%arg{index}");
                signature.push_str(&format!(", i64 {value}"));
                params.push(Input::Scalar(value));
            }
            Parameter::Bytes => {
                let contents = format!("%arg{index}.data");
                let len = format!("%arg{index}.len");
                signature.push_str(&format!(", ptr {contents}, i64 {len}"));
                params.push(Input::Bytes { contents, len });
            }
        }
    }
    signature.push(')');
    signature.push_str(inline.attribute());
    let mut lowering = Lowering::new(module, signature, params);
    lowering.construction_hints = prefers_success(producer);
    // A whole-result forward delegates even the leading header or empty unit.
    // Reserving either here would move the child's cursor past its destination.
    if let [Build::Forward { producer, args }] = producer.body.as_slice() {
        lowering.forward_result(producer, args, placement, returned, lift);
        return lowering.f.finish();
    }
    // Marker-based producers retain their original bookkeeping. Deferred
    // typed producers already receive the checked starting cursor.
    let start = if placement != Placement::Marked {
        "%initial.top".to_owned()
    } else {
        storage::top(&mut lowering.f, "%region")
    };
    let representation = layout.record(&producer.record);
    let mut reservations = match placement {
        Placement::Marked => Reservations::Marked,
        Placement::Fixed(_) => Reservations::Fixed(storage::FixedReservations::new(
            start.clone(),
            "%capacity".to_owned(),
        )),
        Placement::Deferred => Reservations::Deferred(storage::DeferredReservations::new()),
    };
    // Retained headers keep their original first reservation. Headerless empty
    // records initialize one unit byte so every published record advances a cursor.
    if representation.header_bytes != 0 {
        lowering.reserve_word_or_unit(representation.header_bytes, returned, &mut reservations);
    } else if representation.unit_bytes != 0 {
        let unit =
            lowering.reserve_word_or_unit(representation.unit_bytes, returned, &mut reservations);
        lowering.f.line(format!("store i8 0, ptr {unit}, align 1"));
    }
    // A final fixed child completes the subtree. Forward its pair when a
    // compatible definition can encode named rejections in the required range.
    let forward_fixed_child = lifts.forwards_tail(index, lift);
    // A headerless terminal variable child already returns the enclosing end.
    // Its tag encoding must agree before the entire result can flow through.
    let forward_deferred_child = placement == Placement::Deferred
        && representation.header_bytes == 0
        && matches!(producer.body.last(), Some(Build::Record { producer: child, .. })
        if {
            let child = &program.producers[lowering.producer_index(child)];
            self::placement(child, layout) == Placement::Deferred
                && child.error == producer.error
        });
    let forward_final_child = forward_fixed_child || forward_deferred_child;
    let encoding = RejectionEncoding {
        family: producer.error.as_deref(),
        lift,
    };
    // Ordered field initializers are already checked for exactly-once completion.
    for (index, operation) in producer.body.iter().enumerate() {
        lowering.build(
            operation,
            target,
            returned,
            encoding,
            &mut reservations,
            forward_final_child && index + 1 == producer.body.len(),
        );
    }
    // The child's pair is already the full outcome of this fixed parent.
    // Suppress the usual success continuation after forwarding it.
    if forward_final_child {
        return lowering.f.finish();
    }
    // Only schemas whose plan retains a size header materialize the total size.
    // Headerless records derive their frozen extent from their typed shape metadata.
    if representation.header_bytes != 0 {
        let end = match &reservations {
            Reservations::Deferred(state) => state.cursor.clone(),
            // A retained header is only possible in marked or deferred mode.
            _ => storage::top(&mut lowering.f, "%region"),
        };
        let size = lowering.f.value(format!("sub i64 {end}, {start}"));
        storage::store_u64(&mut lowering.f, "%destination", &size);
    }
    // A dynamic caller uses the success lane to commit its checked end. All
    // other status paths keep the pair and numeric returns as before.
    if let Reservations::Deferred(state) = reservations {
        let outcome = lowering.f.value(format!(
            "insertvalue {{ i64, i64 }} zeroinitializer, i64 {}, 1",
            state.cursor
        ));
        lowering.f.line(format!("ret {{ i64, i64 }} {outcome}"));
    } else {
        returned.return_status(&mut lowering.f, 0);
    }
    lowering.f.finish()
}

impl Lowering<'_> {
    /// Delegate one whole result through the original storage ABI.
    /// The checked callee owns initialization and returns the exact success
    /// cursor or failure. A lifted wrapper variant remaps only named statuses.
    fn forward_result(
        &mut self,
        callee: &str,
        args: &[Argument],
        placement: Placement,
        returned: ProducerReturn,
        lift: u8,
    ) {
        let callee = self.producer_index(callee);
        let destination = if placement == Placement::Marked {
            "ptr %region, ptr %destination"
        } else {
            "ptr %destination, i64 %initial.top, i64 %capacity"
        };
        // The destination has already been resolved by the receiving call.
        // Capture scalar and borrowed actuals once in their written order.
        let arguments = self.call_arguments(args);
        let ty = returned.llvm_type();
        let outcome = self.f.value(format!(
            "call {ty} @holo.producer.{callee}({destination}{arguments})"
        ));
        // Most wrappers return the entire pair or status directly. A fixed
        // parent can request a shifted wrapper when it includes this family.
        let adjusted = if lift != 0 {
            let status = returned.status(&mut self.f, &outcome);
            self.lift_rejection(returned, &outcome, &status, lift)
        } else {
            None
        };
        let value = adjusted.as_deref().unwrap_or(&outcome);
        self.f.line(format!("ret {ty} {value}"));
    }

    /// Apply an admitted error-family shift to the declared status range.
    /// Success, size/capacity statuses, and the payload lane remain exact.
    fn lift_rejection(
        &mut self,
        returned: ProducerReturn,
        outcome: &str,
        status: &str,
        offset: u8,
    ) -> Option<String> {
        // With no shift, retaining the original outcome also retains its ABI.
        (offset != 0).then(|| {
            let declared = self.f.value(format!("icmp uge i64 {status}, 4"));
            let lifted = self.f.value(format!("add i64 {status}, {offset}"));
            let mapped = self
                .f
                .value(format!("select i1 {declared}, i64 {lifted}, i64 {status}"));
            self.f.value(format!(
                "insertvalue {} {outcome}, i64 {mapped}, 0",
                returned.llvm_type()
            ))
        })
    }

    /// Reserve a scalar word or the initialized unit of an empty record.
    /// The fixed path uses the caller's destination only after the same checks.
    /// A successful reserve advances the selected cursor or the private marker.
    fn reserve_word_or_unit(
        &mut self,
        bytes: u64,
        returned: ProducerReturn,
        reservations: &mut Reservations,
    ) -> String {
        match reservations {
            Reservations::Fixed(state) => state.reserve(
                &mut self.f,
                "%destination",
                bytes,
                self.construction_hints,
                returned,
            ),
            Reservations::Deferred(state) => state.reserve(
                &mut self.f,
                &bytes.to_string(),
                self.construction_hints,
                returned,
            ),
            Reservations::Marked => storage::reserve(
                &mut self.f,
                "%region",
                &bytes.to_string(),
                self.construction_hints,
                returned,
            ),
        }
    }

    /// Emit a validated constructor operation in field/evaluation order.
    ///
    /// Only destination writes and pure expressions are expressible here. Any
    /// runtime error returns immediately; catch/resume inside a partial record is
    /// absent from this IR, preserving the append's single publication boundary.
    /// `target` selects a checked lowering policy after the original reservation.
    /// `encoding` supplies the verified case range of this emitted body.
    /// `reservations` carries cursor state along the success path.
    /// `forward_child` returns an eligible terminal child's complete pair.
    fn build(
        &mut self,
        operation: &Build,
        target: &str,
        returned: ProducerReturn,
        encoding: RejectionEncoding<'_>,
        reservations: &mut Reservations,
        forward_child: bool,
    ) {
        // Each constructor form either initializes its field or returns a status.
        match operation {
            Build::U64 { value, .. } => {
                let value = self.expression(value);
                let address = self.reserve_word_or_unit(8, returned, reservations);
                storage::store_u64(&mut self.f, &address, &value);
            }
            Build::Bytes { len, value, .. } => {
                let choice = copy::choose(target, len, value);
                let len = self.expression(len);
                let width = self.layout.length_bytes();
                // Compact is an explicit checked representation, never silent truncation.
                // This guard occurs at the byte field's original evaluation point.
                if width == 4 {
                    let representable = self.f.value(format!("icmp ule i64 {len}, 4294967295"));
                    let ready = self.f.fresh_label();
                    let rejected = self.f.fresh_label();
                    self.f.construction_branch(
                        &representable,
                        &ready,
                        &rejected,
                        self.construction_hints.then_some(true),
                    );
                    self.f.label(&rejected);
                    returned.return_status(&mut self.f, 2);
                    self.f.label(&ready);
                }
                let addition = self.f.value(format!(
                    "call {{ i64, i1 }} @llvm.uadd.with.overflow.i64(i64 {len}, i64 {width})"
                ));
                let size = self
                    .f
                    .value(format!("extractvalue {{ i64, i1 }} {addition}, 0"));
                let overflow = self
                    .f
                    .value(format!("extractvalue {{ i64, i1 }} {addition}, 1"));
                let valid = self.f.fresh_label();
                let invalid = self.f.fresh_label();
                // Header+payload overflow is detected before reserving the field.
                self.f.construction_branch(
                    &overflow,
                    &invalid,
                    &valid,
                    self.construction_hints.then_some(false),
                );
                self.f.label(&invalid);
                returned.return_status(&mut self.f, 2);
                self.f.label(&valid);
                let header = match reservations {
                    Reservations::Deferred(state) => {
                        state.reserve(&mut self.f, &size, self.construction_hints, returned)
                    }
                    // A checked byte field always has variable physical size.
                    Reservations::Marked => storage::reserve(
                        &mut self.f,
                        "%region",
                        &size,
                        self.construction_hints,
                        returned,
                    ),
                    Reservations::Fixed(_) => unreachable!("byte field in fixed schema"),
                };
                // Width affects metadata only; all region sizes and indices remain u64.
                if width == 4 {
                    let narrow = self.f.value(format!("trunc i64 {len} to i32"));
                    self.f
                        .line(format!("store i32 {narrow}, ptr {header}, align 1"));
                } else {
                    storage::store_u64(&mut self.f, &header, &len);
                }
                let contents = storage::advance(&mut self.f, &header, &width.to_string());
                self.f.line(format!(
                    "; Holo byte initialization: {}",
                    choice.explanation()
                ));
                // A validated contiguous source can use guarded inline and bulk
                // transfers. Other initializers need their original expression loop.
                if let copy::Choice::GuardedInline { input, offset } = choice {
                    let (source, extent) = self.byte_input(input);
                    let done = self.f.fresh_label();
                    copy::dispatch(
                        &mut self.f,
                        &contents,
                        &len,
                        &source,
                        &extent,
                        offset,
                        &done,
                    );
                } else {
                    self.initialize_bytes(&contents, &len, value);
                }
            }
            Build::Record { producer, args, .. } => {
                let index = self.producer_index(producer);
                let child = &self.program.producers[index];
                let child_size = fixed_size(child, self.layout);
                // The checked typed families are either identical or a direct
                // child. A numeric pair has no declared cases to translate.
                let offset = child
                    .error
                    .as_deref()
                    .zip(encoding.family)
                    .map_or(0, |(source, target)| self.try_error_offset(source, target));
                let offset = producer_lifts::compose(encoding.lift, offset);
                // Fixed parents derive child operands from their static layout.
                // Variable parents use the current SSA cursor and pointer.
                // Marker-based parents call children in the same convention.
                let (cursor, destination) = match reservations {
                    Reservations::Fixed(state) => (
                        Some(state.cursor(&mut self.f).to_owned()),
                        state.destination(&mut self.f, "%destination"),
                    ),
                    Reservations::Deferred(state) => {
                        (Some(state.cursor.clone()), state.tail.clone())
                    }
                    Reservations::Marked => (None, storage::tail(&mut self.f, "%region")),
                };
                // The checker keeps nested calls in the same return convention.
                // A direct-forward edge uses the already shifted child. A
                // continuing edge retains the generic child and maps failure.
                let callee = producer_lifts::symbol(index, if forward_child { offset } else { 0 });
                let mut call = format!("call {} {callee}(", returned.llvm_type());
                // Preserve typed readonly inputs and written order through nested calls.
                let arguments = self.call_arguments(args);
                // A cursor-based child inherits the initialized capacity
                // captured at append. The marker-based ABI passes the region.
                if let Some(cursor) = cursor.as_deref() {
                    let capacity = match &reservations {
                        Reservations::Fixed(state) => state.capacity().to_owned(),
                        Reservations::Deferred(_) => "%capacity".to_owned(),
                        Reservations::Marked => unreachable!("marker-based call has no cursor"),
                    };
                    call.push_str(&format!("ptr {destination}, i64 {cursor}, i64 {capacity}"));
                } else {
                    call.push_str(&format!("ptr %region, ptr {destination}"));
                }
                call.push_str(&arguments);
                call.push(')');
                let outcome = self.f.value(call);
                // A terminal child's compatible pair lets LLVM select a tail
                // call when the target ABI permits. No header write remains.
                if forward_child {
                    self.f
                        .line(format!("ret {} {outcome}", returned.llvm_type()));
                    return;
                }
                let status = returned.status(&mut self.f, &outcome);
                let success = self.f.value(format!("icmp eq i64 {status}, 0"));
                let ready = self.f.fresh_label();
                let failed = self.f.fresh_label();
                // Nested failure leaves all publication and rollback to append.
                self.f.construction_branch(
                    &success,
                    &ready,
                    &failed,
                    self.construction_hints.then_some(true),
                );
                self.f.label(&failed);
                // Translate only declared rejections. Storage statuses retain
                // their identities and the result's payload lane is untouched.
                let adjusted = self.lift_rejection(returned, &outcome, &status, offset);
                let failure = adjusted.as_deref().unwrap_or(&outcome);
                self.f
                    .line(format!("ret {} {failure}", returned.llvm_type()));
                self.f.label(&ready);
                // A cursor-based parent resumes at the child's checked end.
                // Marker-based children have already advanced the marker.
                match reservations {
                    Reservations::Fixed(state) => state.complete_child(
                        child_size.expect("fixed parent contains only fixed child records"),
                    ),
                    Reservations::Deferred(state) => {
                        state.complete_child(&mut self.f, child_size, &outcome);
                    }
                    Reservations::Marked => {}
                }
            }
            Build::FailIf(condition) => {
                let rejected = self.condition(condition);
                let failed = self.f.fresh_label();
                let ready = self.f.fresh_label();
                // Explicit input rejection is distinct from storage exhaustion.
                self.f
                    .line(format!("br i1 {rejected}, label %{failed}, label %{ready}"));
                self.f.label(&failed);
                returned.return_status(&mut self.f, 3);
                self.f.label(&ready);
            }
            Build::Reject {
                condition,
                case,
                payload,
            } => {
                let rejected = self.condition(condition);
                let failed = self.f.fresh_label();
                let ready = self.f.fresh_label();
                // Only the rejecting edge evaluates an active payload. The
                // result lives in the return lanes even after the owner resets.
                self.f
                    .line(format!("br i1 {rejected}, label %{failed}, label %{ready}"));
                self.f.label(&failed);
                let family = encoding
                    .family
                    .expect("typed rejection has a checked family");
                let status = self.append_error_status(family, case) + u64::from(encoding.lift);
                if let Some(payload) = payload {
                    let payload = self.expression(payload);
                    let pair = self.f.value(format!(
                        "insertvalue {} {{ i64 {status}, i64 0 }}, i64 {payload}, 1",
                        returned.llvm_type()
                    ));
                    self.f.line(format!("ret {} {pair}", returned.llvm_type()));
                } else {
                    returned.return_status(&mut self.f, status);
                }
                self.f.label(&ready);
            }
            Build::Forward { .. } => unreachable!("whole results are delegated before reservation"),
        }
    }

    /// Evaluate `value` in increasing index order into `length` reserved bytes.
    ///
    /// `contents` points at unpublished storage. Each iteration keeps all eager
    /// observations, their traps and low-byte conversion. Zero length performs
    /// no initializer work. One fixed native cursor slot is eligible for SSA
    /// promotion, with no extra payload allocation or temporary.
    pub(super) fn initialize_bytes(&mut self, contents: &str, length: &str, value: &Expr) {
        let index_slot = self.f.slot("i64");
        let condition = self.f.fresh_label();
        let body = self.f.fresh_label();
        let done = self.f.fresh_label();
        self.f.line(format!("store i64 0, ptr {index_slot}"));
        self.f.line(format!("br label %{condition}"));
        self.f.label(&condition);
        let index = self.f.value(format!("load i64, ptr {index_slot}"));
        let remaining = self.f.value(format!("icmp ult i64 {index}, {length}"));
        // The counted initializer visits each reserved payload byte once;
        // length zero skips the body without reading uninitialized data.
        self.f
            .line(format!("br i1 {remaining}, label %{body}, label %{done}"));
        self.f.label(&body);
        self.byte_index = Some(index.clone());
        let element = self.expression(value);
        self.byte_index = None;
        let byte = self.f.value(format!("trunc i64 {element} to i8"));
        let address = storage::advance(&mut self.f, contents, &index);
        self.f
            .line(format!("store i8 {byte}, ptr {address}, align 1"));
        let following = self.f.value(format!("add i64 {index}, 1"));
        self.f
            .line(format!("store i64 {following}, ptr {index_slot}"));
        self.f.line(format!("br label %{condition}"));
        self.f.label(&done);
    }
}

#[cfg(test)]
mod tests {
    use crate::ir::{
        Access, Argument, Build, BytesSource, Compare, Condition, ErrorCase, ErrorChild,
        ErrorFamily, Expr, Field, FieldKind, Op, Parameter, Producer, Program, Record,
    };
    use crate::layout::LayoutMode;

    /// Own a small checked mixed-input graph and alternating entry append sequence.
    ///
    /// Bodies cover scalar forwarding, Bytes forwarding, unused Bytes formals and
    /// an unused Apply actual that still performs a trapping input read. This
    /// fixture emits real checked IR; it runs no native code or timing workload.
    fn mixed_program() -> Program {
        let fresh = Producer {
            name: "fresh".into(),
            record: "Payload".into(),
            error: None,
            params: vec![Parameter::U64],
            body: vec![Build::Bytes {
                field: "data".into(),
                len: Expr::Param(0),
                value: Expr::Index,
            }],
        };
        let copy = Producer {
            name: "copy".into(),
            record: "Payload".into(),
            error: None,
            params: vec![Parameter::Bytes],
            body: vec![Build::Bytes {
                field: "data".into(),
                len: Expr::InputLen(0),
                value: Expr::InputByte {
                    param: 0,
                    index: Box::new(Expr::Index),
                },
            }],
        };
        let scalar_forward = Producer {
            name: "scalar_forward".into(),
            record: "Outer".into(),
            error: None,
            params: vec![Parameter::U64],
            body: vec![Build::Record {
                field: "child".into(),
                producer: "fresh".into(),
                args: vec![Argument::Scalar(Expr::Param(0))],
            }],
        };
        let byte_forward = Producer {
            name: "byte_forward".into(),
            record: "Outer".into(),
            error: None,
            params: vec![Parameter::Bytes],
            body: vec![Build::Record {
                field: "child".into(),
                producer: "copy".into(),
                args: vec![Argument::Bytes(BytesSource::Param(0))],
            }],
        };
        let unused = Producer {
            name: "unused_borrow".into(),
            record: "Payload".into(),
            error: None,
            params: vec![Parameter::Bytes],
            body: vec![Build::Bytes {
                field: "data".into(),
                len: Expr::Const(0),
                value: Expr::Const(7),
            }],
        };
        let trapping_actual = Producer {
            name: "trapping_actual".into(),
            record: "Payload".into(),
            error: None,
            params: vec![Parameter::Bytes],
            body: vec![Build::Bytes {
                field: "data".into(),
                len: Expr::Const(1),
                value: Expr::Apply {
                    args: vec![Expr::InputByte {
                        param: 0,
                        index: Box::new(Expr::Const(u64::MAX)),
                    }],
                    body: Box::new(Expr::Const(7)),
                },
            }],
        };
        Program {
            records: vec![
                Record {
                    name: "Payload".into(),
                    fields: vec![Field {
                        name: "data".into(),
                        kind: FieldKind::Bytes,
                    }],
                },
                Record {
                    name: "Outer".into(),
                    fields: vec![Field {
                        name: "child".into(),
                        kind: FieldKind::Record("Payload".into()),
                    }],
                },
            ],
            producers: vec![
                fresh,
                copy,
                scalar_forward,
                byte_forward,
                unused,
                trapping_actual,
            ],
            body: vec![Op::Region {
                name: "memory".into(),
                capacity: Expr::Const(128),
                fail_acquire: false,
                on_failure: vec![],
                body: vec![Op::Batch {
                    name: "items".into(),
                    region: "memory".into(),
                    record: "Payload".into(),
                    body: vec![
                        Op::Append {
                            batch: "items".into(),
                            producer: "fresh".into(),
                            args: vec![Argument::Scalar(Expr::Const(4))],
                            status: "seeded".into(),
                        },
                        Op::Borrow {
                            name: "source".into(),
                            batch: "items".into(),
                            index: Expr::Const(0),
                            access: Access::Readonly,
                        },
                        Op::Append {
                            batch: "items".into(),
                            producer: "copy".into(),
                            args: vec![Argument::Bytes(BytesSource::Field {
                                reference: "source".into(),
                                path: vec!["data".into()],
                            })],
                            status: "copied".into(),
                        },
                        Op::Append {
                            batch: "items".into(),
                            producer: "fresh".into(),
                            args: vec![Argument::Scalar(Expr::Const(4))],
                            status: "fresh_again".into(),
                        },
                    ],
                }],
            }],
        }
    }

    /// Extract one compiler-owned canonical function body for scope assertions.
    /// Missing/malformed output rejects rather than inspecting an unrelated body.
    fn body<'a>(llvm: &'a str, header: &str) -> &'a str {
        llvm.split_once(header)
            .expect("emitted function exists")
            .1
            .split_once("\n}\n")
            .expect("emitted function is complete")
            .0
    }

    /// Metadata must not leak from fresh producers into borrowed-input functions or appends.
    /// Existing native regression fixtures separately establish runtime trap/failure order.
    #[test]
    fn scalar_input_hints_exclude_borrowing_and_alternating_entry_sites() {
        let program = mixed_program();
        let checked = crate::check::check(&program).unwrap();
        // Representation changes affect physical fields, never input-kind scope.
        for mode in [
            LayoutMode::Wide,
            LayoutMode::Derived64,
            LayoutMode::Compact32,
        ] {
            let llvm = super::super::emit_with_layout(&checked, "aarch64-apple-macosx11.0.0", mode)
                .unwrap();
            // Both the leaf and transitive scalar-input wrapper retain hints.
            for index in [0, 2] {
                let function = body(
                    &llvm,
                    &format!("define internal i64 @holo.producer.{index}("),
                );
                assert!(
                    function.contains("!prof"),
                    "scalar forwarding retains the hint"
                );
            }
            // Direct/forwarded/unused/trapping byte inputs all remain excluded.
            for index in [1, 3, 4, 5] {
                let function = body(
                    &llvm,
                    &format!("define internal i64 @holo.producer.{index}("),
                );
                assert!(
                    !function.contains("!prof"),
                    "borrowed signatures remain unhinted"
                );
            }
            let trapping = body(&llvm, "define internal i64 @holo.producer.5(");
            assert!(trapping.contains("call void @holo.assert"));
            let entry = body(&llvm, "define i32 @main(");
            let mut append_hints = Vec::new();
            let mut waiting = false;
            // Capture the decision belonging to each producer invocation, not
            // unrelated acquisition/assert/bounds branches elsewhere in entry.
            for line in entry.lines() {
                if line.contains("call i64 @holo.producer.") {
                    waiting = true;
                } else if waiting && line.trim_start().starts_with("br i1 ") {
                    append_hints.push(line.contains("!prof"));
                    waiting = false;
                }
            }
            assert_eq!(append_hints, [true, false, true]);
        }
    }

    /// Check that a fixed parent forwards the complete pair from a
    /// specialized child and the child encodes the lifted rejection constant.
    /// No native execution is needed to check this pre-optimization shape.
    #[test]
    fn fixed_terminal_error_lift_preserves_storage_statuses_and_payload() {
        let child = Producer {
            name: "leaf".into(),
            record: "Leaf".into(),
            error: Some("LeafError".into()),
            params: vec![Parameter::U64],
            body: vec![
                Build::Reject {
                    condition: Condition {
                        op: Compare::Eq,
                        left: Expr::Param(0),
                        right: Expr::Const(0),
                    },
                    case: "Rejected".into(),
                    payload: Some(Expr::Const(u64::MAX)),
                },
                Build::U64 {
                    field: "value".into(),
                    value: Expr::Param(0),
                },
            ],
        };
        let wrapper = |name: &str, family: &str| Producer {
            name: name.into(),
            record: "Wrapper".into(),
            error: Some(family.into()),
            params: vec![Parameter::U64],
            body: vec![Build::Record {
                field: "leaf".into(),
                producer: "leaf".into(),
                args: vec![Argument::Scalar(Expr::Param(0))],
            }],
        };
        let program = Program {
            records: vec![
                Record {
                    name: "Leaf".into(),
                    fields: vec![Field {
                        name: "value".into(),
                        kind: FieldKind::U64,
                    }],
                },
                Record {
                    name: "Wrapper".into(),
                    fields: vec![Field {
                        name: "leaf".into(),
                        kind: FieldKind::Record("Leaf".into()),
                    }],
                },
            ],
            producers: vec![
                child,
                wrapper("identity", "LeafError"),
                wrapper("lift", "WrapperError"),
            ],
            body: vec![
                Op::ErrorFamily(ErrorFamily {
                    name: "LeafError".into(),
                    cases: vec![ErrorCase {
                        name: "Rejected".into(),
                        payload: true,
                    }],
                    children: Vec::new(),
                }),
                Op::ErrorFamily(ErrorFamily {
                    name: "WrapperError".into(),
                    cases: vec![
                        ErrorCase {
                            name: "Unavailable".into(),
                            payload: false,
                        },
                        ErrorCase {
                            name: "Leaf.Rejected".into(),
                            payload: true,
                        },
                    ],
                    children: vec![ErrorChild {
                        alias: "Leaf".into(),
                        family: "LeafError".into(),
                    }],
                }),
            ],
        };
        let checked = crate::check::check(&program).unwrap();
        let llvm = super::super::emit_with_layout(
            &checked,
            "aarch64-apple-macosx11.0.0",
            LayoutMode::Derived64,
        )
        .unwrap();
        let exact = body(&llvm, "define internal { i64, i64 } @holo.producer.1(");
        assert!(!exact.contains("extractvalue"), "{exact}");
        assert!(exact.contains("ret { i64, i64 } %"), "{exact}");
        let lifted = body(&llvm, "define internal { i64, i64 } @holo.producer.2(");
        assert!(lifted.contains("@holo.producer.0.lift.1("), "{lifted}");
        assert!(!lifted.contains("extractvalue"), "{lifted}");
        assert!(lifted.contains("ret { i64, i64 } %"), "{lifted}");
        let canonical = body(&llvm, "define internal { i64, i64 } @holo.producer.0(");
        let variant = body(
            &llvm,
            "define internal { i64, i64 } @holo.producer.0.lift.1(",
        );
        assert!(canonical.contains("insertvalue { i64, i64 } { i64 4, i64 0 }"));
        assert_eq!(canonical.replace("i64 4, i64 0", "i64 5, i64 0"), variant);
    }
}
