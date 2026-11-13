//! Textual LLVM function construction shared by the backend's focused emitters.
//!
//! Values and block labels have generated numeric identities, so source names are
//! never interpolated into LLVM identifiers. Stack slots are collected in the
//! entry block even when their source declaration is inside a loop; this avoids
//! allocating a new native stack slot on every iteration. This builder performs
//! no source-language checking and makes no alias promises. Narrow construction
//! branch weights are experimental optimizer hints, never safety facts or measured
//! probabilities; the caller must explicitly choose each eligible occurrence.

/// Fixed success-hint metadata in both predicate orientations, defined once per module.
///
/// The private function builder owns these identities. Both destinations remain
/// executable at any frequency; changing block-placement heuristics may regress
/// unrelated generated code and requires paired performance evidence.
pub(super) const CONSTRUCTION_BRANCH_METADATA: &str = "!0 = !{!\"branch_weights\", i32 2000, i32 1}\n\
     !1 = !{!\"branch_weights\", i32 1, i32 2000}\n";

/// One LLVM function under construction, with entry allocas separate from code.
pub(super) struct Function {
    /// LLVM definition line without its opening brace.
    header: String,
    /// Fixed native slots, emitted once at function entry.
    slots: Vec<String>,
    /// Instructions and labels in control-flow order.
    body: Vec<String>,
    /// Body index of the current label, or no index in the entry block.
    current_label: Option<usize>,
    /// Next unique SSA-value identity within this function.
    next_value: usize,
    /// Next unique block identity within this function.
    next_label: usize,
}

impl Function {
    /// Create an empty function from its LLVM definition header.
    ///
    /// `header` includes linkage, result type, name, and typed arguments; it is
    /// compiler-generated rather than source text. The result owns its buffers
    /// and allocates host compiler memory as emitted text grows.
    pub(super) fn new(header: String) -> Self {
        Self {
            header,
            slots: Vec::new(),
            body: Vec::new(),
            current_label: None,
            next_value: 0,
            next_label: 0,
        }
    }

    /// Generate a unique SSA name without adding an instruction.
    ///
    /// The returned name includes `%` and remains valid only in this function.
    /// A caller can reserve a name before emitting a phi's forward-referenced
    /// loop operand. It owns emitting that name's single definition.
    pub(super) fn fresh_value(&mut self) -> String {
        let name = format!("%v{}", self.next_value);
        self.next_value += 1;
        name
    }

    /// Append an instruction that produces a value and return its SSA name.
    ///
    /// `instruction` omits the result assignment. The caller is responsible for
    /// LLVM typing and control-flow legality; LLVM verification checks the output.
    pub(super) fn value(&mut self, instruction: impl Into<String>) -> String {
        let name = self.fresh_value();
        self.line(format!("{name} = {}", instruction.into()));
        name
    }

    /// Add one fixed stack allocation at entry and return its pointer SSA name.
    ///
    /// `ty` is a compiler-chosen LLVM type. Hoisting the allocation preserves its
    /// lifetime for all loop iterations; source scopes still govern usable values.
    pub(super) fn slot(&mut self, ty: &str) -> String {
        let name = self.fresh_value();
        self.slots.push(format!("  {name} = alloca {ty}"));
        name
    }

    /// Append one indented LLVM instruction or comment with no produced value.
    ///
    /// The operation mutates only the compiler's output buffer, never runtime data.
    pub(super) fn line(&mut self, instruction: impl Into<String>) {
        self.body.push(format!("  {}", instruction.into()));
    }

    /// Emit an unchanged guard, optionally favoring its established success edge.
    ///
    /// `condition` is the existing i1 predicate and both labels retain their original
    /// destinations. `success_when_true` is None for ineligible producers; Some
    /// selects the expected-success orientation without inverting/reordering code.
    /// No assumption, runtime operation, source promise or guard removal is added.
    pub(super) fn construction_branch(
        &mut self,
        condition: &str,
        if_true: &str,
        if_false: &str,
        success_when_true: Option<bool>,
    ) {
        // An absent hint reproduces the original branch text, keeping borrowed-
        // input producer decisions outside this candidate's intended scope.
        let metadata = match success_when_true {
            Some(true) => ", !prof !0",
            Some(false) => ", !prof !1",
            None => "",
        };
        self.line(format!(
            "br i1 {condition}, label %{if_true}, label %{if_false}{metadata}"
        ));
    }

    /// Generate a unique block label for the caller's next control-flow region.
    ///
    /// The result omits `%` so it can appear both as a definition and a target.
    pub(super) fn fresh_label(&mut self) -> String {
        let name = format!("block{}", self.next_label);
        self.next_label += 1;
        name
    }

    /// Begin the named basic block after the preceding block's terminator.
    ///
    /// `label` must have been generated by this builder. It does not add a branch;
    /// explicit edges keep cleanup and failure transitions visible to emitters.
    pub(super) fn label(&mut self, label: &str) {
        self.current_label = Some(self.body.len());
        self.body.push(format!("{label}:"));
    }

    /// Borrow the current block name for a phi edge after nested expressions.
    pub(super) fn current_label(&self) -> &str {
        self.current_label.map_or("entry", |index| {
            self.body[index]
                .strip_suffix(':')
                .expect("recorded block label")
        })
    }

    /// Finish the owned LLVM definition, placing all fixed slots at entry.
    ///
    /// Callers must have terminated every block. This consumes compiler buffers;
    /// it does not perform LLVM optimization or any runtime work.
    pub(super) fn finish(self) -> String {
        let mut text = format!("{} {{\nentry:\n", self.header);
        // Each source slot is emitted once before control flow can revisit it.
        for slot in self.slots {
            text.push_str(&slot);
            text.push('\n');
        }
        // Instructions retain the exact sequencing established by the emitter.
        for instruction in self.body {
            text.push_str(&instruction);
            text.push('\n');
        }
        text.push_str("}\n");
        text
    }
}
