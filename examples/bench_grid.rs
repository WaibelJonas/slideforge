//! Times `TileGeometry::Grid` extraction to a `.tfrecord`, with and without
//! tissue filtering -- the way AutoMIL drives slideforge.
//!
//! Usage: cargo run --release --example bench_grid [path] [tile_px] [tile_um] [min_tissue_fraction]
//!
//! run with --release

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use slideforge::ExtractionOptions;
use slideforge::slide::{Slide, SlideOutputs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let path = args
        .first()
        .cloned()
        .unwrap_or_else(|| "tests/fixtures/CMU-1-Small-Region.svs".to_string());
    let tile_px: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(299);
    let tile_um: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(149.0);
    let min_tissue: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.5);

    let slide = Slide::open(&path)?;
    let out_dir = env::temp_dir().join("slideforge_bench_grid");
    let outputs = SlideOutputs::default().with_tfrecord_file(out_dir.join("out.tfrecord"));

    println!("{path}: tile_px={tile_px}, tile_um={tile_um}\n");

    let modes: Vec<(String, ExtractionOptions)> = vec![
        (
            "no tissue filter".to_string(),
            ExtractionOptions::parallel(),
        ),
        (
            format!("min_tissue_fraction={min_tissue}"),
            ExtractionOptions::parallel().with_min_tissue_fraction(min_tissue),
        ),
    ];

    for (label, options) in modes {
        let options = options
            .with_level(0)
            .with_grid_geometry(tile_px, tile_um, None);
        let kept = AtomicUsize::new(0);

        let start = Instant::now();
        slide.extract(&options, &outputs, |_tile| {
            kept.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })?;
        let elapsed = start.elapsed();

        let kept = kept.into_inner();
        let size = std::fs::metadata(out_dir.join("out.tfrecord"))?.len() as f64 / 1e6;
        println!(
            "{label:<28} {elapsed:>10.3?}  {kept:>6} tiles kept  {:>8.1} tiles/s  {size:>7.1} MB",
            kept as f64 / elapsed.as_secs_f64()
        );
    }

    Ok(())
}
