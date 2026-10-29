//! JPEG decoding utilities for TIFF tile data.
//!
//! This module provides a thin wrapper around `zune-jpeg` for decoding
//! JPEG-compressed TIFF tiles. Unlike standalone JPEG images, TIFF stores
//! the intended color interpretation separately in the
//! `PhotometricInterpretation` tag. The decoder therefore requires the
//! corresponding [`Photometric`] value to apply the correct color transform
//! during decompression.
use std::io::Read;

use crate::error::WsiError;
use crate::tile::Photometric;
use image::{DynamicImage, GrayImage, RgbImage};
use zune_jpeg::JpegDecoder as ZuneDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;

#[derive(Debug, Clone, PartialEq)]
/// Decoder for JPEG-compressed TIFF tiles.
pub struct JpegDecoder;

impl JpegDecoder {
    /// Decodes a JPEG-compressed TIFF tile into a [`DynamicImage`].
    ///
    /// The supplied reader must yield a complete JPEG bitstream. For TIFF images
    /// using abbreviated JPEG streams (such as Aperio SVS), this is typically
    /// obtained from [`TileDirectory::jpeg_reader`](crate::tile::TileDirectory::jpeg_reader).
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
        mut reader: R,
        color_transform: &Photometric,
    ) -> Result<DynamicImage, WsiError> {
        let mut jpeg = Vec::new();
        reader.read_to_end(&mut jpeg)?;
        Self::decode_bytes(&jpeg, color_transform)
    }

    /// Same as [`JpegDecoder::decode`], for a JPEG bitstream already in memory.
    pub fn decode_bytes(
        jpeg: &[u8],
        color_transform: &Photometric,
    ) -> Result<DynamicImage, WsiError> {
        let mut decoder = ZuneDecoder::new(ZCursor::new(jpeg));
        decoder
            .decode_headers()
            .map_err(|_| WsiError::UnsupportedFormat)?;

        let input = decoder
            .input_colorspace()
            .ok_or(WsiError::InvalidMetadata)?;
        let output = match (input, color_transform) {
            (ColorSpace::Luma, _) => ColorSpace::Luma,
            // Without a marker saying otherwise, a 3-component JPEG is assumed
            // to be YCbCr. When the TIFF says the stream really holds RGB (as
            // in Aperio SVS), "keeping" YCbCr returns the stored components
            // untouched -- i.e. the RGB values, with no conversion applied.
            (ColorSpace::YCbCr, Photometric::RGB) => ColorSpace::YCbCr,
            _ => ColorSpace::RGB,
        };
        decoder.set_options(decoder.options().jpeg_set_out_colorspace(output));

        let pixels = decoder.decode().map_err(|_| WsiError::UnsupportedFormat)?;
        let info = decoder.info().ok_or(WsiError::InvalidMetadata)?;
        let (width, height) = (u32::from(info.width), u32::from(info.height));

        match output {
            ColorSpace::Luma => GrayImage::from_raw(width, height, pixels)
                .map(DynamicImage::ImageLuma8)
                .ok_or(WsiError::InvalidMetadata),
            _ => RgbImage::from_raw(width, height, pixels)
                .map(DynamicImage::ImageRgb8)
                .ok_or(WsiError::InvalidMetadata),
        }
    }
}
