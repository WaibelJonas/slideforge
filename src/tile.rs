//! Tile directory and tile reading functionality for tiled TIFF images.
//!
//! This module provides [`TileDirectory`], which stores the location and
//! compression metadata for all tiles within a tiled TIFF image. It also
//! contains utilities for reading compressed tile data directly from disk.

use image::DynamicImage;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use tiff::decoder::Decoder;
use tiff::decoder::ifd::Value;
use tiff::tags::Tag;

use crate::Level;
use crate::error::WsiError;

/// photometric interpretation.
///
/// This describes how image pixels should be interpreted.
/// and serves to determine the correct color transformation
/// during JPEG decoding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Photometric {
    WhiteIsZero,
    BlackIsZero,
    RGB,
    Palette,
    TransparencyMask,
    CMYK,
    YCbCr,
    CIELab,
}

impl Photometric {
    /// Returns the JPEG color transform corresponding to this
    /// photometric interpretation.
    pub fn jpeg_color_transform(self) -> jpeg_decoder::ColorTransform {
        match self {
            Self::RGB => jpeg_decoder::ColorTransform::RGB,
            Self::YCbCr => jpeg_decoder::ColorTransform::YCbCr,

            // Fallback is RGB (honestly couldnt be bothered to exhaust this match)
            _ => jpeg_decoder::ColorTransform::RGB,
        }
    }
}

/// Helper function to convert from [`tiff::tags::Tag::PhotometricInterpretation`]
/// to the corresponding [`Photometric`]
impl TryFrom<&Value> for Photometric {
    type Error = WsiError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        let code = match value {
            Value::Short(v) => *v,
            _ => return Err(WsiError::UnsupportedFormat),
        };

        match code {
            0 => Ok(Self::WhiteIsZero),
            1 => Ok(Self::BlackIsZero),
            2 => Ok(Self::RGB),
            3 => Ok(Self::Palette),
            4 => Ok(Self::TransparencyMask),
            5 => Ok(Self::CMYK),
            6 => Ok(Self::YCbCr),
            8 => Ok(Self::CIELab),
            _ => Err(WsiError::UnsupportedFormat),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Directory describing all tiles within a tiled TIFF image.
///
/// Stores the file offsets, compressed byte counts, JPEG tables and
/// photometric interpretation required to locate and decode individual
/// tiles.
pub struct TileDirectory {
    offsets: Vec<u64>,
    byte_counts: Vec<u64>,
    jpeg_tables: Vec<u8>,
    photometric: Photometric,
}

impl TileDirectory {
    /// Reads the tile directory from a TIFF decoder and returns a [`TileDirectory`] instance.
    ///
    /// # Arguments
    /// * `decoder` - A [`Decoder`] instance pointing to an Pyramid level from which to read tile offsets and byte counts.
    ///
    /// # Returns
    /// A [`Result`] containing a [`TileDirectory`] instance on success, or a [`WsiError`] on failure.
    ///
    /// # Errors
    /// Returns [`WsiError`] if the tile offsets or byte counts cannot be read or if they are inconsistent.
    pub fn read_tile_directory(
        decoder: &mut Decoder<BufReader<File>>,
    ) -> Result<TileDirectory, WsiError> {
        let tile_offsets = decoder
            .find_tag(Tag::TileOffsets)?
            .ok_or(WsiError::UnsupportedFormat)?;

        let byte_counts = decoder
            .find_tag(Tag::TileByteCounts)?
            .ok_or(WsiError::UnsupportedFormat)?;

        let jpeg_tables = decoder
            .find_tag(Tag::JPEGTables)?
            .ok_or(WsiError::UnsupportedFormat)?;

        let photometric = decoder
            .find_tag(Tag::PhotometricInterpretation)?
            .ok_or(WsiError::UnsupportedFormat)?;
        let photometric = Photometric::try_from(&photometric)?;

        let tile_offsets = read_u64_list(&tile_offsets)?;
        let byte_counts = read_u64_list(&byte_counts)?;
        let jpeg_tables = read_u8_list(&jpeg_tables)?;

        if tile_offsets.len() != byte_counts.len() {
            return Err(WsiError::InvalidMetadata);
        }

        Ok(TileDirectory {
            offsets: tile_offsets,
            byte_counts: byte_counts,
            jpeg_tables: jpeg_tables,
            photometric: photometric,
        })
    }

    /// Returns the JPEG tables associated with this tile directory.
    pub fn jpeg_tables(&self) -> &[u8] {
        &self.jpeg_tables
    }

    /// Returns a reader over a complete JPEG bitstream for the given tile.
    ///
    /// Aperio SVS stores JPEG-compressed tiles as abbreviated JPEG streams.
    /// The quantization and Huffman tables are stored separately in the TIFF
    /// `JPEGTables` tag. This method reconstructs a complete JPEG stream by
    /// concatenating the JPEG tables (without the trailing EOI marker) with
    /// the tile data (without the leading SOI marker).
    ///
    /// See:
    /// - TIFF Technical Note #2: "TIFF Compression using JPEG", section
    ///   "JPEGTables Field".
    /// - OpenSlide `openslide-decode-tiff.c` for a reference implementation.
    pub fn jpeg_reader<'a>(&'a self, tile: &'a [u8]) -> impl Read + 'a {
        Cursor::new(&self.jpeg_tables[..self.jpeg_tables.len() - 2]).chain(Cursor::new(&tile[2..]))
    }

    /// Reconstructs a complete JPEG image for the specified tile.
    ///
    /// This should primarily be used as a debugging utility
    /// as the JPEG decoder consumes the JPEG bitstream (as returned by `jpeg_reader`) directly.
    pub fn reconstruct_jpeg(&self, tile: &[u8]) -> Result<Vec<u8>, WsiError> {
        let mut jpeg = Vec::new();
        self.jpeg_reader(tile).read_to_end(&mut jpeg)?;
        Ok(jpeg)
    }

    /// Returns the tile location (offset and byte count) for a given tile index.
    pub fn get_tile_location_by_index(&self, index: usize) -> Option<(u64, u64)> {
        Some((*self.offsets.get(index)?, *self.byte_counts.get(index)?))
    }

    /// Returns the photometric interpretation used.
    pub fn photometric(&self) -> &Photometric {
        &self.photometric
    }

    /// Returns the tile location (offset and byte count) for a given tile coordinate (tile_x, tile_y) and level.
    pub fn get_tile_location_by_coord(
        &self,
        tile_x: u32,
        tile_y: u32,
        level: &Level,
    ) -> Result<(u64, u64), WsiError> {
        let index = self.tile_index(tile_x, tile_y, level)?;
        self.get_tile_location_by_index(index)
            .ok_or(WsiError::TileIndexOutOfBounds)
    }

    /// Returns the tile index for a given tile coordinate (tile_x, tile_y) and level.
    fn tile_index(&self, tile_x: u32, tile_y: u32, level: &Level) -> Result<usize, WsiError> {
        if tile_x >= level.tiles_x() || tile_y >= level.tiles_y() {
            return Err(WsiError::TileIndexOutOfBounds);
        }

        Ok((tile_y * level.tiles_x() + tile_x) as usize)
    }

    /// Returns the total number of tiles in the directory.
    pub fn tile_count(&self) -> usize {
        self.offsets.len()
    }
}

pub struct Tile {
    image: DynamicImage,
    level: usize,
    tile_x: u32,
    tile_y: u32,
}

impl Tile {
    pub fn new(image: DynamicImage, level: usize, tile_x: u32, tile_y: u32) -> Tile {
        Tile {
            image,
            level,
            tile_x,
            tile_y,
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), WsiError> {
        Ok(self
            .image
            .save(path)
            .map_err(|_| WsiError::UnsupportedFormat)?)
    }

    pub fn tile_x(&self) -> u32 {
        self.tile_x
    }

    pub fn tile_y(&self) -> u32 {
        self.tile_y
    }

    pub fn image(&self) -> &DynamicImage {
        &self.image
    }

    pub fn level(&self) -> usize {
        self.level
    }
}

/// Helper function to read a list of u64 values from a TIFF tag value.
/// !TODO | This and read_u8_list should probably be moved into a generic utility function (maybe utils.rs)
fn read_u64_list(value: &Value) -> Result<Vec<u64>, WsiError> {
    match value {
        Value::List(values) => values
            .iter()
            .map(|v| match v {
                Value::Unsigned(v) => return Ok(*v as u64),
                _ => return Err(WsiError::UnsupportedFormat),
            })
            .collect(),
        _ => Err(WsiError::UnsupportedFormat),
    }
}

/// Helper function to read a list of u8 values from a TIFF tag value.
/// !TODO | This and read_u64_list should probably be moved into a generic utility function (maybe utils.rs)
fn read_u8_list(value: &Value) -> Result<Vec<u8>, WsiError> {
    match value {
        Value::List(values) => values
            .iter()
            .map(|v| match v {
                Value::Byte(v) => return Ok(*v),
                _ => return Err(WsiError::UnsupportedFormat),
            })
            .collect(),
        _ => Err(WsiError::UnsupportedFormat),
    }
}

/// Reads the tile bytes from a specified location in the TIFF file.
pub fn read_tile_bytes(path: &Path, offset: u64, byte_count: u64) -> Result<Vec<u8>, WsiError> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;

    let mut buffer = vec![0; byte_count as usize];
    file.read_exact(&mut buffer)?;

    Ok(buffer)
}
