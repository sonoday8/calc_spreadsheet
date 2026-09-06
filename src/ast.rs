//! Formula AST: single structure for evaluation and reference analysis.

use std::collections::HashSet;

use crate::error::SpreadsheetError;
use crate::excel_date;
use crate::functions::{bool_to_number, eval_eager_function, is_truthy};
use crate::refs::{
    FormulaRefs, a1_range_size, canonical_cell_name, for_each_a1_range, format_a1,
    is_cell_name_char, is_cell_name_start, parse_a1,
};
use crate::spreadsheet::{CellKind, EvalContext};

/// Intermediate evaluation value (numbers and numeric arrays).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EvalValue {
    Number(f64),
    /// Row-major rectangle: `rows[r][c]`.
    Array(Vec<Vec<f64>>),
}

impl EvalValue {
    #[allow(clippy::wrong_self_convention)]
    fn as_scalar(self) -> Result<f64, SpreadsheetError> {
        match self {
            EvalValue::Number(n) => Ok(n),
            EvalValue::Array(rows) => rows
                .first()
                .and_then(|row| row.first())
                .copied()
                .ok_or(SpreadsheetError::Value),
        }
    }

    fn map_numbers(
        self,
        mut f: impl FnMut(f64) -> Result<f64, SpreadsheetError>,
    ) -> Result<EvalValue, SpreadsheetError> {
        match self {
            EvalValue::Number(n) => Ok(EvalValue::Number(f(n)?)),
            EvalValue::Array(rows) => {
                let mut out = Vec::with_capacity(rows.len());
                for row in rows {
                    let mut new_row = Vec::with_capacity(row.len());
                    for n in row {
                        new_row.push(f(n)?);
                    }
                    out.push(new_row);
                }
                Ok(EvalValue::Array(out))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Number(f64),
    Cell(String),
    /// A1 range kept as endpoints until expand (collect / array eval / aggregate flatten).
    Range {
        start: String,
        end: String,
    },
    /// Spill reference `A1#` — whole dynamic array from anchor.
    SpillRef(String),
    /// Implicit intersection `@expr` (Excel `@`).
    Intersect(Box<Expr>),
    Str(String),
    Neg(Box<Expr>),
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    CmpOp {
        op: CmpOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmpOp {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
}

pub(crate) fn parse_formula(expression: &str) -> Result<Expr, SpreadsheetError> {
    let expression = expression.trim().strip_prefix('=').unwrap_or(expression.trim());
    let mut parser = AstParser {
        chars: expression.chars().collect(),
        position: 0,
        expression,
    };
    let expr = parser.parse_comparison()?;
    parser.skip_whitespace();
    if parser.position < parser.chars.len() {
        return Err(SpreadsheetError::InvalidFormula(expression.to_string()));
    }
    Ok(expr)
}

#[cfg(test)]
pub(crate) fn analyze_formula_refs(expression: &str) -> Result<FormulaRefs, SpreadsheetError> {
    analyze_from_ast_in_sheet(
        &parse_formula(expression)?,
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        None,
    )
}

/// Analyze formula refs, pruning IF/IFS/SWITCH arms when conditions fold via `asts`.
///
/// `occupied` is the set of input cell names. When non-empty, cells absent from both
/// `asts` and `occupied` fold as blank (`0`). An empty set disables blank-folding
/// (standalone formula analysis).
///
/// `eval_cell` is the formula's own A1 name when known (enables FILTER self-spill ref exclusion).
pub(crate) fn analyze_from_ast_in_sheet(
    expr: &Expr,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    eval_cell: Option<&str>,
) -> Result<FormulaRefs, SpreadsheetError> {
    let mut refs = FormulaRefs::default();
    let mut visiting = std::collections::HashSet::new();
    let ignore = HashSet::new();
    collect_refs_in_sheet(
        expr,
        asts,
        occupied,
        eval_cell,
        &mut visiting,
        &mut refs,
        &ignore,
    )?;
    Ok(refs)
}

/// True when this formula may spill from a `FILTER` — top-level or nested under
/// passthrough wrappers (`IFERROR` / `IF` / `IFS` / `SWITCH` / `LET`), with the
/// same const-arm pruning as spill shape analysis (dead arms do not count).
pub(crate) fn expr_is_filter_spill_source(
    expr: &Expr,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
) -> bool {
    fn walk(
        expr: &Expr,
        asts: &std::collections::HashMap<String, Expr>,
        occupied: &std::collections::HashSet<String>,
        visiting: &mut std::collections::HashSet<String>,
    ) -> bool {
        match expr {
            Expr::Call { name, args } => match name.to_uppercase().as_str() {
                "FILTER" => true,
                "IFERROR" if !args.is_empty() => {
                    if walk(&args[0], asts, occupied, visiting) {
                        return true;
                    }
                    // Match spill-shape: when primary has a known array footprint,
                    // fallback is not the static spill source.
                    let primary_spills = expr_spill_shape_in_sheet(&args[0], asts, occupied)
                        .ok()
                        .flatten()
                        .is_some();
                    if primary_spills {
                        return false;
                    }
                    args.get(1)
                        .is_some_and(|fb| walk(fb, asts, occupied, visiting))
                }
                "IF" if args.len() == 3 => {
                    match const_truthy_in_sheet(&args[0], asts, occupied, visiting) {
                        Some(true) => walk(&args[1], asts, occupied, visiting),
                        Some(false) => walk(&args[2], asts, occupied, visiting),
                        None => {
                            walk(&args[1], asts, occupied, visiting)
                                || walk(&args[2], asts, occupied, visiting)
                        }
                    }
                }
                "IFS" => {
                    let mut uncertain = false;
                    for pair in args.chunks(2) {
                        if pair.len() != 2 {
                            break;
                        }
                        match const_truthy_in_sheet(&pair[0], asts, occupied, visiting) {
                            Some(true) if !uncertain => {
                                return walk(&pair[1], asts, occupied, visiting);
                            }
                            Some(false) if !uncertain => {}
                            _ => {
                                uncertain = true;
                                if walk(&pair[1], asts, occupied, visiting) {
                                    return true;
                                }
                            }
                        }
                    }
                    false
                }
                "SWITCH" if args.len() > 1 => {
                    let target = const_number_in_sheet(&args[0], asts, occupied, visiting);
                    let rest = &args[1..];
                    let mut uncertain = target.is_none();
                    let mut i = 0;
                    while i + 1 < rest.len() {
                        match (
                            target,
                            const_number_in_sheet(&rest[i], asts, occupied, visiting),
                        ) {
                            (Some(t), Some(m)) if !uncertain && t == m => {
                                return walk(&rest[i + 1], asts, occupied, visiting);
                            }
                            (Some(t), Some(m)) if !uncertain && t != m => {}
                            _ => {
                                uncertain = true;
                                if walk(&rest[i + 1], asts, occupied, visiting) {
                                    return true;
                                }
                            }
                        }
                        i += 2;
                    }
                    if rest.len() % 2 == 1 {
                        if !uncertain && target.is_some() {
                            return walk(&rest[rest.len() - 1], asts, occupied, visiting);
                        }
                        return walk(&rest[rest.len() - 1], asts, occupied, visiting);
                    }
                    false
                }
                "LET" => match expand_let_for_analysis(args) {
                    Some(expanded) => walk(&expanded, asts, occupied, visiting),
                    None => false,
                },
                _ => false,
            },
            Expr::SpillRef(anchor) => {
                if !visiting.insert(anchor.clone()) {
                    return false;
                }
                let hit = asts
                    .get(anchor)
                    .is_some_and(|inner| walk(inner, asts, occupied, visiting));
                visiting.remove(anchor);
                hit
            }
            // Do not walk BinOp/Neg/CmpOp: `SEQUENCE(..)+FILTER(..)` must stay
            // fail-closed for spill↔ref cycles (soft-skip is FILTER-as-root only).
            _ => false,
        }
    }
    let mut visiting = std::collections::HashSet::new();
    walk(expr, asts, occupied, &mut visiting)
}

pub(crate) fn eval_expr(
    expr: &Expr,
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<f64, SpreadsheetError> {
    eval_value(expr, ctx, source)?.as_scalar()
}

pub(crate) fn eval_value(
    expr: &Expr,
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<EvalValue, SpreadsheetError> {
    match expr {
        Expr::Number(n) => Ok(EvalValue::Number(*n)),
        Expr::Cell(name) => {
            if let Some(bound) = ctx.lookup_binding(name) {
                return Ok(bound.clone());
            }
            Ok(EvalValue::Number(ctx.lookup_number(name)?))
        }
        Expr::Range { start, end } => eval_range_value(start, end, ctx),
        Expr::SpillRef(anchor) => eval_spill_ref(anchor, ctx),
        Expr::Intersect(inner) => Ok(EvalValue::Number(eval_intersect(inner, ctx, source)?)),
        Expr::Str(_) => Err(SpreadsheetError::Value),
        Expr::Neg(inner) => eval_value(inner, ctx, source)?.map_numbers(|n| Ok(-n)),
        Expr::BinOp { op, left, right } => {
            let l = eval_value(left, ctx, source)?;
            let r = eval_value(right, ctx, source)?;
            zip_values(l, r, |a, b| apply_binop(*op, a, b))
        }
        Expr::CmpOp { op, left, right } => {
            let l = eval_value(left, ctx, source)?;
            let r = eval_value(right, ctx, source)?;
            zip_values(l, r, |a, b| Ok(bool_to_number(apply_cmp(*op, a, b))))
        }
        Expr::Call { name, args } => eval_call_value(name, args, ctx, source),
    }
}

/// `LET(name1, value1, …, calculation)` — last arg is the result; names bind locally.
fn let_calculation(args: &[Expr]) -> Option<&Expr> {
    if args.len() < 3 || args.len().is_multiple_of(2) {
        return None;
    }
    args.last()
}

fn let_binding_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Cell(name) => Some(name.clone()),
        Expr::Str(name) => Some(canonical_cell_name(name)),
        _ => None,
    }
}

/// Substitute `LET` bindings into the calculation for spill / ref / soft-skip analysis.
fn expand_let_for_analysis(args: &[Expr]) -> Option<Expr> {
    let calc = let_calculation(args)?;
    let pairs = &args[..args.len() - 1];
    let mut bound = std::collections::HashMap::new();
    for pair in pairs.chunks(2) {
        if pair.len() != 2 {
            return None;
        }
        let name = let_binding_name(&pair[0])?;
        bound.insert(name, &pair[1]);
    }
    Some(substitute_let_bindings(calc, &bound))
}

fn substitute_let_bindings(
    expr: &Expr,
    bound: &std::collections::HashMap<String, &Expr>,
) -> Expr {
    match expr {
        Expr::Cell(name) => {
            if let Some(replacement) = bound.get(name) {
                substitute_let_bindings(replacement, bound)
            } else {
                Expr::Cell(name.clone())
            }
        }
        Expr::Neg(inner) => Expr::Neg(Box::new(substitute_let_bindings(inner, bound))),
        Expr::Intersect(inner) => {
            Expr::Intersect(Box::new(substitute_let_bindings(inner, bound)))
        }
        Expr::BinOp { op, left, right } => Expr::BinOp {
            op: *op,
            left: Box::new(substitute_let_bindings(left, bound)),
            right: Box::new(substitute_let_bindings(right, bound)),
        },
        Expr::CmpOp { op, left, right } => Expr::CmpOp {
            op: *op,
            left: Box::new(substitute_let_bindings(left, bound)),
            right: Box::new(substitute_let_bindings(right, bound)),
        },
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| substitute_let_bindings(a, bound))
                .collect(),
        },
        Expr::Number(n) => Expr::Number(*n),
        Expr::Str(s) => Expr::Str(s.clone()),
        Expr::Range { start, end } => Expr::Range {
            start: start.clone(),
            end: end.clone(),
        },
        Expr::SpillRef(a) => Expr::SpillRef(a.clone()),
    }
}

/// Static result shape for spill footprint / dependency edges.
/// `None` = scalar (or unknown / erroring at eval). `Some((h, w))` with `h * w > 1`.
pub(crate) fn expr_spill_shape(expr: &Expr) -> Result<Option<(usize, usize)>, SpreadsheetError> {
    Ok(match expr {
        Expr::Number(_) | Expr::Cell(_) | Expr::Str(_) | Expr::Intersect(_) => None,
        Expr::SpillRef(_) => None, // resolved via sheet ASTs in lib
        Expr::Range { start, end } => {
            let (c1, r1) = parse_a1(start).ok_or(SpreadsheetError::Value)?;
            let (c2, r2) = parse_a1(end).ok_or(SpreadsheetError::Value)?;
            let (cmin, cmax) = if c1 <= c2 { (c1, c2) } else { (c2, c1) };
            let (rmin, rmax) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
            let height = (rmax - rmin + 1) as usize;
            let width = (cmax - cmin + 1) as usize;
            let _ = a1_range_size(start, end)?;
            if height == 1 && width == 1 {
                None
            } else {
                Some((height, width))
            }
        }
        Expr::Neg(inner) => expr_spill_shape(inner)?,
        Expr::BinOp { left, right, .. } | Expr::CmpOp { left, right, .. } => {
            merge_spill_shapes(expr_spill_shape(left)?, expr_spill_shape(right)?)
        }
        Expr::Call { name, args } => call_spill_shape(name, args)?,
    })
}

/// Resolve spill shape including `A1#` by following anchor formulas in `asts`.
pub(crate) fn expr_spill_shape_in_sheet(
    expr: &Expr,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
) -> Result<Option<(usize, usize)>, SpreadsheetError> {
    fn walk(
        expr: &Expr,
        asts: &std::collections::HashMap<String, Expr>,
        occupied: &std::collections::HashSet<String>,
        visiting: &mut std::collections::HashSet<String>,
    ) -> Result<Option<(usize, usize)>, SpreadsheetError> {
        match expr {
            Expr::SpillRef(anchor) => {
                if !visiting.insert(anchor.clone()) {
                    return Ok(None);
                }
                let shape = if let Some(inner) = asts.get(anchor) {
                    walk(inner, asts, occupied, visiting)?
                } else {
                    None
                };
                visiting.remove(anchor);
                Ok(shape)
            }
            Expr::Intersect(_) => Ok(None),
            Expr::Neg(inner) => walk(inner, asts, occupied, visiting),
            Expr::BinOp { left, right, .. } | Expr::CmpOp { left, right, .. } => {
                Ok(merge_spill_shapes(
                    walk(left, asts, occupied, visiting)?,
                    walk(right, asts, occupied, visiting)?,
                ))
            }
            Expr::Call { name, args } => {
                match name.to_uppercase().as_str() {
                    "SEQUENCE" => Ok(crate::dynamic_array::sequence_spill_shape_from_consts(
                        args
                            .first()
                            .and_then(|e| const_number_in_sheet(e, asts, occupied, visiting)),
                        args
                            .get(1)
                            .and_then(|e| const_number_in_sheet(e, asts, occupied, visiting)),
                    )),
                    "FILTER" | "SORT" | "UNIQUE" if !args.is_empty() => {
                        if name.eq_ignore_ascii_case("FILTER") {
                            filter_spill_shape_in_sheet(args, asts, occupied, visiting)
                        } else {
                            walk(&args[0], asts, occupied, visiting)
                        }
                    }
                    "IF" if args.len() == 3 => {
                        let known = const_truthy_in_sheet(&args[0], asts, occupied, visiting);
                        match known {
                            Some(true) => walk(&args[1], asts, occupied, visiting),
                            Some(false) => walk(&args[2], asts, occupied, visiting),
                            None => Ok(union_spill_shapes([
                                walk(&args[1], asts, occupied, visiting)?,
                                walk(&args[2], asts, occupied, visiting)?,
                            ])),
                        }
                    }
                    "IFERROR" if (1..=2).contains(&args.len()) => {
                        // Prefer primary footprint when known; unioning fallback
                        // arrays creates false spill↔ref cycles under fail-closed CR.
                        let primary = walk(&args[0], asts, occupied, visiting)?;
                        if primary.is_some() {
                            Ok(primary)
                        } else if let Some(fb) = args.get(1) {
                            walk(fb, asts, occupied, visiting)
                        } else {
                            Ok(None)
                        }
                    }
                    "IFS" => {
                        let mut acc = None;
                        let mut uncertain = false;
                        for pair in args.chunks(2) {
                            if pair.len() != 2 {
                                break;
                            }
                            match const_truthy_in_sheet(&pair[0], asts, occupied, visiting) {
                                Some(true) if !uncertain => {
                                    return walk(&pair[1], asts, occupied, visiting);
                                }
                                Some(false) if !uncertain => {}
                                _ => {
                                    uncertain = true;
                                    acc = union_spill_shapes([
                                        acc,
                                        walk(&pair[1], asts, occupied, visiting)?,
                                    ]);
                                }
                            }
                        }
                        Ok(acc)
                    }
                    "SWITCH" if !args.is_empty() => {
                        let target = const_number_in_sheet(&args[0], asts, occupied, visiting);
                        let rest = &args[1..];
                        let mut acc = None;
                        let mut uncertain = target.is_none();
                        let mut i = 0;
                        while i + 1 < rest.len() {
                            match (target, const_number_in_sheet(&rest[i], asts, occupied, visiting)) {
                                (Some(t), Some(m)) if !uncertain && t == m => {
                                    return walk(&rest[i + 1], asts, occupied, visiting);
                                }
                                (Some(t), Some(m)) if !uncertain && t != m => {}
                                _ => {
                                    uncertain = true;
                                    acc = union_spill_shapes([
                                        acc,
                                        walk(&rest[i + 1], asts, occupied, visiting)?,
                                    ]);
                                }
                            }
                            i += 2;
                        }
                        if rest.len() % 2 == 1 {
                            let default_shape = walk(&rest[rest.len() - 1], asts, occupied, visiting)?;
                            if !uncertain && target.is_some() {
                                return Ok(default_shape);
                            }
                            acc = union_spill_shapes([acc, default_shape]);
                        }
                        Ok(acc)
                    }
                    "LET" => match expand_let_for_analysis(args) {
                        Some(expanded) => walk(&expanded, asts, occupied, visiting),
                        None => Ok(None),
                    },
                    _ => expr_spill_shape(expr),
                }
            }
            _ => expr_spill_shape(expr),
        }
    }
    let mut visiting = std::collections::HashSet::new();
    walk(expr, asts, occupied, &mut visiting)
}

fn merge_spill_shapes(
    left: Option<(usize, usize)>,
    right: Option<(usize, usize)>,
) -> Option<(usize, usize)> {
    match (left, right) {
        (None, other) | (other, None) => other,
        (Some((h1, w1)), Some((h2, w2))) => {
            if h1 == h2 && w1 == w2 {
                Some((h1, w1))
            } else if h1 == 1 && w2 == 1 {
                // 1×N ⊗ M×1 → M×N
                Some((h2, w1))
            } else if w1 == 1 && h2 == 1 {
                // M×1 ⊗ 1×N → M×N
                Some((h1, w2))
            } else {
                // Mismatch → runtime `#VALUE!`; over-approx footprint for deps.
                Some((h1.max(h2), w1.max(w2)))
            }
        }
    }
}

/// Union of conditional / broadcast footprints (`None` contributes nothing).
fn union_spill_shapes(
    shapes: impl IntoIterator<Item = Option<(usize, usize)>>,
) -> Option<(usize, usize)> {
    shapes.into_iter().fold(None, merge_spill_shapes)
}

fn if_spill_shape_arms(
    cond: &Expr,
    then_shape: Option<(usize, usize)>,
    else_shape: Option<(usize, usize)>,
) -> Option<(usize, usize)> {
    match const_truthy(cond) {
        Some(true) => then_shape,
        Some(false) => else_shape,
        None => union_spill_shapes([then_shape, else_shape]),
    }
}

fn ifs_spill_shape_arms(
    args: &[Expr],
    shape_of: impl FnMut(&Expr) -> Result<Option<(usize, usize)>, SpreadsheetError>,
) -> Result<Option<(usize, usize)>, SpreadsheetError> {
    ifs_spill_shape_arms_known(args, const_truthy, shape_of)
}

fn ifs_spill_shape_arms_known(
    args: &[Expr],
    mut cond_of: impl FnMut(&Expr) -> Option<bool>,
    mut shape_of: impl FnMut(&Expr) -> Result<Option<(usize, usize)>, SpreadsheetError>,
) -> Result<Option<(usize, usize)>, SpreadsheetError> {
    let mut acc = None;
    let mut uncertain = false;
    for pair in args.chunks(2) {
        if pair.len() != 2 {
            break;
        }
        let arm = shape_of(&pair[1])?;
        match cond_of(&pair[0]) {
            Some(true) if !uncertain => return Ok(arm),
            Some(false) if !uncertain => {}
            _ => {
                uncertain = true;
                acc = union_spill_shapes([acc, arm]);
            }
        }
    }
    Ok(acc)
}

fn switch_spill_shape_arms(
    args: &[Expr],
    shape_of: impl FnMut(&Expr) -> Result<Option<(usize, usize)>, SpreadsheetError>,
) -> Result<Option<(usize, usize)>, SpreadsheetError> {
    switch_spill_shape_arms_known(args, const_number, shape_of)
}

fn switch_spill_shape_arms_known(
    args: &[Expr],
    mut number_of: impl FnMut(&Expr) -> Option<f64>,
    mut shape_of: impl FnMut(&Expr) -> Result<Option<(usize, usize)>, SpreadsheetError>,
) -> Result<Option<(usize, usize)>, SpreadsheetError> {
    let target = number_of(&args[0]);
    let rest = &args[1..];
    let mut acc = None;
    let mut uncertain = target.is_none();
    let mut i = 0;
    while i + 1 < rest.len() {
        let arm = shape_of(&rest[i + 1])?;
        match (target, number_of(&rest[i])) {
            (Some(t), Some(m)) if !uncertain && t == m => return Ok(arm),
            (Some(t), Some(m)) if !uncertain && t != m => {}
            _ => {
                uncertain = true;
                acc = union_spill_shapes([acc, arm]);
            }
        }
        i += 2;
    }
    if rest.len() % 2 == 1 {
        let default_shape = shape_of(&rest[rest.len() - 1])?;
        if !uncertain && target.is_some() {
            // No match among const arms → default only.
            return Ok(default_shape);
        }
        acc = union_spill_shapes([acc, default_shape]);
    }
    Ok(acc)
}

fn const_truthy(expr: &Expr) -> Option<bool> {
    const_number(expr).map(is_truthy)
}

fn const_truthy_in_sheet(
    expr: &Expr,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<bool> {
    const_number_in_sheet(expr, asts, occupied, visiting).map(is_truthy)
}

fn const_number(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Number(n) => Some(*n),
        Expr::Neg(inner) => const_number(inner).map(|n| -n),
        Expr::BinOp { op, left, right } => {
            let l = const_number(left)?;
            let r = const_number(right)?;
            apply_const_binop(*op, l, r)
        }
        Expr::CmpOp { op, left, right } => {
            let l = const_number(left)?;
            let r = const_number(right)?;
            Some(bool_to_number(apply_cmp(*op, l, r)))
        }
        Expr::Call { name, args } => const_call_number(name, args, &mut |e| const_number(e), None),
        _ => None,
    }
}

/// Fold `Cell` refs to numeric literals through `asts` (for `SEQUENCE(A1)` shapes).
/// When `occupied` is non-empty, cells absent from both `asts` and `occupied` are blank (`0`).
/// Occupied non-formula cells (e.g. text) do not fold. Empty `occupied` disables blank-folding.
fn const_number_in_sheet(
    expr: &Expr,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<f64> {
    match expr {
        Expr::Number(n) => Some(*n),
        Expr::Neg(inner) => const_number_in_sheet(inner, asts, occupied, visiting).map(|n| -n),
        Expr::BinOp { op, left, right } => {
            let l = const_number_in_sheet(left, asts, occupied, visiting)?;
            let r = const_number_in_sheet(right, asts, occupied, visiting)?;
            apply_const_binop(*op, l, r)
        }
        Expr::CmpOp { op, left, right } => {
            let l = const_number_in_sheet(left, asts, occupied, visiting)?;
            let r = const_number_in_sheet(right, asts, occupied, visiting)?;
            Some(bool_to_number(apply_cmp(*op, l, r)))
        }
        Expr::Cell(name) => {
            if !visiting.insert(name.clone()) {
                return None;
            }
            let value = if let Some(inner) = asts.get(name) {
                const_number_in_sheet(inner, asts, occupied, visiting)
            } else if !occupied.is_empty() && !occupied.contains(name) {
                Some(0.0) // blank outside the input sheet
            } else {
                None // text / unknown outside sheet context
            };
            visiting.remove(name);
            value
        }
        Expr::Call { name, args } => {
            let kind = |cell: &str| aggregate_cell_kind(cell, asts, occupied);
            const_call_number(
                name,
                args,
                &mut |e| const_number_in_sheet(e, asts, occupied, visiting),
                Some(&kind),
            )
        }
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AggregateCellKind {
    /// Missing from the sheet — Excel aggregates skip; arithmetic treats as 0.
    Blank,
    /// Present text cell — aggregates skip.
    Text,
    /// Formula or numeric input — must fold to participate.
    Value,
}

fn aggregate_cell_kind(
    name: &str,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
) -> AggregateCellKind {
    if !occupied.is_empty() && !occupied.contains(name) {
        AggregateCellKind::Blank
    } else if occupied.contains(name) && !asts.contains_key(name) {
        AggregateCellKind::Text
    } else {
        AggregateCellKind::Value
    }
}

fn apply_const_binop(op: BinOp, l: f64, r: f64) -> Option<f64> {
    match op {
        BinOp::Add => Some(l + r),
        BinOp::Sub => Some(l - r),
        BinOp::Mul => Some(l * r),
        BinOp::Div if r != 0.0 => Some(l / r),
        BinOp::Div => None,
    }
}

/// `Some(true)` if any include element is truthy; `Some(false)` if all fold false; else `None`.
fn const_filter_include_any_truthy(
    include: &Expr,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<bool> {
    match include {
        Expr::Range { start, end } => {
            fold_truthy_over_range(start, end, asts, occupied, visiting)
        }
        Expr::CmpOp { op, left, right } => match (left.as_ref(), right.as_ref()) {
            (Expr::Range { start, end }, scalar) => {
                let s = const_number_in_sheet(scalar, asts, occupied, visiting)?;
                fold_cmp_over_range(*op, start, end, true, s, asts, occupied, visiting)
            }
            (scalar, Expr::Range { start, end }) => {
                let s = const_number_in_sheet(scalar, asts, occupied, visiting)?;
                fold_cmp_over_range(*op, start, end, false, s, asts, occupied, visiting)
            }
            _ => const_number_in_sheet(include, asts, occupied, visiting).map(is_truthy),
        },
        other => const_number_in_sheet(other, asts, occupied, visiting).map(is_truthy),
    }
}

fn fold_truthy_over_range(
    start: &str,
    end: &str,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<bool> {
    let mut any = false;
    let mut ok = true;
    for_each_a1_range(start, end, |cell| {
        if !ok {
            return Ok(());
        }
        match const_number_in_sheet(&Expr::Cell(cell), asts, occupied, visiting) {
            Some(n) if is_truthy(n) => any = true,
            Some(_) => {}
            None => ok = false,
        }
        Ok(())
    })
    .ok()?;
    ok.then_some(any)
}

/// `range_on_left`: compare folded cell vs scalar (`cell op scalar`), else `scalar op cell`.
#[allow(clippy::too_many_arguments)]
fn fold_cmp_over_range(
    op: CmpOp,
    start: &str,
    end: &str,
    range_on_left: bool,
    scalar: f64,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<bool> {
    let mut any = false;
    let mut ok = true;
    for_each_a1_range(start, end, |cell| {
        if !ok {
            return Ok(());
        }
        match const_number_in_sheet(&Expr::Cell(cell), asts, occupied, visiting) {
            Some(v) => {
                let truth = if range_on_left {
                    apply_cmp(op, v, scalar)
                } else {
                    apply_cmp(op, scalar, v)
                };
                if truth {
                    any = true;
                }
            }
            None => ok = false,
        }
        Ok(())
    })
    .ok()?;
    ok.then_some(any)
}

/// FILTER spill footprint: known empty → if_empty; known non-empty → clipped array;
/// unknown → union(array, if_empty).
fn filter_spill_shape_in_sheet(
    args: &[Expr],
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Result<Option<(usize, usize)>, SpreadsheetError> {
    let array_shape = expr_spill_shape_in_sheet(&args[0], asts, occupied)?;
    let empty_shape = if let Some(fb) = args.get(2) {
        expr_spill_shape_in_sheet(fb, asts, occupied)?
    } else {
        None
    };
    let include = args.get(1);
    let clipped = include
        .map(|inc| clip_filter_array_shape(array_shape, inc, asts, occupied, visiting))
        .unwrap_or(array_shape);
    match include.and_then(|inc| const_filter_include_any_truthy(inc, asts, occupied, visiting)) {
        Some(true) => Ok(clipped),
        Some(false) => Ok(empty_shape),
        None => Ok(union_spill_shapes([clipped, empty_shape])),
    }
}

/// Upper-bound clip: max kept rows/cols ≤ include vector length (and not const-false slots).
fn clip_filter_array_shape(
    array_shape: Option<(usize, usize)>,
    include: &Expr,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<(usize, usize)> {
    let (h, w) = array_shape?;
    let Some((max_kept, filter_rows)) = include_max_kept(include, asts, occupied, visiting) else {
        return Some((h, w));
    };
    let (nh, nw) = if filter_rows {
        (h.min(max_kept.max(1)), w)
    } else {
        (h, w.min(max_kept.max(1)))
    };
    if nh == 1 && nw == 1 {
        None
    } else {
        Some((nh, nw))
    }
}

/// `(max_possible_truthy, filter_rows)` — column include filters rows.
fn include_max_kept(
    include: &Expr,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<(usize, bool)> {
    match include {
        Expr::Range { start, end } => {
            let (c1, r1) = parse_a1(start)?;
            let (c2, r2) = parse_a1(end)?;
            let (cmin, cmax) = if c1 <= c2 { (c1, c2) } else { (c2, c1) };
            let (rmin, rmax) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
            let width = (cmax - cmin + 1) as usize;
            let height = (rmax - rmin + 1) as usize;
            if width == 1 {
                Some((height, true))
            } else if height == 1 {
                Some((width, false))
            } else {
                None
            }
        }
        Expr::CmpOp { op, left, right } => match (left.as_ref(), right.as_ref()) {
            (Expr::Range { start, end }, scalar) => {
                count_cmp_max_kept(*op, start, end, true, scalar, asts, occupied, visiting)
            }
            (scalar, Expr::Range { start, end }) => {
                count_cmp_max_kept(*op, start, end, false, scalar, asts, occupied, visiting)
            }
            _ => None,
        },
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn count_cmp_max_kept(
    op: CmpOp,
    start: &str,
    end: &str,
    range_on_left: bool,
    scalar: &Expr,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<(usize, bool)> {
    let (c1, r1) = parse_a1(start)?;
    let (c2, r2) = parse_a1(end)?;
    let (cmin, cmax) = if c1 <= c2 { (c1, c2) } else { (c2, c1) };
    let (rmin, rmax) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
    let width = (cmax - cmin + 1) as usize;
    let height = (rmax - rmin + 1) as usize;
    let filter_rows = width == 1;
    if !filter_rows && height != 1 {
        return None;
    }
    let s = const_number_in_sheet(scalar, asts, occupied, visiting);
    let mut max_kept = 0usize;
    for_each_a1_range(start, end, |cell| {
        match (
            s,
            const_number_in_sheet(&Expr::Cell(cell), asts, occupied, visiting),
        ) {
            (Some(scalar_v), Some(v)) => {
                let truth = if range_on_left {
                    apply_cmp(op, v, scalar_v)
                } else {
                    apply_cmp(op, scalar_v, v)
                };
                if truth {
                    max_kept += 1;
                }
            }
            (_, None) | (None, _) => {
                // Unknown → may be kept.
                max_kept += 1;
            }
        }
        Ok(())
    })
    .ok()?;
    Some((max_kept, filter_rows))
}

fn spill_footprint_cell_set(anchor: &str, shape: Option<(usize, usize)>) -> HashSet<String> {
    let mut out = HashSet::new();
    let Some((height, width)) = shape else {
        if parse_a1(anchor).is_some() {
            out.insert(anchor.to_string());
        }
        return out;
    };
    let Some((col, row)) = parse_a1(anchor) else {
        return out;
    };
    for dr in 0..height {
        for dc in 0..width {
            out.insert(format_a1(col + dc as u32, row + dr as u32));
        }
    }
    out
}

/// Fold eager scalar functions / short-circuit IF when all needed args fold.
fn const_call_number(
    name: &str,
    args: &[Expr],
    fold_arg: &mut dyn FnMut(&Expr) -> Option<f64>,
    cell_kind: Option<&dyn Fn(&str) -> AggregateCellKind>,
) -> Option<f64> {
    match name.to_uppercase().as_str() {
        "IF" if args.len() == 3 => {
            let cond = fold_arg(&args[0])?;
            if is_truthy(cond) {
                fold_arg(&args[1])
            } else {
                fold_arg(&args[2])
            }
        }
        "AND" => {
            if args.is_empty() {
                return None;
            }
            for a in args {
                if !is_truthy(fold_arg(a)?) {
                    return Some(0.0);
                }
            }
            Some(1.0)
        }
        "OR" => {
            if args.is_empty() {
                return None;
            }
            for a in args {
                if is_truthy(fold_arg(a)?) {
                    return Some(1.0);
                }
            }
            Some(0.0)
        }
        "NOT" if args.len() == 1 => Some(bool_to_number(!is_truthy(fold_arg(&args[0])?))),
        "ABS" if args.len() == 1 => Some(fold_arg(&args[0])?.abs()),
        "INT" if args.len() == 1 => Some(fold_arg(&args[0])?.floor()),
        "SUM" | "PRODUCT" | "MIN" | "MAX" | "AVERAGE" | "COUNT" | "COUNTA" => {
            let mut vals = Vec::new();
            let mut counta = 0usize;
            for a in args {
                flatten_const_numeric_arg(a, fold_arg, cell_kind, &mut vals, &mut counta)?;
            }
            match name.to_uppercase().as_str() {
                "SUM" => Some(vals.iter().sum()),
                "COUNT" => Some(vals.len() as f64),
                "COUNTA" => Some(counta as f64),
                "PRODUCT" => {
                    if vals.is_empty() {
                        Some(0.0)
                    } else {
                        Some(vals.iter().product())
                    }
                }
                "MIN" => {
                    if vals.is_empty() {
                        Some(0.0)
                    } else {
                        Some(vals.iter().copied().fold(f64::INFINITY, f64::min))
                    }
                }
                "MAX" => {
                    if vals.is_empty() {
                        Some(0.0)
                    } else {
                        Some(vals.iter().copied().fold(f64::NEG_INFINITY, f64::max))
                    }
                }
                "AVERAGE" => {
                    if vals.is_empty() {
                        None
                    } else {
                        Some(vals.iter().sum::<f64>() / vals.len() as f64)
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Flatten one aggregate argument for const-fold (scalars or A1 ranges of foldable cells).
/// Blanks/text in ranges are skipped like Excel aggregates (not treated as 0).
fn flatten_const_numeric_arg(
    arg: &Expr,
    fold_arg: &mut dyn FnMut(&Expr) -> Option<f64>,
    cell_kind: Option<&dyn Fn(&str) -> AggregateCellKind>,
    out: &mut Vec<f64>,
    counta: &mut usize,
) -> Option<()> {
    match arg {
        Expr::Range { start, end } => {
            let mut ok = true;
            for_each_a1_range(start, end, |cell| {
                if !ok {
                    return Ok(());
                }
                match cell_kind.map(|f| f(&cell)) {
                    Some(AggregateCellKind::Blank) => {}
                    Some(AggregateCellKind::Text) => {
                        *counta += 1;
                    }
                    Some(AggregateCellKind::Value) | None => match fold_arg(&Expr::Cell(cell)) {
                        Some(n) => {
                            out.push(n);
                            *counta += 1;
                        }
                        None => ok = false,
                    },
                }
                Ok(())
            })
            .ok()?;
            if ok {
                Some(())
            } else {
                None
            }
        }
        Expr::Str(_) => {
            *counta += 1;
            Some(())
        }
        Expr::Cell(name) => match cell_kind.map(|f| f(name)) {
            Some(AggregateCellKind::Blank) => Some(()),
            Some(AggregateCellKind::Text) => {
                *counta += 1;
                Some(())
            }
            Some(AggregateCellKind::Value) | None => {
                out.push(fold_arg(arg)?);
                *counta += 1;
                Some(())
            }
        },
        other => {
            out.push(fold_arg(other)?);
            *counta += 1;
            Some(())
        }
    }
}

fn call_spill_shape(
    name: &str,
    args: &[Expr],
) -> Result<Option<(usize, usize)>, SpreadsheetError> {
    match name.to_uppercase().as_str() {
        "IF" if args.len() == 3 => Ok(if_spill_shape_arms(
            &args[0],
            expr_spill_shape(&args[1])?,
            expr_spill_shape(&args[2])?,
        )),
        "IFERROR" if (1..=2).contains(&args.len()) => {
            let primary = expr_spill_shape(&args[0])?;
            if primary.is_some() {
                Ok(primary)
            } else if let Some(fb) = args.get(1) {
                expr_spill_shape(fb)
            } else {
                Ok(None)
            }
        }
        "IFS" => Ok(ifs_spill_shape_arms(args, expr_spill_shape)?),
        "SWITCH" if !args.is_empty() => Ok(switch_spill_shape_arms(args, expr_spill_shape)?),
        "SEQUENCE" => Ok(crate::dynamic_array::sequence_spill_shape_from_consts(
            args.first().and_then(const_number),
            args.get(1).and_then(const_number),
        )),
        // FILTER/SORT/UNIQUE: FILTER uses emptiness-aware footprint when possible.
        "FILTER" if args.len() >= 2 => {
            let array_shape = expr_spill_shape(&args[0])?;
            let empty_shape = if let Some(fb) = args.get(2) {
                expr_spill_shape(fb)?
            } else {
                None
            };
            match args.get(1).and_then(const_truthy) {
                Some(true) => Ok(array_shape),
                Some(false) => Ok(empty_shape),
                None => Ok(union_spill_shapes([array_shape, empty_shape])),
            }
        }
        "SORT" | "UNIQUE" if !args.is_empty() => expr_spill_shape(&args[0]),
        "LET" => match expand_let_for_analysis(args) {
            Some(expanded) => expr_spill_shape(&expanded),
            None => Ok(None),
        },
        // Aggregates and other eager functions return scalars.
        _ => Ok(None),
    }
}

fn apply_binop(op: BinOp, l: f64, r: f64) -> Result<f64, SpreadsheetError> {
    match op {
        BinOp::Add => Ok(l + r),
        BinOp::Sub => Ok(l - r),
        BinOp::Mul => Ok(l * r),
        BinOp::Div => {
            if r == 0.0 {
                Err(SpreadsheetError::DivisionByZero)
            } else {
                Ok(l / r)
            }
        }
    }
}

fn apply_cmp(op: CmpOp, l: f64, r: f64) -> bool {
    match op {
        CmpOp::Gt => l > r,
        CmpOp::Lt => l < r,
        CmpOp::Ge => l >= r,
        CmpOp::Le => l <= r,
        CmpOp::Eq => l == r,
        CmpOp::Ne => l != r,
    }
}

/// Same-shape arrays, scalar↔array, or Excel-style 1×N ⊗ M×1 broadcast.
fn zip_values(
    left: EvalValue,
    right: EvalValue,
    mut f: impl FnMut(f64, f64) -> Result<f64, SpreadsheetError>,
) -> Result<EvalValue, SpreadsheetError> {
    match (left, right) {
        (EvalValue::Number(l), EvalValue::Number(r)) => Ok(EvalValue::Number(f(l, r)?)),
        (EvalValue::Number(l), EvalValue::Array(rows)) => {
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let mut new_row = Vec::with_capacity(row.len());
                for r in row {
                    new_row.push(f(l, r)?);
                }
                out.push(new_row);
            }
            Ok(EvalValue::Array(out))
        }
        (EvalValue::Array(rows), EvalValue::Number(r)) => {
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let mut new_row = Vec::with_capacity(row.len());
                for l in row {
                    new_row.push(f(l, r)?);
                }
                out.push(new_row);
            }
            Ok(EvalValue::Array(out))
        }
        (EvalValue::Array(left_rows), EvalValue::Array(right_rows)) => {
            let lh = left_rows.len();
            let lw = left_rows.first().map(|r| r.len()).unwrap_or(0);
            let rh = right_rows.len();
            let rw = right_rows.first().map(|r| r.len()).unwrap_or(0);
            if lw == 0 || rw == 0 || left_rows.iter().any(|r| r.len() != lw) || right_rows.iter().any(|r| r.len() != rw)
            {
                return Err(SpreadsheetError::Value);
            }

            if lh == rh && lw == rw {
                let mut out = Vec::with_capacity(lh);
                for (lrow, rrow) in left_rows.into_iter().zip(right_rows) {
                    let mut new_row = Vec::with_capacity(lw);
                    for (l, r) in lrow.into_iter().zip(rrow) {
                        new_row.push(f(l, r)?);
                    }
                    out.push(new_row);
                }
                return Ok(EvalValue::Array(out));
            }

            // Excel-style: 1×N ⊗ M×1 → M×N (and the swap).
            if lh == 1 && rw == 1 {
                let mut out = Vec::with_capacity(rh);
                for rrow in &right_rows {
                    let mut new_row = Vec::with_capacity(lw);
                    for &l in &left_rows[0] {
                        new_row.push(f(l, rrow[0])?);
                    }
                    out.push(new_row);
                }
                return Ok(EvalValue::Array(out));
            }
            if lw == 1 && rh == 1 {
                let mut out = Vec::with_capacity(lh);
                for lrow in &left_rows {
                    let mut new_row = Vec::with_capacity(rw);
                    for &r in &right_rows[0] {
                        new_row.push(f(lrow[0], r)?);
                    }
                    out.push(new_row);
                }
                return Ok(EvalValue::Array(out));
            }

            Err(SpreadsheetError::Value)
        }
    }
}

fn eval_spill_ref(anchor: &str, ctx: &EvalContext<'_>) -> Result<EvalValue, SpreadsheetError> {
    if let Some(&(height, width)) = ctx.spill_meta.get(anchor) {
        let (col, row) = parse_a1(anchor).ok_or(SpreadsheetError::Value)?;
        let mut rows = Vec::with_capacity(height);
        for dr in 0..height {
            let mut row_vals = Vec::with_capacity(width);
            for dc in 0..width {
                row_vals.push(ctx.lookup_number(&format_a1(col + dc as u32, row + dr as u32))?);
            }
            rows.push(row_vals);
        }
        if height == 1 && width == 1 {
            Ok(EvalValue::Number(rows[0][0]))
        } else {
            Ok(EvalValue::Array(rows))
        }
    } else {
        Ok(EvalValue::Number(ctx.lookup_number(anchor)?))
    }
}

fn eval_intersect(
    expr: &Expr,
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<f64, SpreadsheetError> {
    match expr {
        Expr::Range { start, end } => implicit_intersect_range(start, end, ctx),
        Expr::SpillRef(anchor) => {
            if let Some(&(height, width)) = ctx.spill_meta.get(anchor) {
                let (col, row) = parse_a1(anchor).ok_or(SpreadsheetError::Value)?;
                let end = format_a1(col + (width as u32 - 1), row + (height as u32 - 1));
                implicit_intersect_range(anchor, &end, ctx)
            } else {
                ctx.lookup_number(anchor)
            }
        }
        Expr::Cell(name) => ctx.lookup_number(name),
        Expr::Intersect(inner) => eval_intersect(inner, ctx, source),
        other => eval_value(other, ctx, source)?.as_scalar(),
    }
}

/// Excel-style implicit intersection from the evaluating cell into a range.
fn implicit_intersect_range(
    start: &str,
    end: &str,
    ctx: &EvalContext<'_>,
) -> Result<f64, SpreadsheetError> {
    let (ec, er) = parse_a1(ctx.eval_cell).ok_or(SpreadsheetError::Value)?;
    let (c1, r1) = parse_a1(start).ok_or(SpreadsheetError::Value)?;
    let (c2, r2) = parse_a1(end).ok_or(SpreadsheetError::Value)?;
    let (cmin, cmax) = if c1 <= c2 { (c1, c2) } else { (c2, c1) };
    let (rmin, rmax) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };

    let target = if cmin == cmax && rmin == rmax {
        format_a1(cmin, rmin)
    } else if cmin == cmax {
        if er < rmin || er > rmax {
            return Err(SpreadsheetError::Value);
        }
        format_a1(cmin, er)
    } else if rmin == rmax {
        if ec < cmin || ec > cmax {
            return Err(SpreadsheetError::Value);
        }
        format_a1(ec, rmin)
    } else if ec >= cmin && ec <= cmax && er >= rmin && er <= rmax {
        format_a1(ec, er)
    } else {
        return Err(SpreadsheetError::Value);
    };
    ctx.lookup_number(&target)
}

fn eval_range_value(
    start: &str,
    end: &str,
    ctx: &EvalContext<'_>,
) -> Result<EvalValue, SpreadsheetError> {
    let _ = a1_range_size(start, end)?;
    let (c1, r1) = parse_a1(start).ok_or(SpreadsheetError::Value)?;
    let (c2, r2) = parse_a1(end).ok_or(SpreadsheetError::Value)?;
    let (cmin, cmax) = if c1 <= c2 { (c1, c2) } else { (c2, c1) };
    let (rmin, rmax) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
    let height = (rmax - rmin + 1) as usize;
    let width = (cmax - cmin + 1) as usize;

    let mut rows = Vec::with_capacity(height);
    for r in rmin..=rmax {
        let mut row = Vec::with_capacity(width);
        for c in cmin..=cmax {
            row.push(ctx.lookup_number(&format_a1(c, r))?);
        }
        rows.push(row);
    }

    if height == 1 && width == 1 {
        Ok(EvalValue::Number(rows[0][0]))
    } else {
        Ok(EvalValue::Array(rows))
    }
}

fn eval_call_value(
    name: &str,
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<EvalValue, SpreadsheetError> {
    match name.to_uppercase().as_str() {
        "IF" => {
            require_arity(args, 3, source)?;
            if is_truthy(eval_expr(&args[0], ctx, source)?) {
                eval_value(&args[1], ctx, source)
            } else {
                eval_value(&args[2], ctx, source)
            }
        }
        "IFERROR" => {
            if args.is_empty() || args.len() > 2 {
                return Err(SpreadsheetError::InvalidFormula(source.to_string()));
            }
            match eval_value(&args[0], ctx, source) {
                Ok(v) => Ok(v),
                Err(_) => {
                    if let Some(fb) = args.get(1) {
                        eval_value(fb, ctx, source)
                    } else {
                        Ok(EvalValue::Number(0.0))
                    }
                }
            }
        }
        "IFS" => {
            if args.len() < 2 || !args.len().is_multiple_of(2) {
                return Err(SpreadsheetError::InvalidFormula(source.to_string()));
            }
            for pair in args.chunks(2) {
                if is_truthy(eval_expr(&pair[0], ctx, source)?) {
                    return eval_value(&pair[1], ctx, source);
                }
            }
            Err(SpreadsheetError::InvalidFormula(source.to_string()))
        }
        "SWITCH" => eval_switch_value(args, ctx, source),
        "LET" => eval_let_value(args, ctx, source),
        "SEQUENCE" => eval_sequence_call(args, ctx, source),
        "UNIQUE" => eval_unique_call(args, ctx, source),
        "SORT" => eval_sort_call(args, ctx, source),
        "FILTER" => eval_filter_call(args, ctx, source),
        other => Ok(EvalValue::Number(eval_call_scalar(other, args, ctx, source)?)),
    }
}

fn eval_let_value(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<EvalValue, SpreadsheetError> {
    let Some(calc) = let_calculation(args) else {
        return Err(SpreadsheetError::InvalidFormula(source.to_string()));
    };
    let pairs = &args[..args.len() - 1];
    let mut map = std::collections::HashMap::new();
    if let Some(parent) = ctx.bindings {
        for (k, v) in parent {
            map.insert(k.clone(), v.clone());
        }
    }
    for pair in pairs.chunks(2) {
        if pair.len() != 2 {
            return Err(SpreadsheetError::InvalidFormula(source.to_string()));
        }
        let Some(name) = let_binding_name(&pair[0]) else {
            return Err(SpreadsheetError::InvalidFormula(source.to_string()));
        };
        let scoped = EvalContext {
            asts: ctx.asts,
            sources: ctx.sources,
            cache: ctx.cache,
            text_cells: ctx.text_cells,
            eval_cell: ctx.eval_cell,
            spill_meta: ctx.spill_meta,
            bindings: Some(&map),
        };
        let value = eval_value(&pair[1], &scoped, source)?;
        map.insert(name, value);
    }
    let scoped = EvalContext {
        asts: ctx.asts,
        sources: ctx.sources,
        cache: ctx.cache,
        text_cells: ctx.text_cells,
        eval_cell: ctx.eval_cell,
        spill_meta: ctx.spill_meta,
        bindings: Some(&map),
    };
    eval_value(calc, &scoped, source)
}

fn eval_sequence_call(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<EvalValue, SpreadsheetError> {
    if args.is_empty() || args.len() > 4 {
        return Err(SpreadsheetError::InvalidFormula(source.to_string()));
    }
    let rows = eval_expr(&args[0], ctx, source)?;
    let columns = if let Some(a) = args.get(1) {
        eval_expr(a, ctx, source)?
    } else {
        1.0
    };
    let start = if let Some(a) = args.get(2) {
        eval_expr(a, ctx, source)?
    } else {
        1.0
    };
    let step = if let Some(a) = args.get(3) {
        eval_expr(a, ctx, source)?
    } else {
        1.0
    };
    crate::dynamic_array::sequence(rows, columns, start, step)
}

fn eval_unique_call(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<EvalValue, SpreadsheetError> {
    if args.is_empty() || args.len() > 3 {
        return Err(SpreadsheetError::InvalidFormula(source.to_string()));
    }
    let array = eval_value(&args[0], ctx, source)?;
    let by_col = if let Some(a) = args.get(1) {
        is_truthy(eval_expr(a, ctx, source)?)
    } else {
        false
    };
    let exactly_once = if let Some(a) = args.get(2) {
        is_truthy(eval_expr(a, ctx, source)?)
    } else {
        false
    };
    crate::dynamic_array::unique(array, by_col, exactly_once)
}

fn eval_sort_call(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<EvalValue, SpreadsheetError> {
    if args.is_empty() || args.len() > 4 {
        return Err(SpreadsheetError::InvalidFormula(source.to_string()));
    }
    let array = eval_value(&args[0], ctx, source)?;
    let sort_index = if let Some(a) = args.get(1) {
        eval_expr(a, ctx, source)?
    } else {
        1.0
    };
    let sort_order = if let Some(a) = args.get(2) {
        eval_expr(a, ctx, source)?
    } else {
        1.0
    };
    let by_col = if let Some(a) = args.get(3) {
        is_truthy(eval_expr(a, ctx, source)?)
    } else {
        false
    };
    crate::dynamic_array::sort(array, sort_index, sort_order, by_col)
}

fn eval_filter_call(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<EvalValue, SpreadsheetError> {
    if args.len() < 2 || args.len() > 3 {
        return Err(SpreadsheetError::InvalidFormula(source.to_string()));
    }
    let array = eval_value(&args[0], ctx, source)?;
    let include = eval_value(&args[1], ctx, source)?;
    match crate::dynamic_array::filter(array, include, None) {
        Err(SpreadsheetError::Calc) => {
            // Empty path: evaluate if_empty now (blanks coerce to 0). `lib.rs` refreshes
            // all FILTER anchors after layers so cases where if_empty needed later
            // spill/formula cells are corrected in fixup.
            if let Some(fb) = args.get(2) {
                eval_value(fb, ctx, source)
            } else {
                Err(SpreadsheetError::Calc)
            }
        }
        other => other,
    }
}

fn eval_call_scalar(
    name: &str,
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<f64, SpreadsheetError> {
    match name {
        "COUNT" => Ok(count_args(args, ctx, source, false)? as f64),
        "COUNTA" => Ok(count_args(args, ctx, source, true)? as f64),
        "DATE" => {
            require_arity(args, 3, source)?;
            let year = eval_expr(&args[0], ctx, source)?;
            let month = eval_expr(&args[1], ctx, source)?;
            let day = eval_expr(&args[2], ctx, source)?;
            excel_date::excel_date(year.trunc() as i32, month.trunc() as i32, day.trunc() as i32)
                .map_err(|_| SpreadsheetError::Num)
        }
        "DATEVALUE" => {
            require_arity(args, 1, source)?;
            let text = eval_text_arg(&args[0], ctx)?;
            excel_date::datevalue(&text).map_err(|_| SpreadsheetError::Value)
        }
        "YEAR" | "MONTH" | "DAY" => {
            require_arity(args, 1, source)?;
            let serial = eval_date_arg(&args[0], ctx, source)?;
            let civil = excel_date::serial_to_ymd(serial).map_err(|_| SpreadsheetError::Num)?;
            Ok(match name {
                "YEAR" => f64::from(civil.year),
                "MONTH" => f64::from(civil.month),
                _ => f64::from(civil.day),
            })
        }
        "DAYS" => {
            require_arity(args, 2, source)?;
            let end = eval_date_arg(&args[0], ctx, source)?;
            let start = eval_date_arg(&args[1], ctx, source)?;
            let end_civil = excel_date::serial_to_ymd(end).map_err(|_| SpreadsheetError::Num)?;
            let start_civil = excel_date::serial_to_ymd(start).map_err(|_| SpreadsheetError::Num)?;
            let end_serial =
                excel_date::ymd_to_serial(end_civil.year, end_civil.month, end_civil.day)
                    .map_err(|_| SpreadsheetError::Num)?;
            let start_serial =
                excel_date::ymd_to_serial(start_civil.year, start_civil.month, start_civil.day)
                    .map_err(|_| SpreadsheetError::Num)?;
            Ok(end_serial - start_serial)
        }
        "DATEDIF" => {
            require_arity(args, 3, source)?;
            let start = eval_date_arg(&args[0], ctx, source)?;
            let end = eval_date_arg(&args[1], ctx, source)?;
            let unit = eval_text_arg(&args[2], ctx)?;
            let start = excel_date::serial_to_ymd(start).map_err(|_| SpreadsheetError::Num)?;
            let end = excel_date::serial_to_ymd(end).map_err(|_| SpreadsheetError::Num)?;
            excel_date::datedif(start, end, &unit).map_err(|err| match err {
                excel_date::DatedifError::Num => SpreadsheetError::Num,
                excel_date::DatedifError::Value => SpreadsheetError::Value,
            })
        }
        _ => {
            let flat = flatten_numeric_args(args, ctx, source)?;
            eval_eager_function(name, &flat, source)
        }
    }
}

fn eval_switch_value(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<EvalValue, SpreadsheetError> {
    if args.is_empty() {
        return Err(SpreadsheetError::InvalidFormula(source.to_string()));
    }
    let target = eval_expr(&args[0], ctx, source)?;
    let rest = &args[1..];
    let mut i = 0;
    while i + 1 < rest.len() {
        if eval_expr(&rest[i], ctx, source)? == target {
            return eval_value(&rest[i + 1], ctx, source);
        }
        i += 2;
    }
    if rest.len() % 2 == 1 {
        eval_value(&rest[rest.len() - 1], ctx, source)
    } else {
        Err(SpreadsheetError::InvalidFormula(source.to_string()))
    }
}

fn flatten_numeric_args(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<Vec<f64>, SpreadsheetError> {
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Expr::Range { start, end } => {
                for_each_a1_range(start, end, |cell| {
                    if let Some(n) = ctx.lookup_aggregate_number(&cell)? {
                        out.push(n);
                    }
                    Ok(())
                })?;
            }
            Expr::Str(_) => {
                // Excel SUM/AVERAGE/… ignore text literals.
            }
            Expr::Cell(name) => {
                if let Some(n) = ctx.lookup_aggregate_number(name)? {
                    out.push(n);
                }
            }
            other => match eval_value(other, ctx, source)? {
                EvalValue::Number(n) => out.push(n),
                EvalValue::Array(rows) => {
                    for row in rows {
                        out.extend(row);
                    }
                }
            },
        }
    }
    Ok(out)
}

/// Excel COUNT counts numbers only; COUNTA counts non-blank (numbers + text).
fn count_args(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
    count_text: bool,
) -> Result<usize, SpreadsheetError> {
    let mut n = 0usize;
    for arg in args {
        match arg {
            Expr::Range { start, end } => {
                for_each_a1_range(start, end, |cell| {
                    match ctx.count_cell_kind(&cell) {
                        CellKind::Number => n += 1,
                        CellKind::Text if count_text => n += 1,
                        CellKind::Text | CellKind::Blank => {}
                    }
                    Ok(())
                })?;
            }
            Expr::Str(_) => {
                if count_text {
                    n += 1;
                }
            }
            Expr::Cell(name) => match ctx.count_cell_kind(name) {
                CellKind::Number => n += 1,
                CellKind::Text if count_text => n += 1,
                CellKind::Text | CellKind::Blank => {}
            },
            other => match eval_value(other, ctx, source)? {
                EvalValue::Number(_) => n += 1,
                EvalValue::Array(rows) => {
                    n += rows.iter().map(|row| row.len()).sum::<usize>();
                }
            },
        }
    }
    Ok(n)
}

fn eval_date_arg(
    expr: &Expr,
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<f64, SpreadsheetError> {
    match expr {
        Expr::Str(text) => excel_date::datevalue(text).map_err(|_| SpreadsheetError::Value),
        other => eval_expr(other, ctx, source),
    }
}

fn eval_text_arg(expr: &Expr, ctx: &EvalContext<'_>) -> Result<String, SpreadsheetError> {
    match expr {
        Expr::Str(text) => Ok(text.clone()),
        Expr::Cell(name) => ctx.cell_text_value(name),
        _ => Err(SpreadsheetError::Value),
    }
}

fn require_arity(args: &[Expr], n: usize, source: &str) -> Result<(), SpreadsheetError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(SpreadsheetError::InvalidFormula(source.to_string()))
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_refs_in_sheet(
    expr: &Expr,
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    eval_cell: Option<&str>,
    visiting: &mut std::collections::HashSet<String>,
    refs: &mut FormulaRefs,
    ignore: &HashSet<String>,
) -> Result<(), SpreadsheetError> {
    match expr {
        Expr::Number(_) | Expr::Str(_) => {}
        Expr::Cell(name) | Expr::SpillRef(name) => {
            if ignore.contains(name) {
                return Ok(());
            }
            refs.work += 1;
            refs.cells.insert(name.clone());
        }
        Expr::Range { start, end } => {
            let size = a1_range_size(start, end)?;
            refs.work += size;
            for_each_a1_range(start, end, |cell| {
                refs.cells.insert(cell);
                Ok(())
            })?;
        }
        Expr::Intersect(inner) | Expr::Neg(inner) => {
            collect_refs_in_sheet(inner, asts, occupied, eval_cell, visiting, refs, ignore)?
        }
        Expr::BinOp { left, right, .. } | Expr::CmpOp { left, right, .. } => {
            collect_refs_in_sheet(left, asts, occupied, eval_cell, visiting, refs, ignore)?;
            collect_refs_in_sheet(right, asts, occupied, eval_cell, visiting, refs, ignore)?;
        }
        Expr::Call { name, args } => {
            collect_call_refs_in_sheet(
                name, args, asts, occupied, eval_cell, visiting, refs, ignore,
            )?
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_call_refs_in_sheet(
    name: &str,
    args: &[Expr],
    asts: &std::collections::HashMap<String, Expr>,
    occupied: &std::collections::HashSet<String>,
    eval_cell: Option<&str>,
    visiting: &mut std::collections::HashSet<String>,
    refs: &mut FormulaRefs,
    ignore: &HashSet<String>,
) -> Result<(), SpreadsheetError> {
    match name.to_uppercase().as_str() {
        "IF" if args.len() == 3 => {
            collect_refs_in_sheet(&args[0], asts, occupied, eval_cell, visiting, refs, ignore)?;
            match const_truthy_in_sheet(&args[0], asts, occupied, visiting) {
                Some(true) => {
                    collect_refs_in_sheet(
                        &args[1], asts, occupied, eval_cell, visiting, refs, ignore,
                    )?
                }
                Some(false) => {
                    collect_refs_in_sheet(
                        &args[2], asts, occupied, eval_cell, visiting, refs, ignore,
                    )?
                }
                None => {
                    collect_refs_in_sheet(
                        &args[1], asts, occupied, eval_cell, visiting, refs, ignore,
                    )?;
                    collect_refs_in_sheet(
                        &args[2], asts, occupied, eval_cell, visiting, refs, ignore,
                    )?;
                }
            }
        }
        "IFERROR" if (1..=2).contains(&args.len()) => {
            collect_refs_in_sheet(&args[0], asts, occupied, eval_cell, visiting, refs, ignore)?;
            let primary_shape = expr_spill_shape_in_sheet(&args[0], asts, occupied)?;
            if primary_shape.is_none() {
                if let Some(fb) = args.get(1) {
                    collect_refs_in_sheet(fb, asts, occupied, eval_cell, visiting, refs, ignore)?;
                }
            }
        }
        "FILTER" if (2..=3).contains(&args.len()) => {
            collect_refs_in_sheet(&args[0], asts, occupied, eval_cell, visiting, refs, ignore)?;
            collect_refs_in_sheet(&args[1], asts, occupied, eval_cell, visiting, refs, ignore)?;
            let emptiness = args
                .get(1)
                .and_then(|inc| const_filter_include_any_truthy(inc, asts, occupied, visiting));
            match emptiness {
                Some(true) => {}
                Some(false) => {
                    if let Some(fb) = args.get(2) {
                        collect_refs_in_sheet(
                            fb, asts, occupied, eval_cell, visiting, refs, ignore,
                        )?;
                    }
                }
                None => {
                    // Unknown: collect if_empty but drop refs inside this FILTER's spill footprint.
                    if let Some(fb) = args.get(2) {
                        let shape = filter_spill_shape_in_sheet(args, asts, occupied, visiting)?;
                        let exclude = eval_cell
                            .map(|a| spill_footprint_cell_set(a, shape))
                            .unwrap_or_default();
                        let mut fb_refs = FormulaRefs::default();
                        collect_refs_in_sheet(
                            fb,
                            asts,
                            occupied,
                            eval_cell,
                            visiting,
                            &mut fb_refs,
                            ignore,
                        )?;
                        for cell in fb_refs.cells {
                            if !exclude.contains(&cell) {
                                refs.cells.insert(cell);
                            }
                        }
                        refs.work += fb_refs.work;
                    }
                }
            }
        }
        "IFS" => {
            let mut uncertain = false;
            for pair in args.chunks(2) {
                if pair.len() != 2 {
                    break;
                }
                collect_refs_in_sheet(
                    &pair[0], asts, occupied, eval_cell, visiting, refs, ignore,
                )?;
                match const_truthy_in_sheet(&pair[0], asts, occupied, visiting) {
                    Some(true) if !uncertain => {
                        collect_refs_in_sheet(
                            &pair[1], asts, occupied, eval_cell, visiting, refs, ignore,
                        )?;
                        return Ok(());
                    }
                    Some(false) if !uncertain => {}
                    _ => {
                        uncertain = true;
                        collect_refs_in_sheet(
                            &pair[1], asts, occupied, eval_cell, visiting, refs, ignore,
                        )?;
                    }
                }
            }
        }
        "SWITCH" if !args.is_empty() => {
            collect_refs_in_sheet(&args[0], asts, occupied, eval_cell, visiting, refs, ignore)?;
            let target = const_number_in_sheet(&args[0], asts, occupied, visiting);
            let rest = &args[1..];
            let mut uncertain = target.is_none();
            let mut i = 0;
            while i + 1 < rest.len() {
                collect_refs_in_sheet(
                    &rest[i], asts, occupied, eval_cell, visiting, refs, ignore,
                )?;
                match (target, const_number_in_sheet(&rest[i], asts, occupied, visiting)) {
                    (Some(t), Some(m)) if !uncertain && t == m => {
                        collect_refs_in_sheet(
                            &rest[i + 1],
                            asts,
                            occupied,
                            eval_cell,
                            visiting,
                            refs,
                            ignore,
                        )?;
                        return Ok(());
                    }
                    (Some(t), Some(m)) if !uncertain && t != m => {}
                    _ => {
                        uncertain = true;
                        collect_refs_in_sheet(
                            &rest[i + 1],
                            asts,
                            occupied,
                            eval_cell,
                            visiting,
                            refs,
                            ignore,
                        )?;
                    }
                }
                i += 2;
            }
            if rest.len() % 2 == 1 {
                collect_refs_in_sheet(
                    &rest[rest.len() - 1],
                    asts,
                    occupied,
                    eval_cell,
                    visiting,
                    refs,
                    ignore,
                )?;
            }
        }
        "LET" => {
            // Collect binding values in order (later values may use earlier names),
            // then the calculation — local names are never sheet refs / work.
            let Some(calc) = let_calculation(args) else {
                for arg in args {
                    collect_refs_in_sheet(
                        arg, asts, occupied, eval_cell, visiting, refs, ignore,
                    )?;
                }
                return Ok(());
            };
            let pairs = &args[..args.len() - 1];
            let mut bound = ignore.clone();
            for pair in pairs.chunks(2) {
                if pair.len() != 2 {
                    break;
                }
                collect_refs_in_sheet(
                    &pair[1], asts, occupied, eval_cell, visiting, refs, &bound,
                )?;
                if let Some(name) = let_binding_name(&pair[0]) {
                    bound.insert(name);
                }
            }
            collect_refs_in_sheet(calc, asts, occupied, eval_cell, visiting, refs, &bound)?;
        }
        _ => {
            for arg in args {
                collect_refs_in_sheet(arg, asts, occupied, eval_cell, visiting, refs, ignore)?;
            }
        }
    }
    Ok(())
}

struct AstParser<'a> {
    chars: Vec<char>,
    position: usize,
    expression: &'a str,
}

impl<'a> AstParser<'a> {
    fn parse_comparison(&mut self) -> Result<Expr, SpreadsheetError> {
        let mut left = self.parse_sum()?;
        loop {
            self.skip_whitespace();
            let op = if self.match_str(">=") {
                CmpOp::Ge
            } else if self.match_str("<=") {
                CmpOp::Le
            } else if self.match_str("<>") || self.match_str("!=") {
                CmpOp::Ne
            } else if self.match_str("==") || self.match_str("=") {
                CmpOp::Eq
            } else if self.match_str(">") {
                CmpOp::Gt
            } else if self.match_str("<") {
                CmpOp::Lt
            } else {
                return Ok(left);
            };
            let right = self.parse_sum()?;
            left = Expr::CmpOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn parse_sum(&mut self) -> Result<Expr, SpreadsheetError> {
        let mut left = self.parse_product()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some('+') => {
                    self.position += 1;
                    let right = self.parse_product()?;
                    left = Expr::BinOp {
                        op: BinOp::Add,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                Some('-') => {
                    self.position += 1;
                    let right = self.parse_product()?;
                    left = Expr::BinOp {
                        op: BinOp::Sub,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => return Ok(left),
            }
        }
    }

    fn parse_product(&mut self) -> Result<Expr, SpreadsheetError> {
        let mut left = self.parse_factor()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some('*') => {
                    self.position += 1;
                    let right = self.parse_factor()?;
                    left = Expr::BinOp {
                        op: BinOp::Mul,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                Some('/') => {
                    self.position += 1;
                    let right = self.parse_factor()?;
                    left = Expr::BinOp {
                        op: BinOp::Div,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => return Ok(left),
            }
        }
    }

    fn parse_factor(&mut self) -> Result<Expr, SpreadsheetError> {
        self.skip_whitespace();
        match self.peek() {
            Some('+') => {
                self.position += 1;
                self.parse_factor()
            }
            Some('-') => {
                self.position += 1;
                Ok(Expr::Neg(Box::new(self.parse_factor()?)))
            }
            Some('@') => {
                self.position += 1;
                Ok(Expr::Intersect(Box::new(self.parse_factor()?)))
            }
            Some('(') => {
                self.position += 1;
                let inner = self.parse_comparison()?;
                self.skip_whitespace();
                if self.peek() != Some(')') {
                    return Err(SpreadsheetError::InvalidFormula(self.expression.to_string()));
                }
                self.position += 1;
                Ok(inner)
            }
            Some('"') => Ok(Expr::Str(self.parse_string_literal()?)),
            Some(ch) if ch.is_ascii_digit() || ch == '.' => Ok(Expr::Number(self.parse_number()?)),
            Some(ch) if is_cell_name_start(ch) => self.parse_identifier_or_call(),
            _ => Err(SpreadsheetError::InvalidFormula(self.expression.to_string())),
        }
    }

    fn parse_identifier_or_call(&mut self) -> Result<Expr, SpreadsheetError> {
        let start = self.position;
        while matches!(self.peek(), Some(ch) if is_cell_name_char(ch)) {
            self.position += 1;
        }
        let name: String = self.chars[start..self.position].iter().collect();
        self.skip_whitespace();

        if self.peek() == Some('(') {
            self.position += 1;
            let args = self.parse_call_args(&name)?;
            self.skip_whitespace();
            if self.peek() != Some(')') {
                return Err(SpreadsheetError::InvalidFormula(self.expression.to_string()));
            }
            self.position += 1;
            return Ok(Expr::Call { name, args });
        }

        if self.peek() == Some('#') && parse_a1(&name).is_some() {
            self.position += 1;
            return Ok(Expr::SpillRef(canonical_cell_name(&name)));
        }

        if self.peek() == Some(':') && parse_a1(&name).is_some() {
            let saved = self.position;
            self.position += 1;
            self.skip_whitespace();
            if matches!(self.peek(), Some(ch) if is_cell_name_start(ch)) {
                let end_start = self.position;
                while matches!(self.peek(), Some(ch) if is_cell_name_char(ch)) {
                    self.position += 1;
                }
                let end_name: String = self.chars[end_start..self.position].iter().collect();
                if parse_a1(&end_name).is_some() {
                    let start = canonical_cell_name(&name);
                    let end = canonical_cell_name(&end_name);
                    a1_range_size(&start, &end)?;
                    return Ok(Expr::Range { start, end });
                }
            }
            self.position = saved;
        }

        Ok(Expr::Cell(canonical_cell_name(&name)))
    }

    fn parse_call_args(&mut self, name: &str) -> Result<Vec<Expr>, SpreadsheetError> {
        self.skip_whitespace();
        if self.peek() == Some(')') {
            return Ok(Vec::new());
        }

        // Text-oriented first args for DATEVALUE / DATEDIF unit still use factor (Str | Cell | expr).
        let upper = name.to_uppercase();
        let mut args = Vec::new();
        loop {
            // Allow ranges and expressions uniformly via parse_comparison,
            // except bare strings already handled in parse_factor.
            if upper == "DATEDIF" && args.len() == 2 || upper == "DATEVALUE" {
                args.push(self.parse_factor()?);
            } else {
                args.push(self.parse_comparison()?);
            }
            self.skip_whitespace();
            match self.peek() {
                Some(',') => {
                    self.position += 1;
                    self.skip_whitespace();
                }
                Some(')') => break,
                _ => {
                    return Err(SpreadsheetError::InvalidFormula(self.expression.to_string()))
                }
            }
        }
        Ok(args)
    }

    fn parse_string_literal(&mut self) -> Result<String, SpreadsheetError> {
        if self.peek() != Some('"') {
            return Err(SpreadsheetError::InvalidFormula(self.expression.to_string()));
        }
        self.position += 1;
        let mut value = String::new();
        while let Some(ch) = self.peek() {
            self.position += 1;
            if ch == '"' {
                if self.peek() == Some('"') {
                    self.position += 1;
                    value.push('"');
                } else {
                    return Ok(value);
                }
            } else {
                value.push(ch);
            }
        }
        Err(SpreadsheetError::InvalidFormula(self.expression.to_string()))
    }

    fn parse_number(&mut self) -> Result<f64, SpreadsheetError> {
        let start = self.position;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_digit() || ch == '.') {
            self.position += 1;
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.position += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.position += 1;
            }
            let exponent_start = self.position;
            while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                self.position += 1;
            }
            if self.position == exponent_start {
                return Err(SpreadsheetError::InvalidFormula(self.expression.to_string()));
            }
        }
        let number: String = self.chars[start..self.position].iter().collect();
        number
            .parse::<f64>()
            .map_err(|_| SpreadsheetError::InvalidFormula(self.expression.to_string()))
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn match_str(&mut self, s: &str) -> bool {
        let s_chars: Vec<char> = s.chars().collect();
        if self.position + s_chars.len() <= self.chars.len()
            && self.chars[self.position..self.position + s_chars.len()] == s_chars
        {
            self.position += s_chars.len();
            true
        } else {
            false
        }
    }
}
