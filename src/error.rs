use thiserror::Error;

#[derive(Error, Debug)]
pub enum WsiError {
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    #[error("TIFF parsing error")]
    Tiff(#[from] tiff::TiffError),

    #[error("unsupported file format")]
    UnsupportedFormat,

    #[error("invalid slide metadata")]
    InvalidMetadata,
}
