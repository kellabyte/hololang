//! Physical plan decisions and inspection, independently checked against small schemas.
//!
//! These tests inspect real checked programs without executing native code. Native
//! storage/failure conformance is separate. Expected constants are hand-calculated,
//! not obtained from backend size/projection helpers. Large logical schemas exercise
//! checked compiler arithmetic without attempting a huge allocation.

use holo::{
    check,
    ir::{Field, FieldKind, Program, Record},
    layout::{self, HeaderReason, LayoutMode, LayoutPlan},
};

/// Build one named schema with owned ordered fields; compiler allocations only.
fn record(name: &str, fields: &[(&str, FieldKind)]) -> Record {
    Record {
        name: name.into(),
        fields: fields
            .iter()
            .map(|(name, kind)| Field {
                name: (*name).into(),
                kind: kind.clone(),
            })
            .collect(),
    }
}

/// Supply fixed, tail-variable, and deliberately complex schemas for plan decisions.
/// Declaration order puts a parent first to test dependency planning, not source order.
fn fixture() -> Program {
    Program {
        records: vec![
            record(
                "Entry",
                &[
                    ("id", FieldKind::U64),
                    ("payload", FieldKind::Record("Payload".into())),
                ],
            ),
            record("Empty", &[]),
            record("Point", &[("x", FieldKind::U64), ("y", FieldKind::U64)]),
            record("Payload", &[("data", FieldKind::Bytes)]),
            record("Mixed", &[("a", FieldKind::Bytes), ("b", FieldKind::Bytes)]),
            record(
                "NonTail",
                &[("data", FieldKind::Bytes), ("tail", FieldKind::U64)],
            ),
            record(
                "Wrapper",
                &[
                    ("head", FieldKind::U64),
                    ("mixed", FieldKind::Record("Mixed".into())),
                ],
            ),
            record(
                "Fixed",
                &[
                    ("empty", FieldKind::Record("Empty".into())),
                    ("point", FieldKind::Record("Point".into())),
                ],
            ),
        ],
        ..Program::default()
    }
}

#[test]
/// Derive both benchmark size prefixes while preserving the full u64 length width.
fn full_width_derivation_removes_only_eligible_headers() {
    let program = fixture();
    let checked = check::check(&program).unwrap();
    let wide = LayoutPlan::new(&checked, LayoutMode::Wide).unwrap();
    let derived = LayoutPlan::new(&checked, LayoutMode::Derived64).unwrap();
    assert_eq!(wide.record("Entry").minimum_size, 32);
    assert_eq!(derived.record("Entry").minimum_size, 16);
    assert_eq!(derived.length_bytes(), 8);
    assert_eq!(derived.record("Entry").header_bytes, 0);
    assert_eq!(derived.record("Entry").reason, HeaderReason::VariableTail);
    assert_eq!(derived.record("Mixed").header_bytes, 8);
    assert_eq!(
        derived.record("Mixed").reason,
        HeaderReason::MultipleVariableFields
    );
    assert_eq!(
        derived.record("NonTail").reason,
        HeaderReason::VariableBeforeSuffix
    );
    assert_eq!(derived.record("Wrapper").header_bytes, 0);
    assert_eq!(derived.record("Wrapper").minimum_size, 32);
}

#[test]
/// An explicit compact policy changes byte lengths, never application scalar width.
fn compact_width_and_positive_fixed_extents_are_inspectable() {
    let program = fixture();
    let checked = check::check(&program).unwrap();
    let plan = LayoutPlan::new(&checked, LayoutMode::Compact32).unwrap();
    assert_eq!(plan.mode(), LayoutMode::Compact32);
    assert_eq!(plan.length_bytes(), 4);
    assert_eq!(plan.record("Entry").minimum_size, 12);
    assert_eq!(plan.record("Mixed").minimum_size, 16);
    assert_eq!(plan.record("NonTail").minimum_size, 20);
    assert_eq!(plan.record("Empty").fixed_size, Some(1));
    assert_eq!(plan.record("Empty").unit_bytes, 1);
    assert_eq!(plan.record("Point").fixed_size, Some(16));
    assert_eq!(plan.record("Fixed").fixed_size, Some(17));
}

#[test]
/// Recursive storage totals reject overflow before emitting wrapped native constants.
fn impossible_derived_static_sizes_reject_without_breaking_wide_control() {
    let mut program = Program::default();
    program
        .records
        .push(record("R0", &[("value", FieldKind::U64)]));
    // Each distinct type embeds two complete children; 64 levels exceed u64 bytes.
    for level in 1..64 {
        let child = FieldKind::Record(format!("R{}", level - 1));
        program.records.push(record(
            &format!("R{level}"),
            &[("left", child.clone()), ("right", child)],
        ));
    }
    let checked = check::check(&program).unwrap();
    assert!(LayoutPlan::new(&checked, LayoutMode::Wide).is_ok());
    let error = LayoutPlan::new(&checked, LayoutMode::Derived64).unwrap_err();
    assert!(error.to_string().contains("exceeds u64"));
    assert!(LayoutPlan::new(&checked, LayoutMode::Compact32).is_err());
}

#[test]
/// Inspection reports stable declaration order, useful offsets, and explicit retained headers.
fn inspection_explains_the_physical_plan() {
    let program = fixture();
    let checked = check::check(&program).unwrap();
    let description = layout::inspect(&checked, LayoutMode::Compact32).unwrap();
    assert!(description.starts_with("layout compact: byte lengths 32 bits; packed alignment 1"));
    assert!(description.contains("Entry: header=0 unit=0 fixed=dynamic minimum=12"));
    assert!(description.contains("payload: offset=8 bytes"));
    assert!(description.contains("multiple variable fields"));
    assert!(description.contains("Empty: header=0 unit=1 fixed=1 minimum=1"));
}
