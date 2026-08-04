use thiserror::Error;

#[derive(Error, Debug)]
pub enum WsiError {
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    #[error("TIFF parsing error")]
    Tiff(#[from] tiff::TiffError),

    #[error("Unsupported file format")]
    UnsupportedFormat,

    #[error("Invalid slide metadata")]
    InvalidMetadata,

    #[error("Tile index out of bounds")]
    TileIndexOutOfBounds,

    #[error("Level index out of bounds")]
    LevelIndexOutOfBounds,

    #[error("Failed to build thread pool")]
    ThreadPool(#[from] rayon::ThreadPoolBuildError),

    #[error("Failed to generate tile extraction report")]
    Report,
}
