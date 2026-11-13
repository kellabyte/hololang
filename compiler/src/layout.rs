//! Checked, schema-aware physical storage planning for explicit layout experiments.
//!
//! The wide control preserves the original representation. Derived64 removes
//! record-size headers where a fixed extent or one variable tail supplies a cheap
//! step. Compact32 uses the same policy with explicit checked u32 byte lengths.
//! Plans allocate compiler maps only; no runtime descriptor, relocation, or owner
//! machinery is added. More complex shapes retain inspectable size headers.

use crate::{
    check::CheckedProgram,
    ir::{FieldKind, Record},
};
use std::{collections::HashMap, fmt};

/// Explicit experimental physical policy, independent of reference permissions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutMode {
    /// Original eight-byte record headers and eight-byte byte lengths. This is
    /// the library compatibility default; the source CLI defaults to Derived64.
    #[default]
    Wide,
    /// Derive eligible record sizes while retaining the full u64 length domain.
    Derived64,
    /// Derive eligible sizes with checked u32 payload lengths; no dynamic fallback.
    Compact32,
}

impl LayoutMode {
    /// Return the stable experimental CLI spelling without allocation or mutation.
    pub fn name(self) -> &'static str {
        match self {
            Self::Wide => "wide",
            Self::Derived64 => "derived",
            Self::Compact32 => "compact",
        }
    }
}

/// Inspectable reason for keeping or omitting a record's total-size header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderReason {
    /// The caller selected the preserved wide control.
    WideControl,
    /// Every field has a compile-time-known physical size.
    FixedSize,
    /// A single variable-sized final field determines the remaining extent.
    VariableTail,
    /// Several variable-sized fields would need additional metadata traversal.
    MultipleVariableFields,
    /// The conservative first planner keeps a header for a variable non-tail field.
    VariableBeforeSuffix,
}

impl fmt::Display for HeaderReason {
    /// Explain this compile-time decision for inspection; no program state changes.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WideControl => "preserved wide control",
            Self::FixedSize => "fixed schema extent",
            Self::VariableTail => "single variable tail",
            Self::MultipleVariableFields => {
                "multiple variable fields: stored size avoids a metadata walk"
            }
            Self::VariableBeforeSuffix => "variable non-tail field: conservative stored size",
        })
    }
}

/// One schema's packed physical representation, derived only from checked fields.
#[derive(Clone, Debug)]
pub struct RecordLayout {
    /// Stored total-size prefix: eight bytes or zero, never a user-writable field.
    pub header_bytes: u64,
    /// One initialized representation byte for an otherwise zero-sized empty record.
    /// This preserves positive batch cursor progress without per-record headers.
    pub unit_bytes: u64,
    /// Exact compile-time size when all component sizes fit and are fixed.
    pub fixed_size: Option<u64>,
    /// Exact zero-payload size in derived modes. Wide overflow cases conservatively
    /// report just their eight-byte header as a proven lower bound.
    pub minimum_size: u64,
    /// The planner's explicit decision, so retained headers are never hidden.
    pub reason: HeaderReason,
}

/// An admitted logical schema whose requested experimental extent is unrepresentable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutError {
    /// Schema responsible for a static extent overflow.
    record: String,
    /// Owned diagnostic explaining the unsupported physical plan.
    message: String,
}

impl fmt::Display for LayoutError {
    /// Render schema identity and failure without hiding a fallback representation.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "layout of {}: {}", self.record, self.message)
    }
}

impl std::error::Error for LayoutError {}

/// Immutable per-schema physical facts for one checked program and selected mode.
#[derive(Debug)]
pub struct LayoutPlan {
    /// Explicit policy; widths do not change in response to runtime input.
    mode: LayoutMode,
    /// Completed schema plans, keyed by checked unique identity.
    records: HashMap<String, RecordLayout>,
}

impl LayoutPlan {
    /// Plan checked acyclic schemas bottom-up, allocating only compiler metadata.
    /// Wide always remains available; derived static overflow is a clear error.
    /// An iterative dependency walk avoids making source nesting a host stack limit.
    pub fn new(checked: &CheckedProgram<'_>, mode: LayoutMode) -> Result<Self, LayoutError> {
        let definitions: HashMap<_, _> = checked
            .program()
            .records
            .iter()
            .map(|record| (record.name.as_str(), record))
            .collect();
        let mut plan = Self {
            mode,
            records: HashMap::new(),
        };
        // The checker proves acyclicity; explicit exit events wait for all child plans.
        for root in &checked.program().records {
            let mut pending = vec![(root, false)];
            while let Some((record, exiting)) = pending.pop() {
                // Shared child schemas are computed once, independent of declaration order.
                if plan.records.contains_key(&record.name) {
                    continue;
                }
                if !exiting {
                    pending.push((record, true));
                    for field in record.fields.iter().rev() {
                        if let FieldKind::Record(name) = &field.kind {
                            pending.push((definitions[name.as_str()], false));
                        }
                    }
                } else {
                    let layout = plan.plan_record(record)?;
                    plan.records.insert(record.name.clone(), layout);
                }
            }
        }
        Ok(plan)
    }

    /// Borrow facts for a known schema from this plan's checked program.
    /// Unknown identities violate this API precondition; no runtime lookup is emitted.
    pub fn record(&self, name: &str) -> &RecordLayout {
        self.records
            .get(name)
            .expect("layout requested for a checked known schema")
    }

    /// Return the selected global byte-length width in bytes, without mutation.
    pub fn length_bytes(&self) -> u64 {
        // Compact is an explicit finite-width contract, never an automatic truncation.
        if self.mode == LayoutMode::Compact32 {
            4
        } else {
            8
        }
    }

    /// Return the explicit policy associated with these immutable layout facts.
    pub fn mode(&self) -> LayoutMode {
        self.mode
    }

    /// Compute one record after its nested schemas, preserving cheap size stepping.
    /// Runtime-sized fields affect eligibility but never cause dynamic layout switching.
    fn plan_record(&self, record: &Record) -> Result<RecordLayout, LayoutError> {
        let mut variable = Vec::new();
        // A Bytes field is variable; a record is fixed exactly when its child plan is fixed.
        for (index, field) in record.fields.iter().enumerate() {
            if matches!(&field.kind, FieldKind::Bytes)
                || matches!(&field.kind, FieldKind::Record(name) if self.record(name).fixed_size.is_none())
            {
                variable.push(index);
            }
        }
        let reason = if self.mode == LayoutMode::Wide {
            HeaderReason::WideControl
        } else if variable.is_empty() {
            HeaderReason::FixedSize
        } else if variable.len() > 1 {
            HeaderReason::MultipleVariableFields
        } else if variable[0] + 1 == record.fields.len() {
            HeaderReason::VariableTail
        } else {
            HeaderReason::VariableBeforeSuffix
        };
        let header_bytes = match reason {
            // Fixed and single-tail layouts derive their step without scanning payload bytes.
            HeaderReason::FixedSize | HeaderReason::VariableTail => 0,
            _ => 8,
        };
        let unit_bytes = u64::from(header_bytes == 0 && record.fields.is_empty());
        let mut minimum = Some(header_bytes + unit_bytes);
        let mut fixed = Some(header_bytes + unit_bytes);
        // Checked additions reject impossible static layouts before native pointer formation.
        for field in &record.fields {
            let (field_minimum, field_fixed) = match &field.kind {
                FieldKind::U64 => (8, Some(8)),
                FieldKind::Bytes => (self.length_bytes(), None),
                FieldKind::Record(name) => {
                    let child = self.record(name);
                    (child.minimum_size, child.fixed_size)
                }
            };
            minimum = minimum.and_then(|size| size.checked_add(field_minimum));
            fixed = fixed.and_then(|size| field_fixed.and_then(|field| size.checked_add(field)));
        }
        // Wide is the original control, including schemas too large to construct.
        // Derived plans reject static overflow instead of emitting wrapped constants.
        if minimum.is_none() && self.mode != LayoutMode::Wide {
            return Err(LayoutError {
                record: record.name.clone(),
                message:
                    "minimum physical extent exceeds u64; requested derived layout is unsupported"
                        .to_owned(),
            });
        }
        Ok(RecordLayout {
            header_bytes,
            unit_bytes,
            fixed_size: fixed,
            minimum_size: minimum.unwrap_or(8),
            reason,
        })
    }
}

/// Describe one explicit plan, including symbolic offsets and retained-header reasons.
/// This check-only inspection requires no LLVM installation or native execution.
pub fn inspect(checked: &CheckedProgram<'_>, mode: LayoutMode) -> Result<String, LayoutError> {
    let plan = LayoutPlan::new(checked, mode)?;
    let mut text = format!(
        "layout {}: byte lengths {} bits; packed alignment 1\n",
        mode.name(),
        plan.length_bytes() * 8
    );
    // Declaration order gives deterministic diagnostics independent of HashMap iteration.
    for record in &checked.program().records {
        let layout = plan.record(&record.name);
        let extent = layout
            .fixed_size
            .map_or_else(|| "dynamic".to_owned(), |size| size.to_string());
        text.push_str(&format!(
            "{}: header={} unit={} fixed={} minimum={} ({})\n",
            record.name,
            layout.header_bytes,
            layout.unit_bytes,
            extent,
            layout.minimum_size,
            layout.reason
        ));
        let mut offset = layout.header_bytes.to_string();
        for field in &record.fields {
            let size = match &field.kind {
                FieldKind::U64 => "8".to_owned(),
                FieldKind::Bytes => format!("{} + length({})", plan.length_bytes(), field.name),
                FieldKind::Record(name) => plan
                    .record(name)
                    .fixed_size
                    .map_or_else(|| format!("size({})", field.name), |size| size.to_string()),
            };
            text.push_str(&format!(
                "  {}: offset={} bytes; extent={}\n",
                field.name, offset, size
            ));
            // Fold known offsets for readability; dynamic/overflowing extents stay explicit.
            offset = match (offset.parse::<u64>(), size.parse::<u64>()) {
                (Ok(left), Ok(right)) if left.checked_add(right).is_some() => {
                    (left + right).to_string()
                }
                _ => format!("{offset} + ({size})"),
            };
        }
    }
    Ok(text)
}
