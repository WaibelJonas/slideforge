mod cli;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::cli::{Cli, Command, DatasetArgs, ExtractArgs};
use clap::Parser;
use slideforge::{
    dataset::{Dataset, DatasetOutputs},
    error::WsiError,
    slide::{Slide, SlideOutputs},
};

fn main() -> ExitCode {
    env_logger::init();
    let result = match Cli::parse().command {
        Command::Extract(extraction_args) => run_slide_extraction(extraction_args),
        Command::Dataset(extraction_args) => run_dataset_extraction(extraction_args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn default_tfrecord_name(slide_path: &Path) -> PathBuf {
    let stem = slide_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("slide");
    PathBuf::from(format!("{stem}.tfrecord"))
}

pub fn run_slide_extraction(args: ExtractArgs) -> Result<(), WsiError> {
    let slide = Slide::open(&args.slide)?;
    let options = args.pipeline.build();
    let outputs = SlideOutputs {
        tfrecord_file: Some(
            args.tfrecord_file
                .unwrap_or_else(|| default_tfrecord_name(&args.slide)),
        ),
        tile_dir: args.tile_dir,
        report_path: args.report,
    };
    slide.extract(&options, &outputs, |_tile| Ok(()))
}

fn run_dataset_extraction(args: DatasetArgs) -> Result<(), WsiError> {
    let dataset = Dataset::from_dir(&args.dataset_dir)?;
    println!(
        "Found {} slide(s) in {}",
        dataset.len(),
        args.dataset_dir.display()
    );

    if args.tile_subdir.is_some() && !args.tiles {
        eprintln!("warning: --tile-subdir has no effect without --tiles");
    }

    let options = args.pipeline.build();
    let outputs = DatasetOutputs {
        tiles: args.tiles,
        tfrecords: true,
        tile_subdir: args.tile_subdir,
        report_path: args.report,
    };
    let report = dataset.extract(&options, &args.output_root, outputs, |_slide, _tile| Ok(()))?;
    println!(
        "Done. {}/{} slides succeeded, {} tiles kept, {} dropped.",
        report.succeeded(),
        report.total_slides(),
        report.total_kept(),
        report.total_dropped()
    );
    Ok(())
}
