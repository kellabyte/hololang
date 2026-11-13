//! Immutable byte constants shared across the generated functions of one module.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

/// Compiler-side table of exact byte contents and deterministic native identities.
#[derive(Default)]
pub(super) struct ByteLiterals {
    ids: RefCell<HashMap<Arc<[u8]>, usize>>,
}

impl ByteLiterals {
    /// Return a static byte address and its logical extent without emitting instructions.
    pub(super) fn capture(&self, bytes: &Arc<[u8]>) -> (String, String) {
        let mut ids = self.ids.borrow_mut();
        let index = if let Some(index) = ids.get(bytes.as_ref()) {
            *index
        } else {
            let index = ids.len();
            ids.insert(Arc::clone(bytes), index);
            index
        };
        (
            format!("@holo.byte_literal.{index}"),
            bytes.len().to_string(),
        )
    }

    /// Emit constants in first-use order, escaping every byte as LLVM hex.
    pub(super) fn emit(&self, module: &mut String) {
        let ids = self.ids.borrow();
        let mut literals = vec![&[][..]; ids.len()];
        for (bytes, &index) in ids.iter() {
            literals[index] = bytes.as_ref();
        }
        for (index, bytes) in literals.into_iter().enumerate() {
            if bytes.is_empty() {
                // Give the zero-length descriptor an address backed by one byte.
                writeln!(
                    module,
                    "@holo.byte_literal.{index} = private unnamed_addr constant [1 x i8] zeroinitializer"
                )
                .expect("writing to a String");
                continue;
            }
            write!(
                module,
                "@holo.byte_literal.{index} = private unnamed_addr constant [{} x i8] c\"",
                bytes.len()
            )
            .expect("writing to a String");
            for byte in bytes {
                write!(module, "\\{byte:02X}").expect("writing to a String");
            }
            module.push_str("\"\n");
        }
    }
}
