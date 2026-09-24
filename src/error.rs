//! The crate's single error type, [`WsiError`].

use thiserror::Error;

/// Errors returned by slideforge's slide-reading and tile-extraction API.
#[derive(Error, Debug)]
pub enum WsiError {
    /// Reading or writing a file failed.
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    /// The underlying TIFF container could not be parsed.
    #[error("TIFF parsing error")]
    Tiff(#[from] tiff::TiffError),

    /// The file isn't a supported WSI format, or an expected TIFF tag was
    /// missing or had an unexpected type.
    #[error("Unsupported file format")]
    UnsupportedFormat,

    /// Slide metadata was present but internally inconsistent (e.g.
    /// mismatched tile offset/byte-count counts) or a required field (e.g.
    /// microns-per-pixel for a target-MPP extraction) was missing.
    #[error("Invalid slide metadata")]
    InvalidMetadata,

    /// A requested tile coordinate is outside the level's tile grid.
    #[error("Tile index out of bounds")]
    TileIndexOutOfBounds,

    /// A requested pyramid level index doesn't exist on this slide.
    #[error("Level index out of bounds")]
    LevelIndexOutOfBounds,

    /// Building a dedicated rayon thread pool for
    /// [`Parallelism::Parallel`](crate::Parallelism::Parallel) with an
    /// explicit thread count failed.
    #[error("Failed to build thread pool")]
    ThreadPool(#[from] rayon::ThreadPoolBuildError),

    /// Rendering a PDF extraction report failed.
    #[error("Failed to generate tile extraction report")]
    Report,
}
