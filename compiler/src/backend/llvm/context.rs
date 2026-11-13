//! Per-function lowering context and lexical runtime bindings.
//!
//! This module owns backend-local names and their native storage. Source roots,
//! access permissions, producer purity, and field types have already been checked.
//! A rebindable reference slot holds one native pointer. A scoped record lends
//! its already resolved address directly. Neither form needs an ownership counter.
//! Compiler maps and cloned names affect build memory only.

use std::cell::Cell;
use std::collections::HashMap;

use crate::ir::{EnumDef, FieldKind, Program};
use crate::layout::LayoutPlan;

use super::{
    byte_sum, cursor_proof, cursor_staging, function::Function, input::Input,
    literal::ByteLiterals, staging_slots::StableStaging, storage, uniform_batch,
};

/// Checked input, physical layout, and literal pool shared during module emission.
pub(super) struct Module<'a> {
    pub(super) program: &'a Program,
    pub(super) layout: &'a LayoutPlan,
    pub(super) byte_literals: ByteLiterals,
    /// Checked declarations indexed once for lowering all enum literals.
    pub(super) enums: HashMap<&'a str, &'a EnumDef>,
}

/// Native storage associated with a checked lexical binding.
#[derive(Clone)]
pub(super) enum Binding {
    /// Mutable unsigned scalar stored in a fixed entry slot.
    Scalar(String),
    /// Immutable scalar selected from a result's success arm, already in SSA.
    ScalarValue(String),
    /// Immutable boolean in its native i1 representation.
    Boolean(String),
    /// Mutable boolean stored in a fixed i1 entry slot.
    BooleanSlot(String),
    /// Immutable enum discriminant in SSA.
    Enum(String),
    /// Mutable enum discriminant stored in a fixed i8 entry slot.
    EnumSlot(String),
    /// Immutable result with its payload and tag captured in SSA.
    SavedResult {
        /// Captured tag and payload.
        value: String,
        /// Checked family used for explicit propagation.
        family: String,
    },
    /// Pointer and exact native count of the current lexical received view.
    Received {
        /// Start of the checked destination byte field.
        contents: String,
        /// Positive native byte count already checked against the request.
        len: String,
    },
    /// Byte view captured by a lexical declaration. Access is checked before lowering.
    ByteView {
        /// Address captured after evaluating the view's bounds.
        contents: String,
        /// Logical byte extent captured at the declaration.
        len: String,
    },
    /// Initialized region metadata stored in a fixed entry slot.
    Region(String),
    /// Batch metadata and the schema of all its published records.
    Batch {
        /// Pointer to `%holo.batch` metadata.
        slot: String,
        /// Checked logical record name.
        record: String,
        /// Identity of the live acquisition owning the backing.
        region: String,
        /// Optional extent proof for all records this lexical batch publishes.
        shape: Option<uniform_batch::Shape>,
    },
    /// A synchronous view of the owner's published records.
    BorrowedBatch {
        /// Captured publication boundary, with no metadata or owner pointer.
        prefix: BatchPrefix,
        /// Checked logical record name.
        record: String,
    },
    /// Rebindable element reference and its established record schema.
    Reference {
        /// Pointer to a fixed native pointer slot.
        slot: String,
        /// Checked logical record name.
        record: String,
        /// Known live acquisition, when the slot cannot be rebound.
        region: Option<String>,
        /// Exact initialized byte-field extent from a proved cursor batch.
        extent: Option<u64>,
    },
    /// Direct reference to the single record owned by a success arm.
    ScopedReference {
        /// Address passed to the producer, already in SSA.
        pointer: String,
        /// Checked logical record name.
        record: String,
    },
}

/// Native operands describing one captured published batch prefix.
#[derive(Clone)]
pub(super) struct BatchPrefix {
    /// First byte of the prefix, or its undereferenced empty boundary.
    pub(super) contents: String,
    /// Byte distance to the captured end of the prefix.
    pub(super) extent: String,
    /// Published record count at capture.
    pub(super) count: String,
}

/// One active lexical cleanup, with operands captured at scope acquisition.
///
/// These compiler records are never runtime objects. Reverse iteration emits one
/// fixed cleanup per active owner, independent of object count.
#[derive(Clone)]
pub(super) enum Cleanup {
    /// Release the sole accepted TCP descriptor held in the entry's SSA operand.
    Tcp,
    /// Release the sole successfully acquired backing allocation.
    Region {
        /// Native base pointer returned by the successful acquisition.
        base: String,
    },
    /// Restore one saved construction-tail marker before its owner ends.
    Tail {
        /// Pointer to initialized region metadata.
        region: String,
        /// Marker captured before the batch or scoped record acquired the tail.
        start: String,
    },
}

/// Native destinations and the cleanup boundary for one enclosing loop.
pub(super) struct LoopTargets {
    /// Continuation after the loop.
    pub(super) exit: String,
    /// Back edge for a plain loop or the advance block for a finite traversal.
    pub(super) advance: String,
    /// Number of active cleanups belonging to scopes outside this loop.
    pub(super) cleanup_depth: usize,
}

/// Mutable emission state for exactly one native function.
pub(super) struct Lowering<'a> {
    /// Validated program supplying schemas and constructor identities.
    pub(super) program: &'a Program,
    /// Checked enum declarations shared across emitted functions.
    pub(super) enums: &'a HashMap<&'a str, &'a EnumDef>,
    /// Checked schema-driven representation; no runtime layout dispatch is emitted.
    pub(super) layout: &'a LayoutPlan,
    /// SSA and basic-block text under construction.
    pub(super) f: Function,
    /// Module constants collected from entry-only text output, in emission order.
    pub(super) text_literals: Vec<String>,
    /// Module-wide deduplicated storage for readonly byte literals.
    pub(super) byte_literals: &'a ByteLiterals,
    /// Native operands in the current producer, ordinary call, or pure scalar frame.
    pub(super) params: Vec<Input>,
    /// Declared error family of the ordinary function receiving a `try` failure.
    pub(super) result_family: Option<&'a str>,
    /// Current byte-comprehension index, only while its value is being emitted.
    pub(super) byte_index: Option<String>,
    /// Compiler-only narrow layout heuristic for this scalar-input producer.
    /// Entry/representation contexts default false; it is not a runtime permission
    /// or a probability. Append chooses its own callee-specific hint independently.
    pub(super) construction_hints: bool,
    /// Shared compiler-only use marker for the supported native byte reduction.
    /// Absence selects the scalar path. Constructors have no counted statements.
    pub(super) byte_sums: Option<&'a byte_sum::Helpers>,
    /// Compiler-only opt-in and use marker for checked entry staging copies.
    pub(super) staging_copies: Option<&'a Cell<bool>>,
    /// Whether an unproved slot may use the guarded per-transfer experiment.
    pub(super) guarded_staging_copies: bool,
    /// Compiler-only provenance and active descriptors for the stable-slot trial.
    pub(super) stable_staging: Option<StableStaging>,
    /// Compile-time identities of copies accepted by the isolated cursor proof.
    pub(super) cursor_staging: Option<&'a cursor_proof::Plan>,
    /// Compile-time size policy and use marker for the shared wide helper.
    pub(super) outlined_cursor_staging: Option<&'a cursor_staging::Outline>,
    /// Whether to try the uniform batch extent proof in entry scopes.
    pub(super) uniform_batch_extents: bool,
    /// A native host calls the generated entry and receives its process status.
    pub(super) hosted_input: bool,
    /// Only this profile resolves descriptor operands against a current client.
    pub(super) hosted_session: bool,
    /// Lexical name maps, innermost last; no runtime map is emitted.
    scopes: Vec<HashMap<String, Binding>>,
    /// Outermost-first lexical cleanup actions, retained across emitted branches.
    pub(super) cleanups: Vec<Cleanup>,
    /// Loop destinations, innermost last.
    pub(super) loops: Vec<LoopTargets>,
}

impl<'a> Lowering<'a> {
    /// Create a function context from checked program metadata and native header.
    ///
    /// `params` contains the function's typed LLVM operands. The returned context
    /// starts with one empty lexical scope and owns compiler-only maps.
    pub(super) fn new(module: &'a Module<'a>, header: String, params: Vec<Input>) -> Self {
        Self {
            program: module.program,
            enums: &module.enums,
            layout: module.layout,
            f: Function::new(header),
            text_literals: Vec::new(),
            byte_literals: &module.byte_literals,
            params,
            result_family: None,
            byte_index: None,
            construction_hints: false,
            byte_sums: None,
            staging_copies: None,
            guarded_staging_copies: false,
            stable_staging: None,
            cursor_staging: None,
            outlined_cursor_staging: None,
            uniform_batch_extents: false,
            hosted_input: false,
            hosted_session: false,
            scopes: vec![HashMap::new()],
            cleanups: Vec::new(),
            loops: Vec::new(),
        }
    }

    /// Introduce an empty lexical binding scope without native allocation work.
    pub(super) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// End the innermost compiler binding scope after emitting its cleanup.
    ///
    /// Structured owner operations emit runtime cleanup directly. No map or
    /// source object traversal occurs at runtime.
    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop().expect("lowering owns a lexical scope");
    }

    /// Associate a validated new name with native storage in the current scope.
    ///
    /// `name` and `binding` are compiler data; duplicate/invalid declarations are
    /// rejected before lowering rather than resolved by this insertion.
    pub(super) fn bind(&mut self, name: &str, binding: Binding) {
        self.scopes
            .last_mut()
            .expect("lowering owns a lexical scope")
            .insert(name.to_owned(), binding);
    }

    /// Find the nearest checked binding by name and return its native descriptor.
    ///
    /// A missing name is an internal invariant failure because the public emitter
    /// only accepts `CheckedProgram`. Cloning copies compiler strings, not records.
    pub(super) fn binding(&self, name: &str) -> Binding {
        self.find_binding(name)
            .unwrap_or_else(|| panic!("checked binding missing during lowering: {name}"))
    }

    /// Resolve a lexical name, including prospective operands absent at a preheader.
    /// The lookup owns only compiler-side strings.
    pub(super) fn find_binding(&self, name: &str) -> Option<Binding> {
        // Lexical lookup stops at the closest binding, preserving shadowing.
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.get(name) {
                return Some(binding.clone());
            }
        }
        None
    }

    /// Obtain a live batch's metadata pointer and established element schema.
    pub(super) fn batch(&self, name: &str) -> (String, String) {
        // Validation has already ruled out scalar/region/reference operands here.
        match self.binding(name) {
            Binding::Batch { slot, record, .. } => (slot, record),
            _ => panic!("checked batch has wrong lowering kind: {name}"),
        }
    }

    /// Snapshot the published boundary before evaluating the next call actual.
    /// Forwarding a borrowed view reuses its original three native operands.
    pub(super) fn capture_batch(&mut self, name: &str) -> BatchPrefix {
        match self.binding(name) {
            Binding::Batch { slot, .. } => {
                let region_slot = storage::member(&mut self.f, "%holo.batch", &slot, 0);
                let region = self.f.value(format!("load ptr, ptr {region_slot}"));
                let start_slot = storage::member(&mut self.f, "%holo.batch", &slot, 1);
                let start = self.f.value(format!("load i64, ptr {start_slot}"));
                let end_slot = storage::member(&mut self.f, "%holo.batch", &slot, 2);
                let end = self.f.value(format!("load i64, ptr {end_slot}"));
                let contents = storage::address_at(&mut self.f, &region, &start);
                let extent = self.f.value(format!("sub i64 {end}, {start}"));
                let count = self.load_batch_count(&slot);
                BatchPrefix {
                    contents,
                    extent,
                    count,
                }
            }
            Binding::BorrowedBatch { prefix, .. } => prefix,
            _ => unreachable!("checked borrowed batch actual must name a batch"),
        }
    }

    /// Return the published count of an owner or the captured count of a view.
    pub(super) fn batch_count(&mut self, name: &str) -> String {
        match self.binding(name) {
            Binding::Batch { slot, .. } => self.load_batch_count(&slot),
            Binding::BorrowedBatch { prefix, .. } => prefix.count,
            _ => unreachable!("checked batch length requires a batch"),
        }
    }

    /// Read a live owner's current count from compiler-owned metadata.
    fn load_batch_count(&mut self, batch: &str) -> String {
        let slot = storage::member(&mut self.f, "%holo.batch", batch, 3);
        self.f.value(format!("load i64, ptr {slot}"))
    }

    /// Bind a checked record slot, recording only origins the trial can keep stable.
    /// A possible rebind removes the fact conservatively for that name.
    pub(super) fn bind_reference(
        &mut self,
        name: &str,
        slot: String,
        record: String,
        region: Option<String>,
        extent: Option<u64>,
    ) {
        let region = region.filter(|_| {
            self.stable_staging
                .as_ref()
                .is_some_and(|trial| trial.keeps_origin(name))
        });
        self.bind(
            name,
            Binding::Reference {
                slot,
                record,
                region,
                extent,
            },
        );
    }

    /// Get the proved shape of one live batch in the optional extent trial.
    /// The ordinary path skips the additional compiler binding lookup.
    pub(super) fn uniform_batch_shape(&self, name: &str) -> Option<uniform_batch::Shape> {
        // Only the trial attaches shape facts to lexical batch bindings.
        if !self.uniform_batch_extents {
            return None;
        }
        match self.binding(name) {
            Binding::Batch { shape, .. } => shape,
            Binding::BorrowedBatch { .. } => None,
            _ => unreachable!("checked cursor requires a batch"),
        }
    }

    /// Get the constant bound of a checked byte projection through a proved cursor.
    /// The trial only admits a single byte field, so every such projection through
    /// the reference has this extent. Rebinding cannot change its checked root.
    pub(super) fn known_byte_extent(&self, name: &str) -> Option<u64> {
        // Ordinary emission does not pay for the extra compiler lookup.
        if !self.uniform_batch_extents {
            return None;
        }
        // Only a proved cursor batch contributes a fixed byte extent.
        match self.binding(name) {
            Binding::Reference { extent, .. } => extent,
            Binding::ScopedReference { .. } => None,
            _ => unreachable!("checked byte field requires a record reference"),
        }
    }

    /// Get the allocation identity for a batch reference in the opt-in trial.
    /// The ordinary emitter performs no extra binding lookup.
    pub(super) fn batch_origin(&self, name: &str) -> Option<String> {
        self.stable_staging.as_ref()?;
        // A checked borrow always names a batch.
        match self.binding(name) {
            Binding::Batch { region, .. } => Some(region),
            Binding::BorrowedBatch { .. } => None,
            _ => unreachable!("checked reference requires a batch"),
        }
    }

    /// Obtain an initialized region's metadata pointer by checked lexical name.
    pub(super) fn region(&self, name: &str) -> String {
        // A checked region operation never receives a non-region binding.
        match self.binding(name) {
            Binding::Region(slot) => slot,
            _ => panic!("checked region has wrong lowering kind: {name}"),
        }
    }

    /// Return a reference's current pointer and its checked record schema.
    ///
    /// Rebindable paths reload their slots. A scoped record has one address.
    pub(super) fn reference(&mut self, name: &str) -> (String, String) {
        // The checker establishes this slot's pointer type and usable root.
        match self.binding(name) {
            Binding::Reference { slot, record, .. } => {
                let pointer = self.f.value(format!("load ptr, ptr {slot}"));
                (pointer, record)
            }
            Binding::ScopedReference { pointer, record } => (pointer, record),
            _ => panic!("checked reference has wrong lowering kind: {name}"),
        }
    }

    /// Locate an established field path in inline variable-sized data.
    ///
    /// `reference` is a checked live record reference. `path` contains validated
    /// schema fields. Returns the field's first byte; a byte field starts with its
    /// frozen length. Record header presence comes from the explicit layout plan.
    pub(super) fn field_pointer(&mut self, reference: &str, path: &[String]) -> String {
        let (mut pointer, mut schema) = self.reference(reference);
        // Each projection descends through a known inline record, never a pointer.
        for (depth, segment) in path.iter().enumerate() {
            let fields = self
                .program
                .records
                .iter()
                .find(|record| record.name == schema)
                .expect("checked record exists")
                .fields
                .clone();
            let header = self.layout.record(&schema).header_bytes.to_string();
            pointer = storage::advance(&mut self.f, &pointer, &header);
            let mut selected = None;
            // Earlier fields determine this field's physical offset. Variable
            // parts use immutable lengths/sizes, whose validity publication proved.
            for field in fields {
                if field.name == *segment {
                    selected = Some(field.kind);
                    break;
                }
                pointer = match field.kind {
                    FieldKind::U64 => storage::advance(&mut self.f, &pointer, "8"),
                    FieldKind::Bool | FieldKind::Enum(_) => {
                        storage::advance(&mut self.f, &pointer, "1")
                    }
                    FieldKind::Bytes => {
                        let len = self.byte_length(&pointer);
                        let width = self.layout.length_bytes().to_string();
                        let contents = storage::advance(&mut self.f, &pointer, &width);
                        storage::advance(&mut self.f, &contents, &len)
                    }
                    FieldKind::Record(record) => {
                        let size = self.record_size(&record, &pointer);
                        storage::advance(&mut self.f, &pointer, &size)
                    }
                };
            }
            // Intermediate path components have already been checked as records.
            if depth + 1 < path.len() {
                match selected.expect("checked field exists") {
                    FieldKind::Record(next_schema) => schema = next_schema,
                    _ => panic!("checked intermediate projection is not a record"),
                }
            }
        }
        pointer
    }

    /// Resolve a checked producer name to its stable emitted function number.
    pub(super) fn producer_index(&self, name: &str) -> usize {
        self.program
            .producers
            .iter()
            .position(|producer| producer.name == name)
            .expect("checked producer exists")
    }

    /// Resolve a checked schema to its stable representation-helper identity.
    /// Lookup uses compiler metadata only and never scans generated records.
    pub(super) fn record_index(&self, name: &str) -> usize {
        self.program
            .records
            .iter()
            .position(|record| record.name == name)
            .expect("checked record exists")
    }

    /// Read or derive one published record's frozen size using its typed layout.
    /// The helper reads shape metadata only, never payload bytes or other records.
    pub(super) fn record_size(&mut self, record: &str, pointer: &str) -> String {
        let index = self.record_index(record);
        self.f
            .value(format!("call i64 @holo.record_size.{index}(ptr {pointer})"))
    }

    /// Read the selected unsigned byte-length encoding and widen it losslessly.
    /// The checker/publication boundary establishes initialized immutable metadata.
    pub(super) fn byte_length(&mut self, pointer: &str) -> String {
        // Narrow layout metadata is widened before all existing 64-bit indexing arithmetic.
        if self.layout.length_bytes() == 4 {
            let narrow = self.f.value(format!("load i32, ptr {pointer}, align 1"));
            self.f.value(format!("zext i32 {narrow} to i64"))
        } else {
            storage::load_u64(&mut self.f, pointer)
        }
    }
}
