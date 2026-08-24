use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use slideforge::ExtractionOptions;

#[derive(Parser)]
#[command(name = "slideforge", version, about = "Whole-slide image tile extraction")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Extract tiles from a single slide
    Extract(ExtractArgs),
    /// Extract tiles from every slide in a directory
    Dataset(DatasetArgs),
}

/// Which pyramid level to extract at. Exactly one of the two must be given.
#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct LevelSelect {
    /// Extract directly from a specific pyramid level (skips resizing)
    #[arg(long)]
    pub level: Option<usize>,
    /// Extract at the pyramid level closest to this target
    /// microns-per-pixel, resizing tiles to match exactly
    #[arg(long)]
    pub target_mpp: Option<f64>,
}

/// How to parallelize tile processing. At most one may be given; omitting
/// both uses rayon's default thread pool.
#[derive(Args)]
#[group(multiple = false)]
pub struct ParallelismArgs {
    /// Process tiles concurrently with a dedicated pool of this many threads
    #[arg(long)]
    pub threads: Option<usize>,
    /// Process tiles sequentially instead of in parallel
    #[arg(long)]
    pub sequential: bool,
}

/// Options controlling how tiles are decoded and processed, shared by
/// both subcommands.
#[derive(Args)]
pub struct PipelineArgs {
    #[command(flatten)]
    pub level: LevelSelect,
    #[command(flatten)]
    pub parallelism: ParallelismArgs,
    /// Minimum Otsu-derived tissue fraction a tile must have to be kept
    /// (disables tissue filtering if omitted)
    #[arg(long)]
    pub min_tissue_fraction: Option<f32>,
    /// Apply Reinhard stain normalization to kept tiles
    #[arg(long)]
    pub stain_normalize: bool,
    /// Suppress the progress bar
    #[arg(long, short = 'q')]
    pub quiet: bool,
}

impl PipelineArgs {
    pub fn build(&self) -> ExtractionOptions {
        let mut options = if self.parallelism.sequential {
            ExtractionOptions::sequential()
        } else if let Some(threads) = self.parallelism.threads {
            ExtractionOptions::parallel_with_threads(threads)
        } else {
            ExtractionOptions::parallel()
        };

        if let Some(level) = self.level.level {
            options = options.with_level(level);
        } else if let Some(target_mpp) = self.level.target_mpp {
            options = options.with_target_mpp(target_mpp);
        }

        if let Some(min_fraction) = self.min_tissue_fraction {
            options = options.with_min_tissue_fraction(min_fraction);
        }
        if self.stain_normalize {
            options = options.with_stain_normalization();
        }
        if !self.quiet {
            options = options.with_progress_bar();
        }
        options
    }
}

#[derive(Args)]
pub struct ExtractArgs {
    /// Path to the slide file
    pub slide: PathBuf,
    #[command(flatten)]
    pub pipeline: PipelineArgs,
    /// Save loose .jpg tiles to this directory
    #[arg(long)]
    pub tile_dir: Option<PathBuf>,
    /// Path to write the .tfrecord to (default: <slide-stem>.tfrecord)
    #[arg(long)]
    pub tfrecord_file: Option<PathBuf>,
    /// Write a PDF extraction report to this file
    #[arg(long)]
    pub report: Option<PathBuf>,
}

#[derive(Args)]
pub struct DatasetArgs {
    /// Directory containing slide files
    pub dataset_dir: PathBuf,
    /// Directory to write per-slide outputs under
    pub output_root: PathBuf,
    #[command(flatten)]
    pub pipeline: PipelineArgs,
    /// Save loose .jpg tiles per slide
    #[arg(long)]
    pub tiles: bool,
    /// Subfolder (relative to each slide's own directory) tiles are saved
    /// under, when --tiles is set
    #[arg(long)]
    pub tile_subdir: Option<PathBuf>,
    /// Write a combined dataset PDF report to this file
    #[arg(long)]
    pub report: Option<PathBuf>,
}
