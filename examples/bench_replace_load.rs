//! Load test: large sheet with many `__PLACEHOLDER__` tokens + replacement map.
//!
//! ```text
//! cargo run --release --example bench_replace_load
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};

use calc_spreadsheet::{
    calculate_spreadsheet, CalculateOptions, CellValue, ReplacementValue,
};

/// Leaf constants (no placeholders).
const LEAF_COUNT: usize = 1024;
/// Independent mid layer (each cell refs a few leaves + placeholders).
const MID_WIDTH: usize = 8192;
const MID_REFS: usize = 8;
/// Heavy dependent layer (wide + many refs) — may hit adaptive parallel.
const HEAVY_WIDTH: usize = 8192;
const HEAVY_REFS: usize = 40;
/// How many distinct `__Rnnn__` keys in the replacement map.
const REPLACE_KEYS: usize = 512;
/// Placeholders embedded per mid/heavy formula (cycled from the map).
const PLACEHOLDERS_PER_FORMULA: usize = 6;

const WARMUP: usize = 2;
const ITERATIONS: usize = 5;

fn main() {
    let (owned, replacements) = build_sheet_with_replacements();
    let cells: Vec<(&str, &str)> = owned
        .iter()
        .map(|(n, e)| (n.as_str(), e.as_str()))
        .collect();

    let placeholder_hits: usize = owned
        .iter()
        .map(|(_, e)| e.matches("__R").count())
        .sum();

    println!("=== bench_replace_load ===");
    println!("cells in:           {}", cells.len());
    println!("replacement keys:   {}", replacements.len());
    println!("placeholder tokens: ~{placeholder_hits} (in formulas before prepare)");
    println!("warmup={WARMUP}, iterations={ITERATIONS}");
    println!();

    let opts_with = CalculateOptions {
        replacements: Some(&replacements),
        ..Default::default()
    };
    let opts_empty = CalculateOptions::default();

    for _ in 0..WARMUP {
        let _ = calculate_spreadsheet(&cells, opts_with).unwrap();
        let _ = calculate_spreadsheet(&cells, opts_empty).unwrap();
    }

    let with_rep = measure(ITERATIONS, || {
        calculate_spreadsheet(&cells, opts_with).unwrap()
    });
    let no_rep = measure(ITERATIONS, || {
        calculate_spreadsheet(&cells, opts_empty).unwrap()
    });

    let sample = with_rep.last_outcome.as_ref().unwrap();
    let sample_key = format!("H0");
    let sample_val = sample.values.get(&sample_key).cloned();
    assert!(
        matches!(sample_val, Some(CellValue::Number(_))),
        "expected numeric H0, got {sample_val:?}"
    );
    assert!(sample.ignored_replacement_keys.is_empty());
    assert_eq!(sample.values.len(), no_rep.last_outcome.as_ref().unwrap().values.len());

    println!("--- with replacements (full path) ---");
    print_stats(&with_rep.stats);
    println!("  cells out: {}", sample.values.len());
    println!(
        "  sample {sample_key} = {:?}",
        sample_val.and_then(|v| v.as_number())
    );
    println!();
    println!("--- without replacements (same sheet; leftover __R*__ → 0 like blank) ---");
    print_stats(&no_rep.stats);
    println!();
    let overhead = with_rep.stats.avg.as_secs_f64() / no_rep.stats.avg.as_secs_f64();
    println!(
        "replace path / empty path (avg): {overhead:.2}x ({:.1} ms vs {:.1} ms)",
        with_rep.stats.avg.as_secs_f64() * 1000.0,
        no_rep.stats.avg.as_secs_f64() * 1000.0
    );
}

struct Stats {
    total: Duration,
    avg: Duration,
    min: Duration,
}

struct RunResult {
    stats: Stats,
    last_outcome: Option<calc_spreadsheet::SpreadsheetOutcome>,
}

fn measure<F>(iterations: usize, mut run: F) -> RunResult
where
    F: FnMut() -> calc_spreadsheet::SpreadsheetOutcome,
{
    let mut total = Duration::ZERO;
    let mut min = Duration::MAX;
    let mut last = None;
    for _ in 0..iterations {
        let started = Instant::now();
        let outcome = run();
        let elapsed = started.elapsed();
        total += elapsed;
        if elapsed < min {
            min = elapsed;
        }
        last = Some(outcome);
    }
    RunResult {
        stats: Stats {
            total,
            avg: total / iterations as u32,
            min,
        },
        last_outcome: last,
    }
}

fn print_stats(s: &Stats) {
    println!(
        "  avg {:.3} ms | min {:.3} ms | total {:.3} ms (n={ITERATIONS})",
        s.avg.as_secs_f64() * 1000.0,
        s.min.as_secs_f64() * 1000.0,
        s.total.as_secs_f64() * 1000.0
    );
}

fn build_sheet_with_replacements() -> (Vec<(String, String)>, HashMap<String, ReplacementValue>) {
    let mut replacements = HashMap::with_capacity(REPLACE_KEYS);
    for i in 0..REPLACE_KEYS {
        let key = format!("__R{i}__");
        // Mix numbers and a few text values used only in concat-style mid cells.
        if i % 17 == 0 {
            replacements.insert(key, ReplacementValue::from_text(format!("T{i}")));
        } else {
            replacements.insert(key, ReplacementValue::from_i64((i % 97) as i64 + 1));
        }
    }

    let mut cells = Vec::with_capacity(LEAF_COUNT + MID_WIDTH + HEAVY_WIDTH);

    for i in 0..LEAF_COUNT {
        cells.push((format!("L{i}"), (i % 97).to_string()));
    }

    // Mid: numeric SUM of leaves + product of placeholder numbers (skip text keys).
    for i in 0..MID_WIDTH {
        let mut args = Vec::with_capacity(MID_REFS + PLACEHOLDERS_PER_FORMULA);
        for j in 0..MID_REFS {
            args.push(format!("L{}", (i + j) % LEAF_COUNT));
        }
        let mut ph_sum = Vec::new();
        for j in 0..PLACEHOLDERS_PER_FORMULA {
            let k = (i + j * 3) % REPLACE_KEYS;
            // Prefer numeric keys (skip those that are text every 17).
            let k = if k % 17 == 0 { (k + 1) % REPLACE_KEYS } else { k };
            ph_sum.push(format!("__R{k}__"));
        }
        let expr = format!(
            "=SUM({},{})",
            args.join(","),
            ph_sum.join(",")
        );
        cells.push((format!("M{i}"), expr));
    }

    // A few concat cells to exercise & + text replacements.
    for i in 0..32 {
        let k = (i * 17) % REPLACE_KEYS;
        cells.push((
            format!("C{i}"),
            format!("=\"id=\"&__R{k}__&M{i}"),
        ));
    }

    // Heavy layer over mid cells + more placeholders.
    for i in 0..HEAVY_WIDTH {
        let mut args = Vec::with_capacity(HEAVY_REFS + PLACEHOLDERS_PER_FORMULA);
        for j in 0..HEAVY_REFS {
            args.push(format!("M{}", (i + j) % MID_WIDTH));
        }
        for j in 0..PLACEHOLDERS_PER_FORMULA {
            let k = (i * 5 + j) % REPLACE_KEYS;
            let k = if k % 17 == 0 { (k + 1) % REPLACE_KEYS } else { k };
            args.push(format!("__R{k}__"));
        }
        cells.push((format!("H{i}"), format!("=SUM({})", args.join(","))));
    }

    (cells, replacements)
}
