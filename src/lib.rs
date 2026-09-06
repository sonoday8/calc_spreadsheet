mod ast;
mod deps;
mod error;
mod excel_date;
mod functions;
mod parser;
mod refs;
mod spreadsheet;
mod value;

use std::collections::{HashMap, HashSet};

use rayon::prelude::*;

pub use error::SpreadsheetError;
pub use value::CellValue;

use ast::{analyze_from_ast, parse_formula, Expr};
use deps::build_layers;
use spreadsheet::{evaluate_formula, is_pure_string_expression, pure_string_value, EvalContext};

/// Minimum cells in a layer before rayon is considered.
///
/// **Host-local:** re-run `examples/bench_threshold` on the target CPU before
/// treating these as universal. Re-calibrated after AST path (2026-09-05):
/// first stable speedup >= 1.1 at width=4096 × refs/cell=80.
const WIDTH_THRESHOLD: usize = 4096;

/// Minimum total work units in a layer (cell-ref occurrences + expanded A1 ranges).
///
/// **Host-local:** calibrated as width(4096) × refs/cell(80) from the same grid.
const WORK_THRESHOLD: usize = 327_680;

pub fn calculate_spreadsheet(
    cells: &[(&str, &str)],
) -> Result<HashMap<String, CellValue>, SpreadsheetError> {
    calculate_spreadsheet_inner(cells, ParallelPolicy::Adaptive)
}

/// Forces layer-inner rayon on or off for every layer (benchmarks / diagnostics).
#[doc(hidden)]
pub fn calculate_spreadsheet_with_parallel(
    cells: &[(&str, &str)],
    parallel: bool,
) -> Result<HashMap<String, CellValue>, SpreadsheetError> {
    calculate_spreadsheet_inner(cells, ParallelPolicy::Force(parallel))
}

#[derive(Clone, Copy)]
enum ParallelPolicy {
    Adaptive,
    Force(bool),
}

fn calculate_spreadsheet_inner(
    cells: &[(&str, &str)],
    policy: ParallelPolicy,
) -> Result<HashMap<String, CellValue>, SpreadsheetError> {
    let mut text_cells = HashMap::new();
    let mut values = HashMap::new();
    let mut asts: HashMap<String, Expr> = HashMap::new();
    let mut sources: HashMap<String, String> = HashMap::new();
    let mut formula_work = HashMap::new();
    let mut dependencies = HashMap::new();

    // First pass: classify cells and parse formulas once.
    let mut pending: Vec<(String, Expr)> = Vec::new();
    for (cell_name, expression) in cells {
        if is_pure_string_expression(expression) {
            let text = pure_string_value(expression).expect("checked as pure string");
            text_cells.insert((*cell_name).to_string(), text.clone());
            values.insert((*cell_name).to_string(), CellValue::Text(text));
        } else {
            let name = (*cell_name).to_string();
            let expr = parse_formula(expression)?;
            sources.insert(
                name.clone(),
                expression
                    .trim()
                    .strip_prefix('=')
                    .unwrap_or(expression.trim())
                    .to_string(),
            );
            pending.push((name, expr));
        }
    }

    let numeric_cells: HashSet<String> = pending.iter().map(|(n, _)| n.clone()).collect();
    for (name, expr) in pending {
        let analysis = analyze_from_ast(&expr)?;
        formula_work.insert(name.clone(), analysis.work);
        let numeric_deps: HashSet<String> = analysis
            .cells
            .into_iter()
            .filter(|r| numeric_cells.contains(r))
            .collect();
        dependencies.insert(name.clone(), numeric_deps);
        asts.insert(name, expr);
    }

    let layers = build_layers(&numeric_cells, &dependencies)?;
    let mut cache = HashMap::new();

    for layer in layers {
        let use_parallel = match policy {
            ParallelPolicy::Force(forced) => forced,
            ParallelPolicy::Adaptive => should_use_parallel(&layer, &formula_work),
        };
        let layer_results =
            evaluate_layer(&asts, &sources, &cache, &text_cells, &layer, use_parallel)?;
        for (name, value) in layer_results {
            cache.insert(name, value);
        }
    }

    for (name, value) in cache {
        values.insert(name, CellValue::Number(value));
    }

    Ok(values)
}

fn should_use_parallel(layer: &[String], work_by_cell: &HashMap<String, usize>) -> bool {
    let width = layer.len();
    let work: usize = layer
        .iter()
        .map(|cell| work_by_cell.get(cell).copied().unwrap_or(0))
        .sum();
    width >= WIDTH_THRESHOLD && work >= WORK_THRESHOLD
}

fn evaluate_layer(
    asts: &HashMap<String, Expr>,
    sources: &HashMap<String, String>,
    cache: &HashMap<String, f64>,
    text_cells: &HashMap<String, String>,
    layer: &[String],
    use_parallel: bool,
) -> Result<Vec<(String, f64)>, SpreadsheetError> {
    let ctx = EvalContext {
        asts,
        sources,
        cache,
        text_cells,
    };

    if use_parallel && layer.len() > 1 {
        layer
            .par_iter()
            .map(|name| {
                let value = evaluate_formula(&ctx, name)?;
                Ok((name.clone(), value))
            })
            .collect()
    } else {
        layer
            .iter()
            .map(|name| {
                let value = evaluate_formula(&ctx, name)?;
                Ok((name.clone(), value))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod adaptive_tests {
    use super::{should_use_parallel, WORK_THRESHOLD, WIDTH_THRESHOLD};
    use std::collections::HashMap;

    #[test]
    fn narrow_light_layer_stays_sequential() {
        let layer: Vec<String> = (0..40).map(|i| format!("C{i}")).collect();
        let mut refs = HashMap::new();
        for name in &layer {
            refs.insert(name.clone(), 10);
        }
        assert!(!should_use_parallel(&layer, &refs));
    }

    #[test]
    fn wide_heavy_layer_uses_parallel() {
        let layer: Vec<String> = (0..WIDTH_THRESHOLD).map(|i| format!("C{i}")).collect();
        let refs_per = WORK_THRESHOLD / WIDTH_THRESHOLD;
        let mut refs = HashMap::new();
        for name in &layer {
            refs.insert(name.clone(), refs_per);
        }
        assert!(should_use_parallel(&layer, &refs));
    }

    #[test]
    fn wide_but_light_layer_stays_sequential() {
        let layer: Vec<String> = (0..WIDTH_THRESHOLD).map(|i| format!("C{i}")).collect();
        let mut refs = HashMap::new();
        for name in &layer {
            refs.insert(name.clone(), 1);
        }
        assert!(!should_use_parallel(&layer, &refs));
    }
}
