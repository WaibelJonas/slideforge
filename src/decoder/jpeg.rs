use image::RgbImage;

use crate::{decoder::TileDecoder, error::WsiError};

#[derive(Debug, Clone, PartialEq)]
pub struct JpegDecoder {}

impl TileDecoder for JpegDecoder {
    fn decode(
        &self,
        _tile_bytes: &[u8],
        _jpeg_tables: Option<&[u8]>,
        _width: u32,
        _height: u32,
    ) -> Result<RgbImage, WsiError> {
        todo!()
    }
}
