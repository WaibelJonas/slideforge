//! Extraction options controlling how tiles are decoded and processed during
//! extraction.

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
#[derive(Debug, Clone, Default)]
pub struct ExtractionOptions {
    pub parallelism: Parallelism,
}

impl ExtractionOptions {
    /// Process tiles sequentially. Default.
    pub fn sequential() -> Self {
        Self {
            parallelism: Parallelism::Sequential,
        }
    }

    /// Process tiles concurrently using rayon's default thread pool.
    pub fn parallel() -> Self {
        Self {
            parallelism: Parallelism::Parallel(None),
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
        }
    }
}
