//! Calibrate parallel thresholds by sweeping layer width × refs/cell.
//!
//! ```text
//! cargo run --release --example bench_threshold
//! ```

use std::time::{Duration, Instant};

use calc_spreadsheet::calculate_spreadsheet_with_parallel;

const WIDTHS: &[usize] = &[64, 128, 256, 512, 1024, 2048, 4096];
const REFS: &[usize] = &[5, 10, 20, 40, 80];
const WARMUP: usize = 3;
const ITERATIONS: usize = 8;

fn main() {
    println!("width × refs/cell speedup grid (seq_avg / par_avg)");
    println!("speedup > 1.0 means parallel is faster");
    println!();

    print!("{:>6}", "w\\r");
    for &refs in REFS {
        print!(" {:>8}", refs);
    }
    println!();

    let mut first_stable: Option<(usize, usize, f64)> = None;

    for &width in WIDTHS {
        print!("{width:>6}");
        for &refs in REFS {
            let owned = build_sheet(width, refs);
            let cells: Vec<(&str, &str)> = owned
                .iter()
                .map(|(n, e)| (n.as_str(), e.as_str()))
                .collect();

            for _ in 0..WARMUP {
                let _ = calculate_spreadsheet_with_parallel(&cells, false).unwrap();
                let _ = calculate_spreadsheet_with_parallel(&cells, true).unwrap();
            }

            let seq = measure(&cells, false);
            let par = measure(&cells, true);
            let speedup = seq.as_secs_f64() / par.as_secs_f64();
            print!(" {speedup:>8.2}");

            if first_stable.is_none() && speedup >= 1.1 {
                first_stable = Some((width, refs, speedup));
            }
        }
        println!();
    }

    println!();
    if let Some((width, refs, speedup)) = first_stable {
        let work = width * refs; // one referencing layer's total refs (approx)
        println!(
            "first grid point with speedup >= 1.1: width={width}, refs/cell={refs}, speedup={speedup:.2}"
        );
        println!("suggested THRESHOLDS (exact crossing; add margin if you want extra safety):");
        println!("  WIDTH_THRESHOLD = {width}");
        println!("  WORK_THRESHOLD  = {work}  (= width * refs for one layer)");
        println!("NOTE: values are host-local — re-run on the target machine before shipping.");
    } else {
        println!("no grid point reached speedup >= 1.1 on this machine/run");
    }
}

fn measure(cells: &[(&str, &str)], parallel: bool) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..ITERATIONS {
        let started = Instant::now();
        let _ = calculate_spreadsheet_with_parallel(cells, parallel).unwrap();
        total += started.elapsed();
    }
    total / ITERATIONS as u32
}

/// Leaf layer of `width` constants + one referencing layer of `width` cells,
/// each SUMming `refs` leaves (shallow sheet → one wide parallelizable layer).
fn build_sheet(width: usize, refs: usize) -> Vec<(String, String)> {
    let mut cells = Vec::with_capacity(width * 2);
    for i in 0..width {
        cells.push((format!("L0_{i}"), (i % 97).to_string()));
    }
    for i in 0..width {
        let args: Vec<String> = (0..refs)
            .map(|j| format!("L0_{}", (i + j) % width))
            .collect();
        cells.push((format!("L1_{i}"), format!("=SUM({})", args.join(", "))));
    }
    cells
}
