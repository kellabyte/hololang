//! Independent mathematical representation rules for explicit layout experiments.
//!
//! This test oracle shares only the selected mode label with the compiler. It
//! never reads a compiler LayoutPlan, field offset, or size helper. Recursive
//! schema inspection derives eligibility from logical fields, with a bounded
//! active path to reject malformed cycles before exhausting the host stack.

use super::ModelError;
use holo::ir::{FieldKind, Program};
use std::collections::BTreeMap;

/// Test-model layout selection, independent of compiler plan implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Encoding {
    /// Every record has an eight-byte size and every payload an eight-byte length.
    Wide,
    /// Eligible size headers are removed; byte lengths retain eight bytes.
    Derived64,
    /// Eligible size headers are removed; byte lengths use checked four-byte counts.
    Compact32,
}

/// Independent shape facts, not a native memory descriptor.
#[derive(Clone, Copy)]
struct Shape {
    /// Whether some descendant contains variable-size bytes.
    variable: bool,
    /// Whether this record can omit its total-size header under the experiment.
    derivable: bool,
}

impl Encoding {
    /// Return the selected payload-length width without interpreting native bytes.
    pub fn length_bytes(self) -> u8 {
        // Only the explicitly compact experiment narrows metadata; wide domains remain unchanged.
        match self {
            Self::Compact32 => 4,
            Self::Wide | Self::Derived64 => 8,
        }
    }

    /// Derive this schema's leading storage from logical fields alone.
    /// Empty headerless records retain one initialized unit byte for forward progress.
    pub fn leading_bytes(self, program: &Program, name: &str) -> Result<u8, ModelError> {
        // The preserved control intentionally carries the original header on every schema.
        if self == Self::Wide {
            return Ok(8);
        }
        let record = program
            .records
            .iter()
            .find(|record| record.name == name)
            .ok_or_else(|| ModelError::new("unknown layout-model schema"))?;
        let shape = shape(program, name, &mut Vec::new(), &mut BTreeMap::new())?;
        // Complex records retain a header; empty units consume positive initialized storage.
        Ok(if !shape.derivable {
            8
        } else if record.fields.is_empty() {
            1
        } else {
            0
        })
    }
}

/// Inspect one logical schema with independent cycle/depth detection.
/// Each field contributes one variable flag; no payload bytes or compiler layout is read.
fn shape(
    program: &Program,
    name: &str,
    active: &mut Vec<String>,
    finished: &mut BTreeMap<String, Shape>,
) -> Result<Shape, ModelError> {
    // Cyclic/malicious test input is an oracle error, never a guessed compact representation.
    if active.len() >= 128 || active.iter().any(|item| item == name) {
        return Err(ModelError::new(
            "cyclic or excessive layout-model schema depth",
        ));
    }
    // Shared schema DAGs may have many repeated paths; completed facts avoid exponential inspection.
    if let Some(shape) = finished.get(name) {
        return Ok(*shape);
    }
    let record = program
        .records
        .iter()
        .find(|record| record.name == name)
        .ok_or_else(|| ModelError::new("unknown nested layout-model schema"))?;
    active.push(name.to_owned());
    let mut dynamic = Vec::new();
    // Field order determines whether the sole variable component is the final tail.
    for (position, field) in record.fields.iter().enumerate() {
        let nested = match &field.kind {
            FieldKind::U64 | FieldKind::Bool | FieldKind::Enum(_) => Shape {
                variable: false,
                derivable: true,
            },
            FieldKind::Bytes => Shape {
                variable: true,
                derivable: true,
            },
            FieldKind::Record(child) => shape(program, child, active, finished)?,
        };
        // A fixed nested record contributes bytes but does not create a variable-tail position.
        if nested.variable {
            dynamic.push(position);
        }
    }
    active.pop();
    let derivable =
        dynamic.is_empty() || (dynamic.len() == 1 && dynamic[0] + 1 == record.fields.len());
    let shape = Shape {
        variable: !dynamic.is_empty(),
        derivable,
    };
    finished.insert(name.to_owned(), shape);
    Ok(shape)
}
