//! Compares wall-clock time for sequential vs. parallel tile extraction.
//!
//! Usage: cargo run --release --example bench_extraction [path] [level]
//!
//! run with --release

use std::env;
use std::time::{Duration, Instant};

use slideforge::ExtractionOptions;
use slideforge::slide::{Slide, SlideOutputs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let path = args
        .next()
        .unwrap_or_else(|| "assets/example.svs".to_string());
    let level: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let slide = Slide::open(&path)?;
    let tile_count = slide.tile_coords(level)?.count();
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    println!("{path}: level {level}, {tile_count} tiles, {cores} logical cores\n");

    let modes: Vec<(&str, ExtractionOptions)> = vec![
        (
            "sequential",
            ExtractionOptions::sequential().with_level(level),
        ),
        (
            "parallel (1 thread)",
            ExtractionOptions::parallel_with_threads(1).with_level(level),
        ),
        (
            "parallel (half cores)",
            ExtractionOptions::parallel_with_threads((cores / 2).max(1)).with_level(level),
        ),
        (
            "parallel (default pool)",
            ExtractionOptions::parallel().with_level(level),
        ),
    ];

    let mut baseline: Option<Duration> = None;

    for (label, options) in modes {
        let start = Instant::now();
        slide.extract(&options, &SlideOutputs::default(), |_tile| Ok(()))?;
        let elapsed = start.elapsed();

        let throughput = tile_count as f64 / elapsed.as_secs_f64();
        let speedup = baseline.map(|b| b.as_secs_f64() / elapsed.as_secs_f64());
        baseline.get_or_insert(elapsed);

        match speedup {
            Some(speedup) => println!(
                "{label:<24} {elapsed:>10.3?}  {throughput:>8.1} tiles/s  {speedup:>5.2}x vs sequential"
            ),
            None => println!("{label:<24} {elapsed:>10.3?}  {throughput:>8.1} tiles/s"),
        }
    }

    Ok(())
}
