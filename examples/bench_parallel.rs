//! Dummy-data benchmark: sequential vs forced-parallel vs adaptive.
//!
//! Mixed sheet: many light/wide layers (hurt forced parallel) + one heavy layer
//! (helps adaptive). Goal: forced parallel slower than sequential, adaptive faster.
//!
//! ```text
//! cargo run --release --example bench_parallel
//! ```

use std::time::{Duration, Instant};

use calc_spreadsheet::{
    calculate_spreadsheet, calculate_spreadsheet_with_parallel, CellValue,
};

// Narrow-but-many light layers: forced rayon pays spawn cost; adaptive stays sequential.
const LEAF_COUNT: usize = 256;
const LIGHT_WIDTH: usize = 256;
const LIGHT_REFS: usize = 3;
const LIGHT_DEPTH: usize = 16; // L1..L16
// One wide heavy layer over adaptive thresholds (adaptive parallelizes only this).
const HEAVY_WIDTH: usize = 4096;
const HEAVY_REFS: usize = 80;
const WARMUP: usize = 3;
const ITERATIONS: usize = 8;

fn main() {
    let owned = build_mixed_sheet();
    let cells: Vec<(&str, &str)> = owned
        .iter()
        .map(|(name, expr)| (name.as_str(), expr.as_str()))
        .collect();

    let light_cells = LIGHT_WIDTH * LIGHT_DEPTH;
    println!("dummy sheet: {} cells (mixed light + heavy)", cells.len());
    println!("  L0 leaves={LEAF_COUNT} (constants, work=0)");
    println!(
        "  light layers L1..L{LIGHT_DEPTH}: width={LIGHT_WIDTH}, refs/cell={LIGHT_REFS} (×{light_cells} cells)"
    );
    println!(
        "  heavy layer L{}: width={HEAVY_WIDTH}, refs/cell={HEAVY_REFS}",
        LIGHT_DEPTH + 1
    );
    println!("  expect: forced-parallel < sequential < adaptive (speed)");

    for _ in 0..WARMUP {
        let _ = calculate_spreadsheet_with_parallel(&cells, false).unwrap();
        let _ = calculate_spreadsheet_with_parallel(&cells, true).unwrap();
        let _ = calculate_spreadsheet(&cells).unwrap();
    }

    let seq = measure_forced(&cells, false, ITERATIONS);
    let par = measure_forced(&cells, true, ITERATIONS);
    let adaptive = measure_adaptive(&cells, ITERATIONS);

    let seq_values = calculate_spreadsheet_with_parallel(&cells, false).unwrap();
    let par_values = calculate_spreadsheet_with_parallel(&cells, true).unwrap();
    let adaptive_values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(seq_values.len(), par_values.len());
    assert_eq!(seq_values.len(), adaptive_values.len());
    for (name, seq_value) in &seq_values {
        assert_eq!(seq_value, &par_values[name]);
        assert_eq!(seq_value, &adaptive_values[name]);
    }

    let sample_key = format!("L{}_0", LIGHT_DEPTH + 1);
    let sample = seq_values[&sample_key].as_number().unwrap();
    println!();
    println!(
        "correctness: OK ({} cells, {sample_key}={sample})",
        seq_values.len()
    );
    println!();
    println!("mode          total          avg            min");
    print_row("sequential", &seq);
    print_row("parallel", &par);
    print_row("adaptive", &adaptive);

    let speedup_par = seq.avg.as_secs_f64() / par.avg.as_secs_f64();
    let speedup_ada = seq.avg.as_secs_f64() / adaptive.avg.as_secs_f64();
    println!();
    println!("speedup (avg sequential / avg parallel): {speedup_par:.2}x  (<1 means parallel slower)");
    println!("speedup (avg sequential / avg adaptive): {speedup_ada:.2}x  (>1 means adaptive faster)");
}

fn print_row(label: &str, stats: &Stats) {
    println!(
        "{label:<12}  {:>10.3} ms  {:>10.3} ms  {:>10.3} ms",
        stats.total.as_secs_f64() * 1000.0,
        stats.avg.as_secs_f64() * 1000.0,
        stats.min.as_secs_f64() * 1000.0
    );
}

struct Stats {
    total: Duration,
    avg: Duration,
    min: Duration,
}

fn measure_forced(cells: &[(&str, &str)], parallel: bool, iterations: usize) -> Stats {
    measure(iterations, || {
        calculate_spreadsheet_with_parallel(cells, parallel).unwrap()
    })
}

fn measure_adaptive(cells: &[(&str, &str)], iterations: usize) -> Stats {
    measure(iterations, || calculate_spreadsheet(cells).unwrap())
}

fn measure<F>(iterations: usize, mut run: F) -> Stats
where
    F: FnMut() -> std::collections::HashMap<String, CellValue>,
{
    let mut total = Duration::ZERO;
    let mut min = Duration::MAX;

    for _ in 0..iterations {
        let started = Instant::now();
        let values = run();
        let elapsed = started.elapsed();
        assert!(matches!(values.values().next(), Some(CellValue::Number(_))));
        total += elapsed;
        if elapsed < min {
            min = elapsed;
        }
    }

    Stats {
        total,
        avg: total / iterations as u32,
        min,
    }
}

/// Many light/wide layers (forced parallel loses) + one heavy layer (adaptive wins).
fn build_mixed_sheet() -> Vec<(String, String)> {
    let mut cells =
        Vec::with_capacity(LEAF_COUNT + LIGHT_WIDTH * LIGHT_DEPTH + HEAVY_WIDTH);

    for i in 0..LEAF_COUNT {
        cells.push((format!("L0_{i}"), (i % 97).to_string()));
    }

    for depth in 1..=LIGHT_DEPTH {
        let prev = depth - 1;
        let prev_count = if prev == 0 { LEAF_COUNT } else { LIGHT_WIDTH };
        for i in 0..LIGHT_WIDTH {
            let args: Vec<String> = (0..LIGHT_REFS)
                .map(|j| format!("L{prev}_{}", (i + j) % prev_count))
                .collect();
            cells.push((format!("L{depth}_{i}"), format!("=SUM({})", args.join(", "))));
        }
    }

    let heavy_depth = LIGHT_DEPTH + 1;
    let prev = LIGHT_DEPTH;
    for i in 0..HEAVY_WIDTH {
        let args: Vec<String> = (0..HEAVY_REFS)
            .map(|j| format!("L{prev}_{}", (i + j) % LIGHT_WIDTH))
            .collect();
        cells.push((
            format!("L{heavy_depth}_{i}"),
            format!("=SUM({})", args.join(", ")),
        ));
    }

    cells
}
