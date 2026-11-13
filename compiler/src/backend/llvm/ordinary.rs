//! Native lowering for checked synchronous ordinary helpers.
//!
//! The checker admits fixed state references, initialized byte views and
//! borrowed batch prefixes. Each call captures operands once and passes only
//! pointers, bounds and copied scalars or booleans. Fixed locals use the function builder.

use crate::ir::{Function, FunctionArgument, FunctionParameterKind, Op};

use super::{
    byte_sum,
    context::{BatchPrefix, Binding, Lowering, Module},
    input::Input,
    results::RESULT_TYPE,
    static_byte_sum::{self, Seed},
};

/// The checked ordinary result determines the private LLVM return type.
fn result_type(function: &Function) -> &'static str {
    if function.returns_bool {
        "i1"
    } else if function.result.is_some() {
        RESULT_TYPE
    } else {
        "i64"
    }
}

/// Emit one checked helper using its generated `index` as an internal symbol.
/// `byte_sums` shares the supported module's helper-use flag with entry lowering.
/// This adds only fixed frame slots for bindings and makes no alias promise.
pub(super) fn emit(
    module: &Module<'_>,
    index: usize,
    function: &Function,
    byte_sums: Option<&byte_sum::Helpers>,
) -> String {
    let mut operands = Vec::new();
    let mut inputs = Vec::with_capacity(function.params.len());
    // Retain typed positions for byte reads while flattening ABI operands.
    for (index, parameter) in function.params.iter().enumerate() {
        let name = format!("%arg{index}");
        let input = match &parameter.kind {
            FunctionParameterKind::U64 => {
                operands.push(format!("i64 {name}"));
                Input::Scalar(name)
            }
            FunctionParameterKind::Bool => {
                operands.push(format!("i1 {name}"));
                Input::Boolean(name)
            }
            FunctionParameterKind::State(_) => {
                operands.push(format!("ptr {name}"));
                Input::State(name)
            }
            FunctionParameterKind::Batch { .. } => {
                let contents = format!("{name}.data");
                let extent = format!("{name}.extent");
                let count = format!("{name}.count");
                operands.push(format!("ptr {contents}"));
                operands.push(format!("i64 {extent}"));
                operands.push(format!("i64 {count}"));
                Input::Batch(BatchPrefix {
                    contents,
                    extent,
                    count,
                })
            }
            FunctionParameterKind::Bytes | FunctionParameterKind::WritableBytes => {
                let contents = format!("{name}.data");
                let len = format!("{name}.len");
                operands.push(format!("ptr {contents}"));
                operands.push(format!("i64 {len}"));
                Input::Bytes { contents, len }
            }
        };
        inputs.push(input);
    }
    let result_type = result_type(function);
    let header = format!(
        "define internal {result_type} @holo.function.{index}({})",
        operands.join(", ")
    );
    let mut lowering = Lowering::new(module, header, inputs);
    lowering.byte_sums = byte_sums;
    lowering.result_family = function.result.as_deref();
    // Locals and state reuse the existing binding representation. Bytes are
    // immutable captures already addressed by formal index and need no slots.
    for (index, parameter) in function.params.iter().enumerate() {
        match (&parameter.kind, lowering.params[index].clone()) {
            (FunctionParameterKind::U64, Input::Scalar(value)) => {
                let slot = lowering.f.slot("i64");
                lowering.f.line(format!("store i64 {value}, ptr {slot}"));
                lowering.bind(&parameter.name, Binding::Scalar(slot));
            }
            (FunctionParameterKind::Bool, Input::Boolean(value)) => {
                lowering.bind(&parameter.name, Binding::Boolean(value));
            }
            (FunctionParameterKind::State(record), Input::State(pointer)) => {
                let slot = lowering.f.slot("ptr");
                lowering.f.line(format!("store ptr {pointer}, ptr {slot}"));
                lowering.bind(
                    &parameter.name,
                    Binding::Reference {
                        slot,
                        record: record.clone(),
                        region: None,
                        extent: None,
                    },
                );
            }
            (FunctionParameterKind::Batch { record, .. }, Input::Batch(prefix)) => {
                lowering.bind(
                    &parameter.name,
                    Binding::BorrowedBatch {
                        prefix,
                        record: record.clone(),
                    },
                );
            }
            (
                FunctionParameterKind::Bytes | FunctionParameterKind::WritableBytes,
                Input::Bytes { .. },
            ) => {}
            _ => unreachable!("ordinary input constructed from its parameter kind"),
        }
    }
    lowering.operations(&function.body);
    // All executable paths return. The structured emitter can leave a final
    // predecessor-free continuation after lowering those paths.
    lowering.f.line("unreachable");
    lowering.f.finish()
}

impl<'a> Lowering<'a> {
    /// Borrow the definition of a checked ordinary call and its native symbol index.
    /// An absent target violates the checked-input invariant. This emits no code.
    pub(super) fn ordinary_definition(&self, name: &str) -> (usize, &'a Function) {
        self.program
            .body
            .iter()
            .enumerate()
            .find_map(|(index, operation)| match operation {
                Op::Function(function) if function.name == name => Some((index, function)),
                _ => None,
            })
            .expect("checked ordinary call target exists")
    }

    /// Capture a checked ordinary call's actuals, then emit one direct call.
    /// Prefix failures trap before the next argument or entry to the callee.
    /// The returned SSA name holds the callee's checked result. Compiler text allocation
    /// and reference-slot loads do not copy caller-owned payload or state.
    pub(super) fn ordinary_call(&mut self, name: &str, args: &[FunctionArgument]) -> String {
        let (index, function) = self.ordinary_definition(name);
        self.ordinary_call_resolved(index, function, args)
    }

    /// Emit the call after resolving its definition for the caller's result handling.
    /// `index` and `function` refer to the same checked definition. This reuses
    /// ordinary actual evaluation and returns the call's SSA result.
    pub(super) fn ordinary_call_resolved(
        &mut self,
        index: usize,
        function: &Function,
        args: &[FunctionArgument],
    ) -> String {
        // Only the supported target and a complete static proof change this call.
        let specialized = self
            .byte_sums
            .and_then(|helpers| static_byte_sum::select(function, args, helpers));
        let mut sum_contents = None;
        let mut sum_seed = specialized.and_then(|sum| match sum.seed {
            Seed::Literal(value) => Some(value.to_string()),
            Seed::Scalar(_) => None,
        });
        let mut operands = Vec::new();
        // Capture each argument fully before beginning the next, even if the
        // callee will never inspect it. Later calls may mutate aliased state.
        for (position, argument) in args.iter().enumerate() {
            match argument {
                FunctionArgument::Scalar(value) => {
                    let value = self.expression(value);
                    // Reuse the once-evaluated original actual if the seed is a formal.
                    if specialized.is_some_and(
                        |sum| matches!(sum.seed, Seed::Scalar(index) if index == position),
                    ) {
                        sum_seed = Some(value.clone());
                    }
                    operands.push(format!("i64 {value}"));
                }
                FunctionArgument::Bool(value) => {
                    let value = self.boolean(value);
                    operands.push(format!("i1 {value}"));
                }
                FunctionArgument::State(name) => {
                    let (pointer, _) = self.reference(name);
                    operands.push(format!("ptr {pointer}"));
                }
                FunctionArgument::Batch(name) => {
                    let BatchPrefix {
                        contents,
                        extent,
                        count,
                    } = self.capture_batch(name);
                    operands.push(format!("ptr {contents}"));
                    operands.push(format!("i64 {extent}"));
                    operands.push(format!("i64 {count}"));
                }
                FunctionArgument::Received(name) => {
                    // This fresh dynamic extent already passed the refill
                    // result guard. Forward its SSA operands without staging.
                    let (contents, len) = self.received_bytes(name);
                    operands.push(format!("ptr {contents}"));
                    operands.push(format!("i64 {len}"));
                }
                FunctionArgument::Bytes { source, ranges } => {
                    let Input::Bytes { contents, len } = self.capture_byte_view(source, ranges)
                    else {
                        unreachable!("checked ordinary Bytes argument has a byte source");
                    };
                    // Save the borrowed location after its original eager bounds check.
                    if specialized.is_some_and(|sum| sum.input == position) {
                        sum_contents = Some(contents.clone());
                    }
                    operands.push(format!("ptr {contents}"));
                    operands.push(format!("i64 {len}"));
                }
            }
        }
        // Even unused or trapping actuals precede the replacement calculation.
        if let (Some(sum), Some(helpers)) = (specialized, self.byte_sums) {
            return static_byte_sum::emit(
                &mut self.f,
                helpers,
                sum.width,
                &sum_contents.expect("proved byte actual was captured"),
                sum_seed.expect("proved scalar seed was captured"),
            );
        }
        let result_type = result_type(function);
        self.f.value(format!(
            "call {result_type} @holo.function.{index}({})",
            operands.join(", ")
        ))
    }
}
