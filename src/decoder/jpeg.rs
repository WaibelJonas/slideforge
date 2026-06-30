//! JPEG decoding utilities for TIFF tile data.
//!
//! This module provides a thin wrapper around `jpeg-decoder` for decoding
//! JPEG-compressed TIFF tiles. Unlike standalone JPEG images, TIFF stores
//! the intended color interpretation separately in the
//! `PhotometricInterpretation` tag. The decoder therefore requires the
//! corresponding [`Photometric`] value to apply the correct color transform
//! during decompression.
use std::io::Read;

use crate::error::WsiError;
use crate::tile::Photometric;
use image::{DynamicImage, ImageBuffer, RgbImage};
use jpeg_decoder::Decoder;
use jpeg_decoder::PixelFormat;

#[derive(Debug, Clone, PartialEq)]
/// Decoder for JPEG-compressed TIFF tiles.
pub struct JpegDecoder;

impl JpegDecoder {
    /// Decodes a JPEG-compressed TIFF tile into a [`DynamicImage`].
    ///
    /// The supplied reader must yield a complete JPEG bitstream. For TIFF images
    /// using abbreviated JPEG streams (such as Aperio SVS), this is typically
    /// obtained from [`TileDirectory::jpeg_reader`].
    ///
    /// The TIFF photometric interpretation is used to configure the JPEG decoder's
    /// color transform. This is required because the JPEG codestream alone does
    /// not always contain sufficient information to determine the intended color
    /// space.
    ///
    /// # Arguments
    ///
    /// * `reader` - Reader providing the JPEG bitstream.
    /// * `color_transform` - TIFF photometric interpretation used during decoding.
    ///
    /// # Returns
    ///
    /// A [`DynamicImage`] containing the decoded tile.
    ///
    /// # Errors
    ///
    /// Returns [`WsiError`] if the JPEG stream cannot be decoded, contains
    /// unsupported pixel formats, or if the decoded image buffer is invalid.
    pub fn decode<R: Read>(
        reader: R,
        color_transform: &Photometric,
    ) -> Result<DynamicImage, WsiError> {
        let mut decoder = Decoder::new(reader);
        decoder.set_color_transform(color_transform.jpeg_color_transform());

        let pixels = decoder.decode().map_err(|_| WsiError::UnsupportedFormat)?;

        let info = decoder.info().ok_or(WsiError::InvalidMetadata)?;

        match info.pixel_format {
            PixelFormat::RGB24 => {
                let image: RgbImage =
                    ImageBuffer::from_raw(info.width.into(), info.height.into(), pixels)
                        .ok_or(WsiError::InvalidMetadata)?;

                Ok(DynamicImage::ImageRgb8(image))
            }

            PixelFormat::L8 => {
                let image =
                    image::GrayImage::from_raw(info.width.into(), info.height.into(), pixels)
                        .ok_or(WsiError::InvalidMetadata)?;

                Ok(DynamicImage::ImageLuma8(image))
            }

            _ => Err(WsiError::UnsupportedFormat),
        }
    }
}
