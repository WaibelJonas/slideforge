//! Tile image decoding. Currently just JPEG, via [`JpegDecoder`].

pub mod jpeg;
pub use jpeg::JpegDecoder;
