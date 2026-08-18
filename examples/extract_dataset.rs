//! Runs dataset-level extraction across every slide in a directory,
//! demonstrating `Dataset::from_dir` + `Dataset::extract`.
//!
//! Usage: cargo run --release --example extract_dataset [dataset_dir] [output_root]
//!
//! Run with --release -- this walks all four sample slides (including two
//! ~200-400MB ones) at once. Set RUST_LOG=info for more verbose logging;
//! per-slide open/extraction failures are logged at `error` level from
//! inside `Dataset::extract` regardless.

use std::env;

use slideforge::ExtractionOptions;
use slideforge::dataset::{Dataset, DatasetOutputs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let mut args = env::args().skip(1);
    let dataset_dir = args.next().unwrap_or_else(|| "assets/dataset".to_string());
    let output_root = args
        .next()
        .unwrap_or_else(|| "assets/dataset_output".to_string());

    let dataset = Dataset::from_dir(&dataset_dir)?;
    println!("Found {} slide(s) in {dataset_dir}", dataset.len());

    let options = ExtractionOptions::parallel()
        .with_min_tissue_fraction(0.2)
        .with_stain_normalization()
        .with_progress_bar()
        .with_target_mpp(5.0);

    // Whether tiles/tfrecords get written at all -- and where -- is fully
    // controlled by `DatasetOutputs`, independent of `options` above.
    let outputs = DatasetOutputs {
        tiles: true,
        tfrecords: true,
        tile_subdir: Some("tiles".into()),
        report_path: Some(format!("{output_root}/report.pdf").into()),
    };
    let report = dataset.extract(&options, &output_root, outputs, |_slide, _tile| Ok(()))?;

    println!(
        "Done. {}/{} slides succeeded, {} tiles kept, {} dropped.",
        report.succeeded(),
        report.total_slides(),
        report.total_kept(),
        report.total_dropped()
    );
    println!("Per-slide tiles/tfrecords written under {output_root}/");
    println!("Dataset report written to {output_root}/report.pdf");
    Ok(())
}
