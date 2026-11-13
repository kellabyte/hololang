//! Independent, safe values for the region oracle and its fixture byte accounting.
//!
//! The model deliberately stores nested values as Rust-owned vectors rather than
//! using native region offsets. This makes expected values independent of the
//! compiler's storage implementation. Sizes use mathematical `u128` arithmetic;
//! these test allocations and scans make no statement about generated-code costs.

/// A published field value in the independent semantic model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// An initialized, fixed-width unsigned scalar.
    Scalar(u64),
    /// Owned initialized payload bytes; the vector length is semantic metadata.
    Bytes(Vec<u8>),
    /// An inline nested record represented independently as owned Rust values.
    Record(Record),
}

/// A named field whose position records construction/declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The source field name used for model projections and diagnostics.
    pub name: String,
    /// The initialized value; unfinished fields are not inserted here.
    pub value: Value,
}

/// A fully initialized semantic record without pointers into native backing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Size header or positive empty-unit prefix, independently chosen from logical schema.
    pub leading_bytes: u8,
    /// Frozen payload-length width used by byte fields in this record.
    pub length_bytes: u8,
    /// Initialized fields in declaration order, preserving the fixture layout order.
    pub fields: Vec<Field>,
}

impl Value {
    /// Return this initialized value's bytes under its containing record's length width.
    ///
    /// The result includes payload length or independently encoded nested records. It is an
    /// independent mathematical accounting result, not a native memory access.
    pub fn encoded_len(&self, length_bytes: u8) -> u128 {
        // Each variant carries distinct fixture metadata, so account for its
        // actual representation instead of relying on Rust's enum/object sizes.
        match self {
            Self::Scalar(_) => 8,
            Self::Bytes(bytes) => u128::from(length_bytes) + bytes.len() as u128,
            Self::Record(record) => record.encoded_len(),
        }
    }
}

impl Record {
    /// Construct a model record from initialized ordered fields without copying.
    ///
    /// `fields` transfers ownership; leading_bytes and length_bytes are independent
    /// schema/mode results, never native offsets. The caller supplies complete
    /// fields and valid widths; this function has no fallible native allocation.
    pub fn new(fields: Vec<Field>, leading_bytes: u8, length_bytes: u8) -> Self {
        Self {
            leading_bytes,
            length_bytes,
            fields,
        }
    }

    /// Derive the record's encoded size including its independent leading storage.
    ///
    /// This scans model fields and uses `u128`; it intentionally does not reuse
    /// the compiler's incremental bump-pointer reservation algorithm.
    pub fn encoded_len(&self) -> u128 {
        u128::from(self.leading_bytes)
            + self
                .fields
                .iter()
                .map(|field| field.value.encoded_len(self.length_bytes))
                .sum::<u128>()
    }

    /// Find an initialized field and its byte offset from this record's start.
    ///
    /// Returns `None` for an absent name. Offset accounting includes the record
    /// selected prefix and all preceding semantic fields, independent of emitter helpers.
    pub fn field(&self, name: &str) -> Option<(&Value, u128)> {
        let mut offset = u128::from(self.leading_bytes);
        // Visit fields in declaration order so a projection's offset reflects
        // every preceding inline value, including variable-sized payloads.
        for field in &self.fields {
            // A matching field starts at the accumulated offset; later fields
            // cannot influence this initialized projection's position.
            if field.name == name {
                return Some((&field.value, offset));
            }
            offset += field.value.encoded_len(self.length_bytes);
        }
        None
    }

    /// Find a mutable initialized field, preserving the model record's shape.
    ///
    /// Returns `None` for an absent name. Evaluator writes must still restrict
    /// replacement to same-shaped scalar/byte contents rather than resizing data.
    pub fn field_mut(&mut self, name: &str) -> Option<&mut Value> {
        // Only the selected semantic field is exposed; no native pointers or
        // compiler metadata participate in this test-model mutation.
        self.fields
            .iter_mut()
            .find(|field| field.name == name)
            .map(|field| &mut field.value)
    }
}
