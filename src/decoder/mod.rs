use image::RgbImage;

use crate::error::WsiError;

pub mod jpeg;

pub trait TileDecoder {
    fn decode(
        &self,
        tile_bytes: &[u8],
        jpeg_tables: Option<&[u8]>,
        width: u32,
        height: u32,
    ) -> Result<RgbImage, WsiError>;
}
