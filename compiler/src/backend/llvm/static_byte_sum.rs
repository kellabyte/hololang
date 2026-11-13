//! Exact-width call-site specialization for a checked pure byte sum.
//!
//! A literal logical prefix establishes the extent only after its ordinary
//! eager bounds check. The callee must consist of one private wrapping sum,
//! with no observable work beyond its scalar result. Tiny widths use inline
//! loads and large widths use an outlined helper. Other widths keep their
//! ordinary call. The selection adds no runtime width dispatch or storage.

use crate::ir::{BytesSource, Expr, Function, FunctionArgument, FunctionParameterKind, Op};

use super::{byte_sum, function::Function as NativeFunction};

/// Source of the initial scalar total after original arguments are captured.
#[derive(Clone, Copy)]
pub(super) enum Seed {
    /// Initializer is a compile-time unsigned scalar.
    Literal(u64),
    /// Initializer reads the eager scalar actual at this formal position.
    Scalar(usize),
}

/// Facts that permit replacing one ordinary call after capture.
#[derive(Clone, Copy)]
pub(super) struct StaticSum {
    /// Byte formal whose captured checked extent the callee consumes.
    pub(super) input: usize,
    /// Exact proven prefix width, tiny or large enough for the wide helper.
    pub(super) width: u64,
    /// Source of the result's starting total.
    pub(super) seed: Seed,
}

/// Recognize one checked callee and the literal prefix at its relevant actual.
///
/// This inspects compiler-owned structure only. A rejected shape continues
/// through ordinary lowering. Selection leaves all eager validation in place.
pub(super) fn select(
    function: &Function,
    args: &[FunctionArgument],
    helpers: &byte_sum::Helpers,
) -> Option<StaticSum> {
    // Reject any extra observable work, including writes, calls, or a different
    // return. The existing loop matcher proves the body reads one exact formal.
    let [
        Op::Let { name, value },
        Op::Repeat { count, index, body },
        Op::Return(Expr::Local(result)),
    ] = function.body.as_slice()
    else {
        return None;
    };
    let (accumulator, BytesSource::Param(input)) = byte_sum::recognize(count, index, body)? else {
        return None;
    };
    if name != result || name != accumulator {
        return None;
    }
    // The literal denotes the logical span, independent of the backing field's
    // published extent. The caller still checks it before evaluating later args.
    let FunctionArgument::Bytes {
        prefix: Some(Expr::Const(width)),
        ..
    } = args.get(input)?
    else {
        return None;
    };
    // Every intermediate width without a wide proof retains its checked call.
    if *width > 2 && !helpers.selects_wide(*width) {
        return None;
    }
    let seed = match value {
        Expr::Const(value) => Seed::Literal(*value),
        Expr::Local(name) => Seed::Scalar(function.params.iter().position(|parameter| {
            parameter.name == *name && parameter.kind == FunctionParameterKind::U64
        })?),
        // A computed or trapping initializer belongs to the ordinary callee.
        _ => return None,
    };
    Some(StaticSum {
        input,
        width: *width,
        seed,
    })
}

/// Emit the exact selected load and wrapping result after all actuals finish.
///
/// `contents` is the original borrowed location, `seed` an eager scalar operand.
/// Empty input returns the seed without a payload load. A two-byte load uses
/// alignment one, and summing both halves is endian-independent. A wide input
/// passes the checked constant width to its outlined helper with no new branch.
pub(super) fn emit(
    f: &mut NativeFunction,
    helpers: &byte_sum::Helpers,
    width: u64,
    contents: &str,
    seed: String,
) -> String {
    // Width selection happens in the compiler. Every nonempty case has a
    // dominating original prefix check for the complete initialized load.
    let partial = match width {
        0 => return seed,
        1 => {
            let byte = f.value(format!("load i8, ptr {contents}, align 1"));
            f.value(format!("zext i8 {byte} to i64"))
        }
        2 => {
            let word = f.value(format!("load i16, ptr {contents}, align 1"));
            let low = f.value(format!("and i16 {word}, 255"));
            let high = f.value(format!("lshr i16 {word}, 8"));
            let sum = f.value(format!("add nuw i16 {low}, {high}"));
            f.value(format!("zext i16 {sum} to i64"))
        }
        width if helpers.selects_wide(width) => {
            let helper = helpers.select(true);
            return f.value(format!(
                "call i64 @{helper}(ptr {contents}, i64 {width}, i64 {seed})"
            ));
        }
        _ => unreachable!("only proved tiny or large widths select this lowering"),
    };
    f.value(format!("add i64 {seed}, {partial}"))
}
