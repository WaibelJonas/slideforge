//! Tile directory and tile reading functionality for TIFF images.
//! 
//! [`TileDirectory`] provides methods to read and manage tile offsets and byte counts from a TIFF image.
//! The module also includes functions to read tile bytes from a specified location in the TIFF file.

use std::path::Path;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::fs::File;
use tiff::decoder::Decoder;
use tiff::decoder::ifd::Value;
use tiff::tags::Tag;

use crate::Level;
use crate::error::WsiError;

#[derive(Debug, Clone, PartialEq)]
// Represents a tile directory in a TIFF image, containing offsets and byte counts for each tile.
pub struct TileDirectory {
    offsets: Vec<u64>,
    byte_counts: Vec<u64>,
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

        let tile_offsets = read_u64_list(&tile_offsets)?;
        let byte_counts = read_u64_list(&byte_counts)?;

        if tile_offsets.len() != byte_counts.len() {
            return Err(WsiError::InvalidMetadata);
        }

        Ok(
            TileDirectory {
                offsets: tile_offsets,
                byte_counts: byte_counts,
            }
        )
    }

    /// Returns the tile location (offset and byte count) for a given tile index.
    pub fn get_tile_location_by_index(
        &self,
        index: usize,
    ) -> Option<(u64, u64)> {
        Some(
            (
                *self.offsets.get(index)?,
                *self.byte_counts.get(index)?,
            )
        )
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

        Ok(
            (tile_y * level.tiles_x() + tile_x) as usize
        )
    }

    /// Returns the total number of tiles in the directory.
    pub fn tile_count(&self) -> usize {
        self.offsets.len()
    }
}

/// Helper function to read a list of u64 values from a TIFF tag value.
fn read_u64_list(value: &Value) -> Result<Vec<u64>, WsiError> {
    match value {
        Value::List(values) => {
            values.iter().map(|v| {
                match v {
                    Value::Unsigned(v) => return Ok(*v as u64),
                    _ => return Err(WsiError::UnsupportedFormat),
                }
            }).collect()
        },
        _ => Err(WsiError::UnsupportedFormat),
    }
}

/// Reads the tile bytes from a specified location in the TIFF file.
pub fn read_tile_bytes(
    path: &Path,
    offset: u64,
    byte_count: u64,
) -> Result<Vec<u8>, WsiError> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;

    let mut buffer = vec![0; byte_count as usize];
    file.read_exact(&mut buffer)?;

    Ok(buffer)
}