//! LLVM lowering for the validated M1 region construction representation.
//!
//! The public entry point requires a `CheckedProgram`; unchecked operations cannot
//! request native code through this API. Focused child modules separate scalar
//! observations, lexical cleanup, publication, constructors, and storage helpers.
//! Output is native LLVM IR, not generated C or a foreign implementation kernel.

mod byte_sum;
mod context;
mod copy;
mod cursor;
mod cursor_inventory;
mod cursor_proof;
mod cursor_staging;
mod exit;
mod expression;
mod function;
mod input;
mod literal;
mod operations;
mod ordinary;
mod producer;
mod producer_inline;
mod producer_lifts;
mod publication;
mod refill;
mod representation;
mod results;
mod scope;
mod send;
mod session;
mod staging_copy;
mod staging_slots;
mod static_byte_sum;
mod storage;
mod tcp;
mod uniform_batch;

use std::{cell::Cell, collections::HashSet};

use crate::check::CheckedProgram;
use crate::ir::{Build, FunctionParameterKind, Op};
use crate::layout::{LayoutError, LayoutMode, LayoutPlan};

use context::{Lowering, Module};

/// Explain each checked producer byte field's initial copy strategy.
///
/// `target_triple` is the native backend target. The report shares the emitter's
/// selector and returns owned display text without generating LLVM, allocating
/// runtime storage, or promising LLVM's final optimization choice.
pub fn explain_copies(checked: &CheckedProgram<'_>, target_triple: &str) -> String {
    let mut report = format!("Byte initialization for target {target_triple:?}\n");
    let mut found = false;
    // Producer definitions own their byte-field expressions, including synthetic
    // nested definitions. Reporting follows that checked definition order.
    for producer in &checked.program().producers {
        for operation in &producer.body {
            // Scalar and nested-record field steps have no byte strategy here.
            if let Build::Bytes { field, len, value } = operation {
                found = true;
                let reason = copy::choose(target_triple, len, value).explanation();
                report.push_str(&format!(
                    "  producer {:?}, field {field:?}: {reason}\n",
                    producer.name
                ));
            }
        }
    }
    // An empty result means no byte fields are initialized in these definitions.
    if !found {
        report.push_str("  no byte initializers\n");
    }
    report
}

/// Lower a checked region program to a complete LLVM module for a 64-bit target.
///
/// `checked` carries source/schema/root validation; `target_triple` is supplied by
/// the native driver. The returned text owns compiler memory and must pass LLVM
/// verification before execution. No target feature, alias, or speed guarantee is
/// inferred here. The driver owns optimization, object generation, and linking.
pub fn emit(checked: &CheckedProgram<'_>, target_triple: &str) -> String {
    // The preserved wide plan represents every checked schema using runtime
    // headers even when an optional static extent cannot fit a host integer.
    emit_with_layout(checked, target_triple, LayoutMode::Wide)
        .expect("wide checked layouts are always representable")
}

/// Lower checked input with one explicit schema-driven representation experiment.
///
/// The plan is derived from this exact checked program, preventing a caller from
/// pairing another program's layout facts with its storage operations. Invalid
/// static layout extents return LayoutError before native code is emitted. No
/// layout change relocates existing data or bypasses access/root verification.
/// On macOS arm64, Derived64 admits a checked exact 192-byte checksum prefix.
pub fn emit_with_layout(
    checked: &CheckedProgram<'_>,
    target_triple: &str,
    mode: LayoutMode,
) -> Result<String, LayoutError> {
    // The qualified layout uses the existing exact-width policy. Target
    // eligibility is still checked where the byte-sum helpers are created.
    let emission = if mode == LayoutMode::Derived64 {
        Emission::ExactThreeBlockSums
    } else {
        Emission::Ordinary
    };
    emit_with_options(checked, target_triple, mode, emission)
}

/// Select the wide byte-sum helper for an exact three-block literal prefix.
///
/// Derived64 uses this selection by default on macOS arm64. The explicit
/// entry also lets other layouts exercise it on that target. Prefix
/// evaluation uses the ordinary checked path with no runtime width dispatch
/// or added storage. Unsupported targets retain ordinary emission.
pub fn emit_with_exact_three_block_sums(
    checked: &CheckedProgram<'_>,
    target_triple: &str,
    mode: LayoutMode,
) -> Result<String, LayoutError> {
    emit_with_options(checked, target_triple, mode, Emission::ExactThreeBlockSums)
}

/// Emit a captured-prefix experiment for uniformly initialized byte records.
/// The macOS arm64 Derived64 trial proves each eligible lexical batch's
/// append extents before using a constant cursor advance and field bound.
/// Other batches and targets retain the ordinary checked emitter. Neither
/// storage representation nor public source syntax changes.
pub fn emit_with_uniform_batch_extents(
    checked: &CheckedProgram<'_>,
    target_triple: &str,
    mode: LayoutMode,
) -> Result<String, LayoutError> {
    emit_with_options(checked, target_triple, mode, Emission::UniformBatchExtents)
}

/// Extend the outlined bulk transfer to all proved entry staging widths.
///
/// The same checked input and layout reach the ordinary cursor proof. Only
/// macOS arm64 Derived64 sites with proved initialized, disjoint ranges can
/// use the private helper, and only above the existing short-copy limit.
/// The ordinary emitter limits the helper to a proved small-buffer maximum.
/// The helper adds no storage or source-level conditions. Unproved sites and
/// other targets retain ordinary emission. Layout failures return `LayoutError`.
pub fn emit_with_outlined_bulk_staging(
    checked: &CheckedProgram<'_>,
    target_triple: &str,
    mode: LayoutMode,
) -> Result<String, LayoutError> {
    emit_with_options(checked, target_triple, mode, Emission::OutlinedCursor)
}

/// Opt into the checked staging-copy experiment for ordinary entry loops.
///
/// This preserves the input checked program and selected layout. It emits a
/// range-copy fast path on macOS arm64 only when both initialized subranges
/// and disjointness are proved at runtime. All other cases retain the
/// original checked loop. The ordinary compiler entry point is unchanged.
pub fn emit_with_staging_copy(
    checked: &CheckedProgram<'_>,
    target_triple: &str,
    mode: LayoutMode,
) -> Result<String, LayoutError> {
    emit_with_options(checked, target_triple, mode, Emission::Guarded)
}

/// Emit the checked staging experiment with stable byte field descriptors.
///
/// The extra trial is limited to the same macOS arm64 target. It borrows the
/// same checked input and selected layout. It hoists a pair of live fields
/// only for fixed reference slots with distinct acquisition identities.
/// Unsupported slots retain the per-transfer experiment and its scalar fallback.
pub fn emit_with_stable_staging_slots(
    checked: &CheckedProgram<'_>,
    target_triple: &str,
    mode: LayoutMode,
) -> Result<String, LayoutError> {
    emit_with_options(checked, target_triple, mode, Emission::StableSlots)
}

/// Emit the release candidate using only staging copies with a live-slot proof.
///
/// The checked program and requested layout are shared with ordinary emission.
/// On the reviewed macOS arm64 target, a proved slot pair may reuse its descriptors
/// and lower bounded transfers. Unproved sites keep the ordinary scalar loop.
/// This entry point lets callers verify and time the candidate before adoption.
pub fn emit_with_proved_staging_slots(
    checked: &CheckedProgram<'_>,
    target_triple: &str,
    mode: LayoutMode,
) -> Result<String, LayoutError> {
    emit_with_options(checked, target_triple, mode, Emission::ProvedSlots)
}

/// Module emission policy selected by the default or an explicit experiment.
enum Emission {
    /// Bounded staging outlines with the original large-literal scan threshold.
    Ordinary,
    /// Use bounded staging outlines and include the exact three-block scan width.
    ExactThreeBlockSums,
    /// Prove uniform batch extents while using the ordinary staging policy.
    UniformBatchExtents,
    /// Experimentally outline the wide edge at every selected cursor site.
    OutlinedCursor,
    /// Check full subranges and their separation at each eligible transfer.
    Guarded,
    /// Try live allocation and descriptor reuse before the guarded path.
    StableSlots,
    /// Qualify only the live-slot proof for ordinary compiler use.
    ProvedSlots,
}

/// Emit one module with its normal layout and a compiler-only policy.
///
/// A candidate shares the ordinary checked input. Helper definitions and
/// declarations are emitted only when a selected site needs them.
fn emit_with_options(
    checked: &CheckedProgram<'_>,
    target_triple: &str,
    mode: LayoutMode,
    emission: Emission,
) -> Result<String, LayoutError> {
    let program = checked.program();
    let layout = LayoutPlan::new(checked, mode)?;
    let context = Module {
        program,
        layout: &layout,
        byte_literals: literal::ByteLiterals::default(),
        enums: program
            .body
            .iter()
            .filter_map(|op| match op {
                Op::Enum(definition) => Some((definition.name.as_str(), definition)),
                _ => None,
            })
            .collect(),
    };
    // Escape unusual configuration characters rather than allowing target text to
    // inject LLVM syntax. An unsupported triple still fails in the native driver.
    let triple = target_triple
        .replace('\\', "\\5C")
        .replace('"', "\\22")
        .replace('\n', "\\0A")
        .replace('\r', "\\0D");
    let mut module = format!(
        "; Holo checked-region IR: initial bounded backing configuration\n\
         ; Explicit physical layout: {}\n\
         target triple = \"{triple}\"\n\n{}\n",
        mode.name(),
        include_str!("../../../../runtime/llvm/prelude.ll")
    );
    let byte_sums = byte_sum::supports_target(target_triple)
        .then(|| byte_sum::Helpers::new(matches!(emission, Emission::ExactThreeBlockSums)));
    let staging_copies = (matches!(
        emission,
        Emission::Guarded | Emission::StableSlots | Emission::ProvedSlots
    ) && copy::supports_target(target_triple))
    .then(|| Cell::new(false));
    let copy_inline_hints = copy::inline_candidates(program, target_triple);
    let constructor_copies = copy_inline_hints.iter().any(|&hint| hint);
    // Declare fixed inline transfers only for modules with a guarded copy site.
    if constructor_copies {
        module.push_str(copy::DECLARATIONS);
        module.push_str(&copy::checked_loop(&context));
        module.push('\n');
    }
    // Every schema gets the same typed extent/lookup path in every mode. Native
    // optimization can inline constant extents without runtime layout dispatch.
    for index in 0..program.records.len() {
        module.push_str(&representation::size(&context, index));
        module.push('\n');
        module.push_str(&representation::indexed(index));
        module.push('\n');
    }
    // Borrowed prefixes add a lookup symbol only for schemas crossing an
    // ordinary call boundary. Existing owner modules keep their original IR.
    let mut borrowed_batch_records = HashSet::new();
    for operation in &program.body {
        if let Op::Function(function) = operation {
            for parameter in &function.params {
                if let FunctionParameterKind::Batch { record, .. } = &parameter.kind {
                    borrowed_batch_records.insert(record.as_str());
                }
            }
        }
    }
    for (index, record) in program.records.iter().enumerate() {
        if borrowed_batch_records.contains(record.name.as_str()) {
            module.push_str(&representation::indexed_prefix(index));
            module.push('\n');
        }
    }
    let lifts = producer_lifts::ProducerLifts::new(program, &layout);
    let inlining = producer_inline::plan(program, &layout, target_triple, &copy_inline_hints);
    // Keep the generic definition for ordinary append and any call whose
    // nominal encoding matches it. Emit specialized definitions separately.
    for (index, &inline) in inlining.iter().enumerate() {
        module.push_str(&producer::emit(
            &context,
            index,
            target_triple,
            inline,
            &lifts,
            0,
        ));
        module.push('\n');
    }
    // Each variant uses the same checked destination and actual operands.
    for (index, &inline) in inlining.iter().enumerate() {
        for &lift in lifts.variants(index) {
            module.push_str(&producer::emit(
                &context,
                index,
                target_triple,
                inline,
                &lifts,
                lift,
            ));
            module.push('\n');
        }
    }
    // An outermost declaration is emitted once as a direct internal function.
    // Programs without declarations retain exactly their previous module text.
    for (index, operation) in program.body.iter().enumerate() {
        if let Op::Function(function) = operation {
            module.push_str(&ordinary::emit(
                &context,
                index,
                function,
                byte_sums.as_ref(),
            ));
            module.push('\n');
        }
    }
    // Hosted modules return through a C entry. The TCP profile takes its
    // descriptor; other hosted profiles borrow theirs from the native owner.
    // Ordinary modules keep their direct main and avoid the native adapters.
    let hosted = checked.hosted_input();
    let tcp = checked.hosted_tcp();
    let header = if tcp {
        module.push_str(refill::DECLARATIONS);
        module.push_str(send::DECLARATIONS);
        module.push_str(tcp::DECLARATIONS);
        "define i32 @holo_tcp_entry(i64 %refill_kind, i64 %refill_fd, i64 %tcp_queue)"
    } else if hosted {
        module.push_str(refill::DECLARATIONS);
        // Only output programs stage the optional adapter and its setup.
        if checked.hosted_output() {
            module.push_str(send::DECLARATIONS);
        }
        // A listener is available only when entry explicitly advances sessions.
        if checked.hosted_session() {
            module.push_str(session::DECLARATIONS);
        }
        "define i32 @holo_hosted_entry(i64 %refill_kind, i64 %refill_fd)"
    } else {
        "define i32 @main()"
    };
    let mut entry = Lowering::new(&context, header.to_owned(), Vec::new());
    entry.byte_sums = byte_sums.as_ref();
    entry.staging_copies = staging_copies.as_ref();
    entry.guarded_staging_copies = matches!(emission, Emission::Guarded | Emission::StableSlots);
    // No proof scanning or cached descriptors are needed on ordinary targets.
    if staging_copies.is_some() && matches!(emission, Emission::StableSlots | Emission::ProvedSlots)
    {
        entry.stable_staging = Some(staging_slots::StableStaging::new(&program.body));
    }
    // Only entry borrows the plan for the qualified macOS arm64 layout.
    let cursor_staging = (matches!(
        emission,
        Emission::Ordinary
            | Emission::OutlinedCursor
            | Emission::UniformBatchExtents
            | Emission::ExactThreeBlockSums
    ) && mode == LayoutMode::Derived64
        && copy::supports_target(target_triple))
    .then(|| cursor_proof::plan(program))
    .filter(|plan| !plan.is_empty());
    // Ordinary emission uses the qualified upper bound. The explicit trial
    // remains available to compare larger ranges without changing source.
    let outlined_cursor_staging = cursor_staging
        .as_ref()
        .map(|_| cursor_staging::Outline::new(matches!(emission, Emission::OutlinedCursor)));
    entry.cursor_staging = cursor_staging.as_ref();
    entry.outlined_cursor_staging = outlined_cursor_staging.as_ref();
    entry.uniform_batch_extents = matches!(emission, Emission::UniformBatchExtents)
        && mode == LayoutMode::Derived64
        && copy::supports_target(target_triple);
    entry.hosted_input = hosted || tcp;
    entry.hosted_session = checked.hosted_session();
    if hosted && !tcp {
        entry.guard_refill_host();
    }
    // Normalize the listener's kind for reused stream I/O adapters.
    if entry.hosted_session {
        entry.bind_session_io_kind();
    }
    entry.operations(&program.body);
    entry.f.line("ret i32 0");
    emit_text_literals(&mut module, &entry.text_literals);
    context.byte_literals.emit(&mut module);
    module.push_str(&entry.f.finish());
    // Shared metadata is a private optimizer heuristic, not a proof that either
    // failure edge is unreachable. The function builder owns both fixed identities.
    module.push('\n');
    module.push_str(function::CONSTRUCTION_BRANCH_METADATA);
    // Omit the helper and its declarations on unsupported targets or when all
    // loops retained scalar lowering. Unaffected modules keep their exact text.
    if let Some(byte_sums) = byte_sums {
        byte_sums.emit(&mut module);
    }
    // Constructor copies already declare the same intrinsics. A staging-only
    // module needs the declarations too, without changing untouched modules.
    if (cursor_staging.is_some() || staging_copies.is_some_and(Cell::into_inner))
        && !constructor_copies
    {
        module.push_str(copy::DECLARATIONS);
    }
    // Emit one helper for the module only after a proved site actually calls it.
    if outlined_cursor_staging.is_some_and(|outline| outline.used()) {
        cursor_staging::emit_outlined_bulk(&mut module);
    }
    Ok(module)
}

/// Append NUL-terminated literal constants and the host output declaration.
/// Checked UTF-8 bytes are escaped uniformly, so source cannot become LLVM
/// syntax. The module grows linearly in literal bytes and references them
/// directly at runtime. A program with no text output adds no LLVM text.
fn emit_text_literals(module: &mut String, literals: &[String]) {
    use std::fmt::Write as _;

    // Other programs retain the original prelude and require no puts symbol.
    if literals.is_empty() {
        return;
    }
    module.push_str("declare i32 @puts(ptr)\n");
    // Each constant's identity matches the checked print site in entry.
    for (index, text) in literals.iter().enumerate() {
        write!(
            module,
            "@holo.print_text.{index} = private unnamed_addr constant [{} x i8] c\"",
            text.len() + 1
        )
        .expect("writing to a String");
        // Encode bytewise to preserve UTF-8, quotes and percent characters.
        for byte in text.as_bytes() {
            write!(module, "\\{byte:02X}").expect("writing to a String");
        }
        module.push_str("\\00\"\n");
    }
    module.push('\n');
}
