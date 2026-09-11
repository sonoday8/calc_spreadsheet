mod ast;
mod deps;
mod dynamic_array;
mod error;
mod excel_date;
mod functions;
mod parser;
mod refs;
mod replace;
mod spreadsheet;
mod value;

use std::collections::{HashMap, HashSet};

use rayon::prelude::*;

pub use error::SpreadsheetError;
pub use replace::{format_number, ReplacementValue};
pub use value::CellValue;

use ast::{
    analyze_from_ast_in_sheet, expr_is_filter_spill_source, expr_spill_shape_in_sheet, parse_formula,
    EvalValue, Expr,
};
use deps::build_layers;
use refs::{canonical_cell_name, format_a1, parse_a1};
use spreadsheet::{evaluate_formula, is_pure_string_expression, pure_string_value, EvalContext};

/// Host-local defaults from `examples/bench_threshold` (AST path, 2026-09-05):
/// first stable speedup >= 1.1 at width=4096 × refs/cell=80.
/// Override per call via [`CalculateOptions::thresholds`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParallelThresholds {
    /// Minimum cells in a layer before rayon is considered.
    pub min_layer_width: usize,
    /// Minimum total work units in a layer (cell-ref occurrences + expanded A1 ranges).
    pub min_layer_work: usize,
}

impl Default for ParallelThresholds {
    fn default() -> Self {
        Self {
            min_layer_width: 4096,
            min_layer_work: 327_680,
        }
    }
}

/// Optional inputs for [`calculate_spreadsheet`].
///
/// - `replacements: None` — empty map (Excel-like prep still runs)
/// - `thresholds: None` — [`ParallelThresholds::default`]
#[derive(Clone, Copy, Debug, Default)]
pub struct CalculateOptions<'a> {
    pub replacements: Option<&'a HashMap<String, ReplacementValue>>,
    pub thresholds: Option<ParallelThresholds>,
}

/// Evaluation result (values plus any ignored invalid replacement keys).
#[derive(Debug, Clone, PartialEq)]
pub struct SpreadsheetOutcome {
    pub values: HashMap<String, CellValue>,
    pub ignored_replacement_keys: Vec<String>,
}

/// Evaluate a sheet with optional placeholder replacements and parallel thresholds.
///
/// Always runs Excel-like cell prep first: formulas must start with `=`, and bare
/// non-numeric text becomes a string cell.
pub fn calculate_spreadsheet(
    cells: &[(&str, &str)],
    options: CalculateOptions<'_>,
) -> Result<SpreadsheetOutcome, SpreadsheetError> {
    let empty = HashMap::new();
    let replacements = options.replacements.unwrap_or(&empty);
    let thresholds = options.thresholds.unwrap_or_default();
    let prepared = replace::prepare_cells(cells, replacements);
    let values = evaluate_prepared_cells(&prepared.cells, ParallelPolicy::Adaptive(thresholds))?;
    Ok(SpreadsheetOutcome {
        values,
        ignored_replacement_keys: prepared.ignored_replacement_keys,
    })
}

/// Forces layer-inner rayon on or off for every layer (benchmarks / diagnostics).
#[doc(hidden)]
pub fn calculate_spreadsheet_with_parallel(
    cells: &[(&str, &str)],
    parallel: bool,
) -> Result<HashMap<String, CellValue>, SpreadsheetError> {
    let prepared = replace::prepare_cells(cells, &HashMap::new());
    evaluate_prepared_cells(&prepared.cells, ParallelPolicy::Force(parallel))
}

#[derive(Clone, Copy)]
enum ParallelPolicy {
    Adaptive(ParallelThresholds),
    Force(bool),
}

fn evaluate_prepared_cells(
    cells: &[(String, String)],
    policy: ParallelPolicy,
) -> Result<HashMap<String, CellValue>, SpreadsheetError> {
    let input: Vec<(&str, &str)> = cells
        .iter()
        .map(|(name, expr)| (name.as_str(), expr.as_str()))
        .collect();
    calculate_spreadsheet_inner(&input, policy)
}

fn calculate_spreadsheet_inner(
    cells: &[(&str, &str)],
    policy: ParallelPolicy,
) -> Result<HashMap<String, CellValue>, SpreadsheetError> {
    let mut text_cells = HashMap::new();
    let mut values = HashMap::new();
    let mut sources: HashMap<String, String> = HashMap::new();

    // First pass: classify cells and parse formulas once.
    let mut pending: Vec<(String, Expr)> = Vec::new();
    for (cell_name, expression) in cells {
        let cell_name = canonical_cell_name(cell_name);
        if is_pure_string_expression(expression) {
            let text = pure_string_value(expression).expect("checked as pure string");
            text_cells.insert(cell_name.clone(), text.clone());
            values.insert(cell_name, CellValue::Text(text));
        } else {
            let expr = parse_formula(expression)?;
            sources.insert(
                cell_name.clone(),
                expression
                    .trim()
                    .strip_prefix('=')
                    .unwrap_or(expression.trim())
                    .to_string(),
            );
            pending.push((cell_name, expr));
        }
    }

    let numeric_cells: HashSet<String> = pending.iter().map(|(n, _)| n.clone()).collect();
    let occupied: HashSet<String> = numeric_cells
        .iter()
        .cloned()
        .chain(text_cells.keys().cloned())
        .collect();

    let mut asts: HashMap<String, Expr> = HashMap::new();
    for (name, expr) in &pending {
        asts.insert(name.clone(), expr.clone());
    }

    // Potential spill coverage: non-anchor cells that a formula may write into.
    let mut spill_providers: HashMap<String, HashSet<String>> = HashMap::new();
    for (name, expr) in &pending {
        if let Some((height, width)) = expr_spill_shape_in_sheet(expr, &asts, &occupied)? {
            if let Some((col, row)) = parse_a1(name) {
                for dr in 0..height {
                    for dc in 0..width {
                        if dr == 0 && dc == 0 {
                            continue;
                        }
                        let target = format_a1(col + dc as u32, row + dr as u32);
                        spill_providers
                            .entry(target)
                            .or_default()
                            .insert(name.clone());
                    }
                }
            }
        }
    }

    let mut formula_work = HashMap::new();
    let mut formula_refs: HashMap<String, HashSet<String>> = HashMap::new();
    let mut dependencies = HashMap::new();
    for (name, expr) in &pending {
        let analysis = analyze_from_ast_in_sheet(expr, &asts, &occupied, Some(name.as_str()))?;
        formula_work.insert(name.clone(), analysis.work);
        formula_refs.insert(name.clone(), analysis.cells.clone());
        let numeric_deps: HashSet<String> = analysis
            .cells
            .into_iter()
            .filter(|r| numeric_cells.contains(r))
            .collect();
        dependencies.insert(name.clone(), numeric_deps);
    }

    // Spill-provider edges. FILTER anchors soft-skip cycles; others fail-closed.
    let explicit_deps = dependencies.clone();
    let mut soft_spill_watch: Vec<(String, String)> = Vec::new(); // (reader, anchor)
    for (name, refs) in &formula_refs {
        for r in refs {
            if numeric_cells.contains(r) {
                continue;
            }
            if let Some(anchors) = spill_providers.get(r) {
                for anchor in anchors {
                    if anchor == name {
                        continue;
                    }
                    if depends_on(anchor, name, &explicit_deps) {
                        if asts.get(anchor).is_some_and(|expr| {
                            expr_is_filter_spill_source(expr, &asts, &occupied)
                        }) {
                            soft_spill_watch.push((name.clone(), anchor.clone()));
                        } else {
                            return Err(SpreadsheetError::CircularReference(name.clone()));
                        }
                    } else {
                        dependencies
                            .get_mut(name)
                            .expect("dependency entry")
                            .insert(anchor.clone());
                    }
                }
            }
        }
    }

    let layers = build_layers(&numeric_cells, &dependencies)?;
    let mut cache = HashMap::new();
    let mut spill_meta: HashMap<String, (usize, usize)> = HashMap::new();

    let filter_anchors: Vec<String> = pending
        .iter()
        .filter(|(_, expr)| expr_is_filter_spill_source(expr, &asts, &occupied))
        .map(|(name, _)| name.clone())
        .collect();

    for layer in layers {
        let use_parallel = match policy {
            ParallelPolicy::Force(forced) => forced,
            ParallelPolicy::Adaptive(thresholds) => {
                should_use_parallel(&layer, &formula_work, thresholds)
            }
        };
        let layer_results = evaluate_layer(
            &asts,
            &sources,
            &cache,
            &text_cells,
            &spill_meta,
            &layer,
            use_parallel,
        )?;
        let mut layer_results = layer_results;
        layer_results.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, value) in layer_results {
            apply_eval_to_cache(
                &name,
                value,
                &occupied,
                &mut cache,
                &mut text_cells,
                &mut spill_meta,
            )?;
        }

        let layer_set: HashSet<&str> = layer.iter().map(String::as_str).collect();
        // Only seed cells already written. Pulling a future-layer FILTER into the
        // wave would spill early; the later layer apply would then hit #SPILL!.
        let touched: Vec<String> = soft_spill_watch
            .iter()
            .filter(|(reader, anchor)| {
                layer_set.contains(reader.as_str()) || layer_set.contains(anchor.as_str())
            })
            .flat_map(|(reader, anchor)| [reader.clone(), anchor.clone()])
            .filter(|cell| cache.contains_key(cell) || text_cells.contains_key(cell))
            .collect();
        if !touched.is_empty() {
            fixup_soft_spill_closure(
                &touched,
                &soft_spill_watch,
                &dependencies,
                &asts,
                &sources,
                &mut text_cells,
                &occupied,
                &mut cache,
                &mut spill_meta,
            )?;
        }
    }

    // Empty-path if_empty may need cells evaluated after the FILTER first ran; refresh
    // FILTER anchors then propagate soft-spill watchers and their dependents.
    fixup_soft_spill_closure(
        &filter_anchors,
        &soft_spill_watch,
        &dependencies,
        &asts,
        &sources,
        &mut text_cells,
        &occupied,
        &mut cache,
        &mut spill_meta,
    )?;

    for (name, value) in cache {
        values.insert(name, CellValue::Number(value));
    }
    for (name, text) in text_cells {
        values.entry(name).or_insert(CellValue::Text(text));
    }

    Ok(values)
}

/// Re-evaluate `seeds` plus related soft-watched readers and transitive dependents.
#[allow(clippy::too_many_arguments)]
fn fixup_soft_spill_closure(
    seeds: &[String],
    soft_spill_watch: &[(String, String)],
    dependencies: &HashMap<String, HashSet<String>>,
    asts: &HashMap<String, Expr>,
    sources: &HashMap<String, String>,
    text_cells: &mut HashMap<String, String>,
    occupied: &HashSet<String>,
    cache: &mut HashMap<String, f64>,
    spill_meta: &mut HashMap<String, (usize, usize)>,
) -> Result<(), SpreadsheetError> {
    let mut wave: HashSet<String> = seeds.iter().cloned().collect();
    for (reader, anchor) in soft_spill_watch {
        if wave.contains(reader) || wave.contains(anchor) {
            wave.insert(reader.clone());
            wave.insert(anchor.clone());
        }
    }

    // Expand to cells that explicitly depend on anything in the wave.
    let mut growing = true;
    while growing {
        growing = false;
        for (cell, prereqs) in dependencies {
            if wave.contains(cell) {
                continue;
            }
            if prereqs.iter().any(|p| wave.contains(p)) {
                wave.insert(cell.clone());
                growing = true;
            }
        }
    }

    // Never evaluate cells that have not been applied yet (wrong layer order).
    wave.retain(|cell| cache.contains_key(cell));

    if wave.is_empty() {
        return Ok(());
    }

    let sub_deps: HashMap<String, HashSet<String>> = wave
        .iter()
        .map(|cell| {
            let prereqs = dependencies
                .get(cell)
                .map(|ps| {
                    ps.iter()
                        .filter(|p| wave.contains(*p))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            (cell.clone(), prereqs)
        })
        .collect();

    // Soft-skip feedback needs at most two topo sweeps: readers absorb the
    // grown spill, then anchors/dependents absorb updated readers. No CR valve.
    for _ in 0..2 {
        let layers = build_layers(&wave, &sub_deps)?;
        let mut any = false;
        for layer in layers {
            let mut layer = layer;
            layer.sort();
            for name in layer {
                let before = cell_fingerprint(name.as_str(), cache, text_cells, spill_meta);
                reevaluate_cell(
                    &name,
                    asts,
                    sources,
                    text_cells,
                    occupied,
                    cache,
                    spill_meta,
                )?;
                if cell_fingerprint(name.as_str(), cache, text_cells, spill_meta) != before {
                    any = true;
                }
            }
        }
        if !any {
            break;
        }
    }
    Ok(())
}

fn cell_fingerprint(
    name: &str,
    cache: &HashMap<String, f64>,
    text_cells: &HashMap<String, String>,
    spill_meta: &HashMap<String, (usize, usize)>,
) -> Vec<(String, u64)> {
    let mut keys = vec![name.to_string()];
    if let Some(&(h, w)) = spill_meta.get(name) {
        if let Some((col, row)) = parse_a1(name) {
            for dr in 0..h {
                for dc in 0..w {
                    keys.push(format_a1(col + dc as u32, row + dr as u32));
                }
            }
        }
    }
    keys.sort();
    keys.into_iter()
        .map(|k| {
            let bits = if let Some(n) = cache.get(&k) {
                n.to_bits()
            } else if let Some(t) = text_cells.get(&k) {
                // Stable-ish fingerprint for text formula results.
                let mut h = 0u64;
                for b in t.as_bytes() {
                    h = h.wrapping_mul(16777619).wrapping_add(u64::from(*b));
                }
                h
            } else {
                f64::NAN.to_bits()
            };
            (k, bits)
        })
        .collect()
}

fn reevaluate_cell(
    name: &str,
    asts: &HashMap<String, Expr>,
    sources: &HashMap<String, String>,
    text_cells: &mut HashMap<String, String>,
    occupied: &HashSet<String>,
    cache: &mut HashMap<String, f64>,
    spill_meta: &mut HashMap<String, (usize, usize)>,
) -> Result<(), SpreadsheetError> {
    let value = {
        let ctx = EvalContext {
            asts,
            sources,
            cache,
            text_cells,
            eval_cell: name,
            spill_meta,
            bindings: None,
        };
        evaluate_formula(&ctx, name)?
    };
    // apply_eval_to_cache clears any prior footprint for this anchor.
    apply_eval_to_cache(name, value, occupied, cache, text_cells, spill_meta)
}

/// True if `from` transitively depends on `to` following `deps[cell] → prerequisites`.
fn depends_on(from: &str, to: &str, deps: &HashMap<String, HashSet<String>>) -> bool {
    let mut stack = vec![from];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if node == to {
            return true;
        }
        if !seen.insert(node.to_string()) {
            continue;
        }
        if let Some(next) = deps.get(node) {
            for dep in next {
                stack.push(dep.as_str());
            }
        }
    }
    false
}

fn clear_anchor_spill_footprint(
    anchor: &str,
    cache: &mut HashMap<String, f64>,
    spill_meta: &mut HashMap<String, (usize, usize)>,
) {
    if let Some((h, w)) = spill_meta.remove(anchor) {
        if let Some((col, row)) = parse_a1(anchor) {
            for dr in 0..h {
                for dc in 0..w {
                    let target = format_a1(col + dc as u32, row + dr as u32);
                    if target != anchor {
                        cache.remove(&target);
                    }
                }
            }
        }
    }
}

fn prior_spill_targets(
    anchor: &str,
    spill_meta: &HashMap<String, (usize, usize)>,
) -> HashSet<String> {
    let mut out = HashSet::new();
    let Some(&(h, w)) = spill_meta.get(anchor) else {
        return out;
    };
    let Some((col, row)) = parse_a1(anchor) else {
        return out;
    };
    for dr in 0..h {
        for dc in 0..w {
            let target = format_a1(col + dc as u32, row + dr as u32);
            if target != anchor {
                out.insert(target);
            }
        }
    }
    out
}

/// Write a formula result into the cache, spilling arrays into empty A1 neighbors.
/// Idempotent for the same anchor. Validates the full footprint before mutating so
/// a `#SPILL!` leaves cache / spill_meta unchanged.
fn apply_eval_to_cache(
    anchor: &str,
    value: EvalValue,
    occupied: &HashSet<String>,
    cache: &mut HashMap<String, f64>,
    text_cells: &mut HashMap<String, String>,
    spill_meta: &mut HashMap<String, (usize, usize)>,
) -> Result<(), SpreadsheetError> {
    match value {
        EvalValue::Number(n) => {
            clear_anchor_spill_footprint(anchor, cache, spill_meta);
            text_cells.remove(anchor);
            cache.insert(anchor.to_string(), n);
            Ok(())
        }
        EvalValue::Text(s) => {
            clear_anchor_spill_footprint(anchor, cache, spill_meta);
            cache.remove(anchor);
            text_cells.insert(anchor.to_string(), s);
            Ok(())
        }
        EvalValue::Array(rows) => {
            let height = rows.len();
            let width = rows.first().map(|r| r.len()).unwrap_or(0);
            if height == 0 || width == 0 {
                return Err(SpreadsheetError::Value);
            }
            if height == 1 && width == 1 {
                clear_anchor_spill_footprint(anchor, cache, spill_meta);
                text_cells.remove(anchor);
                cache.insert(anchor.to_string(), rows[0][0]);
                return Ok(());
            }

            let Some((col, row)) = parse_a1(anchor) else {
                clear_anchor_spill_footprint(anchor, cache, spill_meta);
                text_cells.remove(anchor);
                cache.insert(anchor.to_string(), rows[0][0]);
                return Ok(());
            };

            let mut planned: Vec<(String, f64)> = Vec::with_capacity(height * width);
            for (dr, row_vals) in rows.iter().enumerate() {
                if row_vals.len() != width {
                    return Err(SpreadsheetError::Value);
                }
                for (dc, &n) in row_vals.iter().enumerate() {
                    planned.push((format_a1(col + dc as u32, row + dr as u32), n));
                }
            }

            let reusable = prior_spill_targets(anchor, spill_meta);
            for (target, _) in &planned {
                if target == anchor {
                    continue;
                }
                if occupied.contains(target) {
                    return Err(SpreadsheetError::Spill);
                }
                if (cache.contains_key(target) || text_cells.contains_key(target))
                    && !reusable.contains(target)
                {
                    return Err(SpreadsheetError::Spill);
                }
            }

            clear_anchor_spill_footprint(anchor, cache, spill_meta);
            text_cells.remove(anchor);
            for (target, n) in planned {
                text_cells.remove(&target);
                cache.insert(target, n);
            }
            spill_meta.insert(anchor.to_string(), (height, width));
            Ok(())
        }
    }
}

fn should_use_parallel(
    layer: &[String],
    work_by_cell: &HashMap<String, usize>,
    thresholds: ParallelThresholds,
) -> bool {
    let width = layer.len();
    let work: usize = layer
        .iter()
        .map(|cell| work_by_cell.get(cell).copied().unwrap_or(0))
        .sum();
    width >= thresholds.min_layer_width && work >= thresholds.min_layer_work
}

fn evaluate_layer(
    asts: &HashMap<String, Expr>,
    sources: &HashMap<String, String>,
    cache: &HashMap<String, f64>,
    text_cells: &HashMap<String, String>,
    spill_meta: &HashMap<String, (usize, usize)>,
    layer: &[String],
    use_parallel: bool,
) -> Result<Vec<(String, EvalValue)>, SpreadsheetError> {
    if use_parallel && layer.len() > 1 {
        layer
            .par_iter()
            .map(|name| {
                let ctx = EvalContext {
                    asts,
                    sources,
                    cache,
                    text_cells,
                    eval_cell: name.as_str(),
                    spill_meta,
                    bindings: None,
                };
                let value = evaluate_formula(&ctx, name)?;
                Ok((name.clone(), value))
            })
            .collect()
    } else {
        layer
            .iter()
            .map(|name| {
                let ctx = EvalContext {
                    asts,
                    sources,
                    cache,
                    text_cells,
                    eval_cell: name.as_str(),
                    spill_meta,
                    bindings: None,
                };
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
    use super::{should_use_parallel, ParallelThresholds};
    use std::collections::HashMap;

    #[test]
    fn narrow_light_layer_stays_sequential() {
        let layer: Vec<String> = (0..40).map(|i| format!("C{i}")).collect();
        let mut refs = HashMap::new();
        for name in &layer {
            refs.insert(name.clone(), 10);
        }
        assert!(!should_use_parallel(
            &layer,
            &refs,
            ParallelThresholds::default()
        ));
    }

    #[test]
    fn wide_heavy_layer_uses_parallel() {
        let thresholds = ParallelThresholds::default();
        let layer: Vec<String> = (0..thresholds.min_layer_width)
            .map(|i| format!("C{i}"))
            .collect();
        let refs_per = thresholds.min_layer_work / thresholds.min_layer_width;
        let mut refs = HashMap::new();
        for name in &layer {
            refs.insert(name.clone(), refs_per);
        }
        assert!(should_use_parallel(&layer, &refs, thresholds));
    }

    #[test]
    fn wide_but_light_layer_stays_sequential() {
        let thresholds = ParallelThresholds::default();
        let layer: Vec<String> = (0..thresholds.min_layer_width)
            .map(|i| format!("C{i}"))
            .collect();
        let mut refs = HashMap::new();
        for name in &layer {
            refs.insert(name.clone(), 1);
        }
        assert!(!should_use_parallel(&layer, &refs, thresholds));
    }

    #[test]
    fn custom_low_thresholds_enable_parallel_on_small_layer() {
        let thresholds = ParallelThresholds {
            min_layer_width: 2,
            min_layer_work: 4,
        };
        let layer = vec!["A1".into(), "A2".into()];
        let mut refs = HashMap::new();
        refs.insert("A1".into(), 2);
        refs.insert("A2".into(), 2);
        assert!(should_use_parallel(&layer, &refs, thresholds));
    }

    #[test]
    fn custom_high_thresholds_keep_default_sized_layer_sequential() {
        let thresholds = ParallelThresholds {
            min_layer_width: usize::MAX,
            min_layer_work: usize::MAX,
        };
        let defaults = ParallelThresholds::default();
        let layer: Vec<String> = (0..defaults.min_layer_width)
            .map(|i| format!("C{i}"))
            .collect();
        let refs_per = defaults.min_layer_work / defaults.min_layer_width;
        let mut refs = HashMap::new();
        for name in &layer {
            refs.insert(name.clone(), refs_per);
        }
        assert!(!should_use_parallel(&layer, &refs, thresholds));
    }
}
