//! Extraction options controlling how tiles are decoded and processed during
//! extraction.

use std::fmt;
use std::sync::Arc;

use crate::logging::ExtractionObserver;

/// Governs whether tiles are processed sequentially or concurrently across
/// multiple threads.
/// Future Options (TODO):
/// - OutputDirectory: Option<PathBuf> - specify output directory for extracted tiles
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
}

impl fmt::Debug for ExtractionOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtractionOptions")
            .field("parallelism", &self.parallelism)
            .field("min_tissue_fraction", &self.min_tissue_fraction)
            .field("observer", &self.observer.is_some())
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
        }
    }

    /// Process tiles concurrently using rayon's default thread pool.
    pub fn parallel() -> Self {
        Self {
            parallelism: Parallelism::Parallel(None),
            min_tissue_fraction: None,
            observer: None,
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
        }
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
        self.observer = Some(Arc::new(observer));
        self
    }

    /// Reports extraction events through [`SimpleLogging`](crate::logging::SimpleLogging),
    /// i.e. via the `log` crate.
    pub fn with_logging(self) -> Self {
        self.with_observer(crate::logging::SimpleLogging)
    }
}
