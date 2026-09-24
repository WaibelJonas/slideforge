//! Fast whole slide image (WSI) tile extraction, with tissue detection and
//! stain normalization.
//!
//! Whole slide images are gigapixel-scale microscopy scans used in digital
//! pathology, stored as tiled, pyramidal (multi-resolution) TIFF files.
//! Slideforge currently reads the Aperio SVS format.
//!
//! [`Slide`](slide::Slide) opens a single WSI and extracts tiles at a
//! specific pyramid level or a target microns-per-pixel value, optionally
//! dropping background-only tiles (Otsu thresholding) and normalizing
//! stain color (Reinhard normalization) along the way. Extracted tiles can
//! be written out as loose JPEGs, a `.tfrecord` file for ML training
//! pipelines, and/or summarized in a PDF report.
//! [`Dataset`](dataset::Dataset) extends this across a whole directory of
//! slides at once.
//!
//! # Example
//!
//! ```no_run
//! use slideforge::slide::{Slide, SlideOutputs};
//! use slideforge::ExtractionOptions;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let slide = Slide::open("slide.svs")?;
//!
//! let options = ExtractionOptions::parallel()
//!     .with_target_mpp(0.5)
//!     .with_min_tissue_fraction(0.1)
//!     .with_stain_normalization();
//!
//! let outputs = SlideOutputs::default().with_tile_dir("tiles/");
//!
//! slide.extract(&options, &outputs, |tile| {
//!     println!("kept tile ({}, {})", tile.tile_x(), tile.tile_y());
//!     Ok(())
//! })?;
//! # Ok(())
//! # }
//! ```

pub mod backend;
pub mod dataset;
pub mod decoder;
pub mod error;
pub mod extraction;
pub mod filter;
pub mod logging;
pub mod metadata;
pub mod report;
pub mod slide;
pub mod stain;
pub mod tfrecord;
pub mod tile;

pub use extraction::{ExtractionOptions, Parallelism};
pub use filter::TissueMask;
pub use logging::{
    ExtractionObserver, ExtractionStats, ProgressBar, ReportCollector, SimpleLogging,
};
pub use metadata::{Dimensions, Level, Metadata, TileSize};
pub use report::ExtractionReport;
pub use stain::normalize_reinhard;
