//! Extraction options controlling how tiles are decoded and processed during
//! extraction.

use std::sync::Arc;
use std::{fmt, path::PathBuf};

use crate::logging::{DualObserver, ExtractionObserver};

/// Governs whether tiles are processed sequentially or concurrently across
/// multiple threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parallelism {
    /// Decode and process tiles seuqnetially.
    Sequential,
    /// Decode and process tiles concurrently.
    ///
    /// `None` => default rayon thread pool,
    /// 'Some(n' => pool of `n` threads.
    Parallel(Option<usize>),
}

impl Default for Parallelism {
    fn default() -> Self {
        Self::Sequential
    }
}

// Governs the resolution level at which to extract tiles.
#[derive(Debug, Clone, Copy)]
pub enum ExtractionLevel {
    /// Extract directly from a specific pyramid level
    /// This skips processing steps such as resizing
    Index(usize),
    /// A Specific target resolution as given as a MPP value.
    /// The pyramid level which is closest to the target
    /// is then chosen, then tiles are resized to match
    /// the per pixel resolution
    TargetMpp(f64),
}

/// Options controlling tile extraction.
#[derive(Clone, Default)]
pub struct ExtractionOptions {
    pub parallelism: Parallelism,
    /// Minimum Otsu-derived tissue fraction a tile must have to be kept
    /// during extraction. Tiles below this fraction are skipped.
    ///
    /// `None` (the default) disables tissue filtering entirely.
    pub min_tissue_fraction: Option<f32>,
    /// Receives tissue-filter events during extraction. `None` (the
    /// default) means no events are reported anywhere.
    pub observer: Option<Arc<dyn ExtractionObserver>>,
    /// Whether to apply Reinhard stain normalization to kept tiles before
    /// they reach the extraction callback. Disabled (`false`) by default.
    pub normalize_stain: bool,
    /// The output directory to which to extract tiles
    pub output_dir: Option<PathBuf>,
    /// The file to which to write extracted tile information to (as a .tfrecord)
    pub tfrecord_file: Option<PathBuf>,
    /// Target extraction level
    pub extraction_level: Option<ExtractionLevel>,
}

impl fmt::Debug for ExtractionOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtractionOptions")
            .field("parallelism", &self.parallelism)
            .field("min_tissue_fraction", &self.min_tissue_fraction)
            .field("observer", &self.observer.is_some())
            .field("output_dir", &self.output_dir)
            .field("tfrecord_file", &self.tfrecord_file)
            .field("extraction_level", &self.extraction_level)
            .finish()
    }
}

impl ExtractionOptions {
    /// Process tiles sequentially. Default.
    pub fn sequential() -> Self {
        Self {
            parallelism: Parallelism::Sequential,
            min_tissue_fraction: None,
            observer: None,
            normalize_stain: false,
            output_dir: None,
            tfrecord_file: None,
            extraction_level: None,
        }
    }

    /// Process tiles concurrently using rayon's default thread pool.
    pub fn parallel() -> Self {
        Self {
            parallelism: Parallelism::Parallel(None),
            min_tissue_fraction: None,
            observer: None,
            normalize_stain: false,
            output_dir: None,
            tfrecord_file: None,
            extraction_level: None,
        }
    }

    /// Process tiles concurrently using a dedicated pool of `threads`
    /// threads
    ///
    /// # Arguments
    /// * `threads` - The number of threads to use for concurrent processing.
    pub fn parallel_with_threads(threads: usize) -> Self {
        Self {
            parallelism: Parallelism::Parallel(Some(threads)),
            min_tissue_fraction: None,
            observer: None,
            normalize_stain: false,
            output_dir: None,
            tfrecord_file: None,
            extraction_level: None,
        }
    }

    /// Add a output directory to which tiles are extracted
    ///
    /// # Arguments
    /// * `dir` - Directory to which to save tiles to
    pub fn with_output_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.output_dir = Some(dir.into());
        self
    }

    /// Skip tiles whose Otsu-derived tissue fraction is below `min_fraction`
    /// during extraction.
    ///
    /// # Arguments
    /// * `min_fraction` - The minimum tissue fraction a tile must have to be kept.
    pub fn with_min_tissue_fraction(mut self, min_fraction: f32) -> Self {
        self.min_tissue_fraction = Some(min_fraction);
        self
    }

    /// Reports extraction events (e.g. tissue-filter drops) through the
    /// given [`ExtractionObserver`].
    ///
    /// # Arguments
    /// * `observer` - The observer to report events to.
    pub fn with_observer(mut self, observer: impl ExtractionObserver + 'static) -> Self {
        self.add_observer(Arc::new(observer));
        self
    }

    /// Reports extraction events (e.g. tissue-filter drops) through the
    /// given shared [`ExtractionObserver`].
    ///
    /// # Arguments
    /// * `observer` - The observer to report events to.
    pub fn with_shared_observer(mut self, observer: Arc<dyn ExtractionObserver>) -> Self {
        self.add_observer(observer);
        self
    }

    /// Reports extraction events through [`SimpleLogging`](crate::logging::SimpleLogging),
    /// i.e. via the `log` crate.
    pub fn with_logging(self) -> Self {
        self.with_observer(crate::logging::SimpleLogging)
    }

    /// Reports extraction progress through a
    /// [`ProgressBar`](crate::logging::ProgressBar), i.e. a
    /// rudimentary text progress bar printed to stderr.
    pub fn with_progress_bar(self) -> Self {
        self.with_observer(crate::logging::ProgressBar::new())
    }

    /// Applies Reinhard stain normalization to tiles before extraction
    pub fn with_stain_normalization(mut self) -> Self {
        self.normalize_stain = true;
        self
    }

    /// Add a .tfrecord file to which extracted tiles are recorded
    pub fn with_tfrecord_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.tfrecord_file = Some(path.into());
        self
    }

    pub fn with_level(mut self, level_idx: usize) -> Self {
        self.extraction_level = Some(ExtractionLevel::Index(level_idx));
        self
    }

    pub fn with_target_mpp(mut self, target_mpp: f64) -> Self {
        self.extraction_level = Some(ExtractionLevel::TargetMpp(target_mpp));
        self
    }

    fn add_observer(&mut self, observer: Arc<dyn ExtractionObserver>) {
        self.observer = Some(match self.observer.take() {
            Some(existing) => Arc::new(DualObserver(existing, observer)),
            None => observer,
        })
    }
}
