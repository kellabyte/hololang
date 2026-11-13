//! Lower synchronous hosted socket calls, retaining the existing checked byte
//! range operations. Source owner names resolve to lexical IR identities.
//! Lowering allocates compiler nodes only; the checker validates live ownership
//! and the native adapter owns descriptor admission and close effects.

use super::{Block, Kind, Lowerer, arity, ast};
use crate::{
    ir,
    source::{Diagnostic, Span},
};

impl Lowerer {
    /// Resolve the writable range that a hosted read can fill in place.
    pub(super) fn refill_destination(
        &self,
        operand: &ast::Expr,
    ) -> Result<ir::BytesSource, Diagnostic> {
        if matches!(operand.kind, ast::ExprKind::Bytes(_)) {
            return Err(Diagnostic::new(
                operand.span,
                "byte string literals are readonly",
            ));
        } else if let ast::ExprKind::Name(name) = &operand.kind {
            let binding = self.lookup(name, operand.span)?;
            match binding.kind {
                Kind::BytesView(ir::Access::Readwrite) => {
                    return Ok(ir::BytesSource::Local(binding.ir_name));
                }
                Kind::BytesView(ir::Access::Readonly) | Kind::ReceivedBytes => {
                    return Err(Diagnostic::new(
                        operand.span,
                        "refill requires a writable destination; this byte view is readonly",
                    ));
                }
                _ => {}
            }
        } else if let ast::ExprKind::Member { .. } = operand.kind {
            let (reference, path, kind) = self.projection(operand)?;
            if kind == ir::FieldKind::Bytes {
                return Ok(ir::BytesSource::Field { reference, path });
            }
        }
        Err(Diagnostic::new(
            operand.span,
            "expected a readwrite Bytes field or named writable byte view for refill",
        ))
    }

    /// Resolve an initialized field, named byte view, or literal for a synchronous send.
    fn send_source(&self, operand: &ast::Expr) -> Result<ir::BytesSource, Diagnostic> {
        if let ast::ExprKind::Bytes(bytes) = &operand.kind {
            return Ok(ir::BytesSource::Literal(bytes.clone()));
        } else if let ast::ExprKind::Name(name) = &operand.kind {
            let binding = self.lookup(name, operand.span)?;
            if matches!(binding.kind, Kind::BytesView(_)) {
                return Ok(ir::BytesSource::Local(binding.ir_name));
            }
        } else if let ast::ExprKind::Member { .. } = operand.kind {
            let (reference, path, kind) = self.projection(operand)?;
            if kind == ir::FieldKind::Bytes {
                return Ok(ir::BytesSource::Field { reference, path });
            }
        }
        Err(Diagnostic::new(
            operand.span,
            "core.send requires an initialized Bytes field, a named byte view, or a byte string literal",
        ))
    }

    /// Resolve a lexical TCP binding by name, returning only its IR identity.
    /// An unknown name or different binding kind has no socket authority.
    pub(super) fn tcp_name(&self, name: &str, span: Span) -> Result<String, Diagnostic> {
        let binding = self.lookup(name, span)?;
        // Scalars and other owners cannot serve as file descriptors.
        if !matches!(binding.kind, Kind::TcpStream) {
            return Err(Diagnostic::new(span, "expected a TCP stream binding"));
        }
        Ok(binding.ir_name)
    }

    /// Resolve a call's explicit owner operand without evaluating an expression.
    /// A computed value cannot introduce, copy, or transfer socket ownership.
    fn tcp_operand(&self, operand: &ast::Expr) -> Result<String, Diagnostic> {
        // Only a directly named lexical binding can select a stream.
        let ast::ExprKind::Name(name) = &operand.kind else {
            return Err(Diagnostic::new(
                operand.span,
                "expected a TCP stream binding name",
            ));
        };
        self.tcp_name(name, operand.span)
    }

    /// Attach an explicit socket owner to a transfer. Host-owned calls keep
    /// their original IR. The wrapper and operation share a checker span.
    pub(super) fn tcp_io(stream: Option<String>, operation: ir::Op) -> ir::Op {
        // Only an explicit owner operand adds this compiler-only wrapper.
        match stream {
            Some(stream) => ir::Op::TcpIo {
                stream,
                operation: Box::new(operation),
            },
            None => operation,
        }
    }

    /// Lower a hosted immutable status declaration. The source result name is
    /// introduced after its operands resolve. Byte range expressions stay in
    /// the native operation so source capture still precedes their evaluation.
    pub(super) fn hosted_status_binding(
        &mut self,
        operation: &str,
        args: &[ast::Expr],
        name: &str,
        binding_span: Span,
        call_span: Span,
    ) -> Result<Block, Diagnostic> {
        let mut output = Block::default();
        let status = Kind::Scalar { mutable: false };
        // Send accepts the existing three range operands, with a lexical
        // owner as the first operand only in the four-argument form.
        let operation = match operation {
            "send" => {
                let (stream, range) = if let [owner, range @ ..] = args
                    && range.len() == 3
                {
                    (Some(self.tcp_operand(owner)?), range)
                } else {
                    arity(args, 3, call_span)?;
                    (None, args)
                };
                let source = self.send_source(&range[0])?;
                let previous = std::mem::replace(&mut self.deferred_actual, true);
                let operands = (|| {
                    let offset = self.scalar(&range[1], &mut output)?;
                    let count = self.scalar(&range[2], &mut output)?;
                    Ok::<_, Diagnostic>((offset, count))
                })();
                self.deferred_actual = previous;
                let (offset, count) = operands?;
                assert!(output.ops.is_empty(), "send range must remain expressions");
                let result = self.bind(name, status, binding_span)?;
                Self::tcp_io(
                    stream,
                    ir::Op::Send {
                        source,
                        offset,
                        count,
                        result,
                    },
                )
            }
            "wait_input" | "wait_output" => {
                // Zero arguments keep the host-owned readiness form.
                // One argument selects a checked lexical stream.
                let stream = if let [owner] = args {
                    Some(self.tcp_operand(owner)?)
                } else {
                    arity(args, 0, call_span)?;
                    None
                };
                let result = self.bind(name, status, binding_span)?;
                let wait = if operation == "wait_input" {
                    ir::Op::WaitInput { result }
                } else {
                    ir::Op::WaitOutput { result }
                };
                Self::tcp_io(stream, wait)
            }
            "close" => {
                // The IR verifier consumes the named owner. The result
                // represents terminal status and cannot yield an owner.
                arity(args, 1, call_span)?;
                let stream = self.tcp_operand(&args[0])?;
                let result = self.bind(name, status, binding_span)?;
                ir::Op::TcpClose { stream, result }
            }
            "next_session" => {
                // Only the preexisting native session path can select a next client.
                arity(args, 0, call_span)?;
                let result = self.bind(name, status, binding_span)?;
                ir::Op::NextSession { result }
            }
            _ => unreachable!("caller selects a hosted status operation"),
        };
        output.push(operation, binding_span, []);
        Ok(output)
    }
}
