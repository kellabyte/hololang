//! Focused definition-lowering acceptance and compiler-resource regression checks.
//!
//! These tests parse real experimental source before inspecting/checking emitted
//! IR. They establish adapter rejection/representation behavior, not native
//! performance. Expansion tests bound compiler work without timing assertions.

use super::{Definitions, instantiate, lower};
use crate::frontend::parse;
use crate::ir;
use crate::source::{Diagnostic, Span};

/// Parse and lower one source fixture, leaving checker/native execution to each test.
fn definitions(source: &str) -> Result<Definitions, Diagnostic> {
    lower(&parse::parse(source)?)
}

/// Require rejection and retain its explanation for focused assertions.
fn rejection(source: &str) -> Diagnostic {
    match definitions(source) {
        Ok(_) => panic!("expected definition rejection"),
        Err(error) => error,
    }
}

/// Forward scalar calls resolve closed pure contracts and produce valid parameterized IR.
#[test]
fn forward_pure_helpers_resolve() {
    let source = "func first\nwith noalloc, noblock\narg x u64 as readonly copy\nreturns u64\n{ return second(x) }\nfunc second\narg y u64 as readonly copy\nreturns u64\n{ return core.add_wrapping(y, 1) }";
    let lowered = definitions(source).unwrap();
    assert_eq!(lowered.scalars.len(), 2);
    assert_eq!(lowered.scalars["first"].params, 1);
    assert!(matches!(
        lowered.scalars["first"].value,
        ir::Expr::Binary { .. }
    ));
}

/// Fresh nested literals and named calls remain destination producers with valid byte indexes.
#[test]
fn nested_destinations_and_index_helpers_check() {
    let source = "struct Inner { value u64 }\nstruct Outer {\nfirst Inner\nsecond Inner\npayload Bytes\n}\nfunc tweak\nwith noalloc, noblock\narg x u64 as readonly copy\nreturns u64\n{ return core.xor(x, 7) }\nfunc inner\nwith noblock\narg seed u64 as readonly copy\nreturns Inner\n{ return Inner { value: seed } }\nfunc outer\nwith noblock\narg length u64 as readonly copy\narg seed u64 as readonly copy\nreturns Outer\n{\ncore.reject_if(length > 100)\nreturn Outer {\nfirst: Inner { value: tweak(seed) }\nsecond: inner(seed)\npayload: core.bytes(length, tweak(core.index()))\n}\n}";
    let lowered = definitions(source).unwrap();
    assert_eq!(lowered.producers.len(), 3);
    assert!(
        lowered
            .producers
            .iter()
            .any(|producer| producer.name.starts_with("$literal."))
    );
    let program = ir::Program {
        records: lowered.records,
        producers: lowered.producers,
        body: Vec::new(),
    };
    crate::check::check(&program).unwrap();
}

/// Byte inputs keep their full typed slots through named and synthetic nested calls.
/// The safe checker still authorizes the emitted unchecked definitions separately.
#[test]
fn readonly_byte_inputs_preserve_typed_nested_forwarding() {
    let source = r#"
struct Inner { data Bytes }
struct Outer { id u64, first Inner, second Inner }
func identity arg value u64 as readonly copy returns u64 { return value }
func child arg input Bytes as readonly reference returns Inner {
    return Inner { data: core.bytes(core.len(input), input[core.index()]) }
}
func parent
    with noblock
    arg left Bytes as readonly reference
    arg tag u64 as readonly copy
    arg right Bytes as readonly reference
    returns Outer
{
    core.reject_if(core.len(left) > tag)
    return Outer {
        id: identity(left[0])
        first: Inner {
            data: core.bytes(core.len(left), identity(left[core.index()]))
        }
        second: child(right)
    }
}
"#;
    let lowered = definitions(source).unwrap();
    let parent = lowered
        .producers
        .iter()
        .find(|producer| producer.name == "parent")
        .unwrap();
    let kinds = vec![
        ir::Parameter::Bytes,
        ir::Parameter::U64,
        ir::Parameter::Bytes,
    ];
    assert_eq!(parent.params, kinds);
    let literal = lowered
        .producers
        .iter()
        .find(|producer| producer.name.starts_with("$literal."))
        .unwrap();
    assert_eq!(literal.params, kinds);
    assert!(parent.body.iter().any(|step| {
        matches!(step, ir::Build::Record { producer, args, .. }
            if producer == "child"
                && matches!(args.as_slice(), [ir::Argument::Bytes(ir::BytesSource::Param(2))]))
    }));
    let program = ir::Program {
        records: lowered.records,
        producers: lowered.producers,
        body: Vec::new(),
    };
    crate::check::check(&program).unwrap();
}

/// Readonly bytes can enter ordinary calls, while pure and constructor boundaries stay typed.
#[test]
fn readonly_byte_inputs_keep_distinct_call_categories() {
    let ordinary = definitions(
        "func length arg input Bytes as readonly reference returns u64 { return core.len(input) }",
    )
    .expect("readonly byte formals select the ordinary function category");
    assert!(matches!(
        ordinary.ordinary["length"].as_slice(),
        [ir::FunctionParameterKind::Bytes]
    ));
    assert!(!ordinary.scalars.contains_key("length"));
    let cases = [
        (
            "func length arg input Bytes as readonly reference returns u64 { return core.len(input) }\nfunc bad returns u64 { return length(0) }",
            "cannot be called from a pure helper",
        ),
        (
            "struct R { data Bytes }\nfunc bad arg input Bytes as readonly copy returns R { return R { data: core.bytes(0, 0) } }",
            "Bytes as readonly reference",
        ),
        (
            "struct R { data Bytes }\nfunc bad arg input Bytes as readwrite reference returns R { return R { data: core.bytes(0, 0) } }",
            "Bytes as readonly reference",
        ),
        (
            "struct R { value u64 }\nfunc bad arg input Bytes as readonly reference returns R { return R { value: input } }",
            "not a scalar value",
        ),
        (
            "struct R { data Bytes }\nfunc bad arg input Bytes as readonly reference returns R { return R { data: input } }",
            "requires `core.bytes",
        ),
        (
            "struct R { data Bytes }\nstruct O { child R }\nfunc child arg input Bytes as readonly reference returns R { return R { data: core.bytes(core.len(input), 0) } }\nfunc bad arg value u64 as readonly copy returns O { return O { child: child(value) } }",
            "expected a readonly Bytes input parameter",
        ),
        (
            "struct R { data Bytes }\nfunc bad with noalloc arg input Bytes as readonly reference returns R { return R { data: core.bytes(core.len(input), 0) } }",
            "fresh dynamic destination storage",
        ),
    ];
    // Each declaration is checked even without an entry caller, so unused effects/types cannot hide.
    for (source, expected) in cases {
        let error = rejection(source);
        assert!(error.message.contains(expected), "{}", error.message);
    }
}

/// Byte-bearing producers eagerly capture trapping helper actuals even if unused or repeated.
/// This inspects the general scoped-expression IR; root native tests exercise the trap itself.
#[test]
fn byte_input_helper_actuals_are_eager_and_not_duplicated() {
    let source = r#"
struct R { ignored u64, doubled u64 }
func ignore arg value u64 as readonly copy returns u64 { return 0 }
func twice arg value u64 as readonly copy returns u64 {
    return core.add_wrapping(value, value)
}
func make arg input Bytes as readonly reference returns R {
    return R { ignored: ignore(input[999]), doubled: twice(input[0]) }
}
"#;
    let lowered = definitions(source).unwrap();
    let producer = lowered
        .producers
        .iter()
        .find(|producer| producer.name == "make")
        .unwrap();
    let [
        ir::Build::U64 { value: ignored, .. },
        ir::Build::U64 { value: doubled, .. },
    ] = producer.body.as_slice()
    else {
        panic!("expected two scalar destination initializers");
    };
    assert!(matches!(ignored, ir::Expr::Apply { args, body }
        if matches!(args.as_slice(), [ir::Expr::InputByte { param: 0, .. }])
            && matches!(**body, ir::Expr::Const(0))));
    assert!(matches!(doubled, ir::Expr::Apply { args, body }
        if matches!(args.as_slice(), [ir::Expr::InputByte { param: 0, .. }])
            && matches!(&**body, ir::Expr::Binary { left, right, .. }
                if matches!(**left, ir::Expr::Param(0)) && matches!(**right, ir::Expr::Param(0)))));
    let program = ir::Program {
        records: lowered.records,
        producers: lowered.producers,
        body: Vec::new(),
    };
    crate::check::check(&program).unwrap();
}

/// Unused declarations cannot hide unresolved calls, mismatched arity, or recursive helpers.
#[test]
fn all_helpers_are_validated() {
    let cases = [
        (
            "func unused\nreturns u64\n{ return missing() }",
            "unknown function",
        ),
        (
            "func unused\nreturns u64\n{ return core.add_wrapping(1) }",
            "expects 2",
        ),
        (
            "func unused\nreturns u64\n{ return unused() }",
            "recursive function",
        ),
        (
            "func first\nreturns u64\n{ return second() }\nfunc second\nreturns u64\n{ return first() }",
            "recursive function",
        ),
    ];
    // Each independent invalid graph must fail before entry lowering or native emission.
    for (source, expected) in cases {
        assert!(rejection(source).message.contains(expected));
    }
}

/// Explicit argument contracts never silently change into references, mutable values, or defaults.
#[test]
fn headers_and_contracts_reject_unsupported_modes() {
    let cases = [
        (
            "func bad\narg x u64 as readwrite copy\nreturns u64\n{ return x }",
            "readonly copy",
        ),
        (
            "func bad\narg x u64 as readonly reference\nreturns u64\n{ return x }",
            "readonly copy",
        ),
        (
            "func bad\narg x u64 as readonly copy\narg x u64 as readonly copy\nreturns u64\n{ return x }",
            "duplicate argument",
        ),
        (
            "func bad\nwith fast\nreturns u64\n{ return 0 }",
            "unknown function contract",
        ),
        (
            "func bad\nwith noalloc, noalloc\nreturns u64\n{ return 0 }",
            "duplicate function contract",
        ),
        (
            "func bad\nreturns Missing\n{ return 0 }",
            "unknown return type",
        ),
        (
            "func same\nreturns u64\n{ return 0 }\nfunc same\nreturns u64\n{ return 1 }",
            "duplicate function",
        ),
    ];
    // Verify each mode/error independently so unsupported syntax cannot accidentally become a promise.
    for (source, expected) in cases {
        assert!(rejection(source).message.contains(expected));
    }
}

/// Scalars reject unknown core calls, contextual-index escape, infix bitwise operators, and extra effects.
#[test]
fn pure_expression_boundaries_are_explicit() {
    let cases = [
        (
            "func bad\nreturns u64\n{ return core.index() }",
            "only inside a byte initializer",
        ),
        (
            "func bad\nreturns u64\n{ return core.random() }",
            "unsupported pure core call",
        ),
        (
            "func bad\nreturns u64\n{ return 1 & 2 }",
            "infix bitwise operators are unsupported",
        ),
        (
            "func bad\nreturns u64\n{\nlet x = 1\nreturn x\n}",
            "exactly one pure",
        ),
    ];
    // Rejected forms never lower through foreign kernels or a more permissive secondary path.
    for (source, expected) in cases {
        assert!(rejection(source).message.contains(expected));
    }
}

/// Schema/initializer mismatches are source failures rather than reordered or copied constructions.
#[test]
fn fields_types_and_order_are_preserved() {
    let cases = [
        ("struct Bad { value Missing }", "unknown field type"),
        ("struct Bad { value Bad }", "recursive inline struct"),
        ("struct Bad {\nx u64\nx u64\n}", "duplicate field"),
        (
            "struct R {\na u64\nb u64\n}\nfunc make\nreturns R\n{ return R { b: 1, a: 2 } }",
            "declaration order",
        ),
        (
            "struct R { x u64 }\nfunc make\nreturns R\n{ return R {} }",
            "exactly 1 fields",
        ),
        (
            "struct R { bytes Bytes }\nfunc make\nreturns R\n{ return R { bytes: 3 } }",
            "requires `core.bytes",
        ),
        (
            "struct R { x u64 }\nfunc make\nreturns R\n{ return other() }",
            "unknown constructor `other`",
        ),
        (
            "struct R { x u64 }\nfunc make\nreturns R\n{ return existing }",
            "returning an existing aggregate is unsupported",
        ),
    ];
    // Each fixture isolates one shape violation, including schemas with no users.
    for (source, expected) in cases {
        let error = rejection(source);
        assert!(error.message.contains(expected), "{source}: {error:?}");
    }
}

/// Even fixed-field constructors allocate fresh region storage under the strict contract.
#[test]
fn constructor_noalloc_is_not_no_malloc() {
    let error =
        rejection("struct R { x u64 }\nfunc make\nwith noalloc\nreturns R\n{ return R { x: 1 } }");
    assert!(error.message.contains("fresh dynamic destination storage"));
    assert_eq!(
        &"struct R { x u64 }\nfunc make\nwith noalloc\nreturns R\n{ return R { x: 1 } }"
            [error.span.start..error.span.end],
        "noalloc"
    );
}

/// Substitution counts repeated parameters before cloning and preserves its budget on failure.
#[test]
fn instantiate_preflights_duplicate_parameters() {
    let template = ir::Expr::binary(
        ir::Binary::AddWrapping,
        ir::Expr::Param(0),
        ir::Expr::Param(0),
    );
    let argument = ir::Expr::binary(ir::Binary::Xor, ir::Expr::Const(1), ir::Expr::Const(2));
    let mut budget = 6;
    let error = instantiate(
        &template,
        &[argument],
        Span { start: 2, end: 4 },
        &mut budget,
    )
    .unwrap_err();
    assert!(error.message.contains("node budget"));
    assert_eq!(budget, 6);
    let mut budget = 3;
    let result = instantiate(
        &template,
        &[ir::Expr::Param(7)],
        Span { start: 0, end: 0 },
        &mut budget,
    )
    .unwrap();
    assert!(
        matches!(result, ir::Expr::Binary { left, right, .. } if matches!(*left, ir::Expr::Param(7)) && matches!(*right, ir::Expr::Param(7)))
    );
    assert_eq!(budget, 0);
}

/// A malformed template is diagnosed without indexing beyond the supplied actual arguments.
#[test]
fn instantiate_rejects_missing_parameters() {
    let mut budget = 10;
    assert!(
        instantiate(
            &ir::Expr::Param(1),
            &[ir::Expr::Const(0)],
            Span { start: 0, end: 0 },
            &mut budget
        )
        .unwrap_err()
        .message
        .contains("invalid parameter index")
    );
    assert_eq!(budget, 10);
}

/// Recursive input indexes consume the same substitution budget as other expression children.
#[test]
fn instantiate_counts_borrowed_input_index_children() {
    let input = ir::Expr::InputByte {
        param: 7,
        index: Box::new(ir::Expr::binary(
            ir::Binary::AddWrapping,
            ir::Expr::Const(1),
            ir::Expr::Const(2),
        )),
    };
    let mut budget = 3;
    let error = instantiate(
        &ir::Expr::Param(0),
        std::slice::from_ref(&input),
        Span { start: 0, end: 0 },
        &mut budget,
    )
    .unwrap_err();
    assert!(error.message.contains("node budget"));
    assert_eq!(budget, 3);
    let mut budget = 4;
    let result = instantiate(
        &ir::Expr::Param(0),
        &[input],
        Span { start: 0, end: 0 },
        &mut budget,
    )
    .unwrap();
    assert!(matches!(result, ir::Expr::InputByte { param: 7, .. }));
    assert_eq!(budget, 0);
}

/// Substitution accounts for complete Apply bodies without capturing their fresh parameters.
#[test]
fn instantiate_preserves_scoped_scalar_parameter_frames() {
    let template = ir::Expr::Apply {
        args: vec![ir::Expr::Param(0)],
        body: Box::new(ir::Expr::binary(
            ir::Binary::AddWrapping,
            ir::Expr::Param(0),
            ir::Expr::Param(0),
        )),
    };
    let argument = ir::Expr::binary(ir::Binary::Xor, ir::Expr::Const(1), ir::Expr::Const(2));
    let mut budget = 7;
    let result = instantiate(
        &template,
        &[argument],
        Span { start: 0, end: 0 },
        &mut budget,
    )
    .unwrap();
    assert!(matches!(result, ir::Expr::Apply { args, body }
        if matches!(args.as_slice(), [ir::Expr::Binary { .. }])
            && matches!(&*body, ir::Expr::Binary { left, right, .. }
                if matches!(**left, ir::Expr::Param(0)) && matches!(**right, ir::Expr::Param(0)))));
    assert_eq!(budget, 0);
}

/// Exponentially duplicated source helpers stop at the compiler-node budget.
#[test]
fn repeated_helper_expansion_is_bounded() {
    let mut source =
        String::from("func f0\narg x u64 as readonly copy\nreturns u64\n{ return x }\n");
    // Each successive helper doubles its previous tree, exceeding the budget with small source.
    for index in 1..25 {
        source.push_str(&format!("func f{index}\narg x u64 as readonly copy\nreturns u64\n{{ return core.add_wrapping(f{}(x), f{}(x)) }}\n", index - 1, index - 1));
    }
    assert!(rejection(&source).message.contains("budget"));
}

/// Cached templates cannot evade the bound by acquiring one surrounding operation per helper.
#[test]
fn combined_cached_helper_depth_is_bounded() {
    let mut source =
        String::from("func f0\narg x u64 as readonly copy\nreturns u64\n{ return x }\n");
    // Source-order completion caches each dependency before its shallow caller is processed.
    for index in 1..70 {
        source.push_str(&format!("func f{index}\narg x u64 as readonly copy\nreturns u64\n{{ return core.add_wrapping(f{}(x), 1) }}\n", index - 1));
    }
    assert!(rejection(&source).message.contains("depth limit of 64"));
}

/// Finished schemas retain their heights so declaration order cannot hide excessive nesting.
#[test]
fn cached_schema_depth_is_bounded() {
    let mut source = String::from("struct S0 { value u64 }\n");
    // Each child has already been fully resolved when its parent becomes a new DFS root.
    for index in 1..70 {
        source.push_str(&format!("struct S{index} {{ child S{} }}\n", index - 1));
    }
    assert!(rejection(&source).message.contains("depth limit of 64"));
}
