pub mod backend;
pub mod decoder;
pub mod error;
pub mod extraction;
pub mod metadata;
pub mod slide;
pub mod tile;

pub use extraction::{ExtractionOptions, Parallelism};
pub use metadata::{Dimensions, Level, Metadata, TileSize};
