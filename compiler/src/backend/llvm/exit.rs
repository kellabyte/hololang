//! Entry completion and loop transfers with explicit reverse lexical cleanup.
//!
//! Each exit restores active tail markers and releases regions and any TCP
//! owner, without scanning objects or adding runtime bookkeeping. An unreachable
//! continuation permits the structured emitter to finish sibling paths and scope
//! endings; that continuation has no executable incoming edge.

use super::{
    context::{Cleanup, Lowering},
    storage,
};

impl Lowering<'_> {
    /// End entry execution successfully after cleaning every active owner.
    ///
    /// No arguments or return value are carried. Compiler-side cleanup records
    /// remain intact for separately emitted sibling branches. At runtime each
    /// active marker is restored and each acquired base or accepted descriptor
    /// is released exactly once. Following instructions cannot execute from it.
    pub(super) fn exit_zero(&mut self) {
        self.emit_cleanups(0);
        // Hosted input returns to its caller. Direct entry retains
        // the process exit used by ordinary compiled source.
        if self.hosted_input {
            self.f.line("ret i32 0");
        } else {
            self.f.line("call void @exit(i32 0)");
            self.f.line("unreachable");
        }
        // This block has no predecessor from the exit. Ordinary structured code
        // can append its terminators without resuming or repeating runtime cleanup.
        let continuation = self.f.fresh_label();
        self.f.label(&continuation);
    }

    /// Clean up the scopes crossed by one loop transfer and branch to its target.
    pub(super) fn loop_transfer(&mut self, exits: bool) {
        let targets = self.loops.last().expect("checked enclosing loop");
        let target = if exits {
            targets.exit.clone()
        } else {
            targets.advance.clone()
        };
        self.emit_cleanups(targets.cleanup_depth);
        self.f.line(format!("br label %{target}"));
        // The structured emitter can finish the old path in a detached block.
        let continuation = self.f.fresh_label();
        self.f.label(&continuation);
    }

    /// Emit cleanup from innermost scope to the retained lexical depth.
    fn emit_cleanups(&mut self, depth: usize) {
        for index in (depth..self.cleanups.len()).rev() {
            match self.cleanups[index].clone() {
                Cleanup::Tcp => self.release_tcp(),
                Cleanup::Region { base } => self.f.line(format!("call void @free(ptr {base})")),
                Cleanup::Tail { region, start } => storage::restore(&mut self.f, &region, &start),
            }
        }
    }
}
