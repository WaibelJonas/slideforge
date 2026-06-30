/// Representation of a single Whole Slide Image (WSI) along with its metadata.
use std::path::{Path, PathBuf};

use crate::backend::svs::parse_slide;
use crate::error::WsiError;
use crate::metadata::Metadata;
use crate::tile::{TileDirectory, read_tile_bytes};

#[derive(Debug, Clone, PartialEq)]
/// Represents a single WSI, along with its metadata.
pub struct Slide {
    path: PathBuf,
    metadata: Metadata,
    tile_directories: Vec<TileDirectory>,
}

impl Slide {
    /// Opens a WSI from the specified path and returns a corresponding
    /// [`Slide`] instance.
    ///
    /// # Arguments
    /// * `path` - Path to the WSI file.
    ///
    /// # Returns
    /// A [`Result`] containing a [`Slide`] instance on success, or a [`WsiError`] on failure.
    ///
    /// # Errors
    /// Returns [`WsiError`] if the file cannot be opened or the metadata cannot be read.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WsiError> {
        let path = path.as_ref();

        let parsed = parse_slide(path)?;

        Ok(Self {
            path: path.to_path_buf(),
            metadata: parsed.metadata,
            tile_directories: parsed.tile_directories,
        })
    }

    /// Returns the path of the WSI.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Returns a reference to the metadata associated with the WSI.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns the number of pyramid levels in the WSI.
    pub fn level_count(&self) -> usize {
        self.metadata.level_count()
    }

    /// Reads and returns the tile at the specified level and coordinates.
    ///
    /// Given a level index and tile coordinates (tile_x, tile_y), this function
    /// retrieves the corresponding tile bytes from the WSI.
    ///
    /// # Arguments
    /// * `level_idx` - The index of the pyramid level.
    /// * `tile_x` - The x-coordinate of the tile.
    /// * `tile_y` - The y-coordinate of the tile.
    ///
    /// # Returns
    /// A [`Result`] containing a vector of bytes representing the tile on success, or a [`WsiError`] on failure.
    ///
    /// # Errors
    /// Returns [`WsiError::LevelIndexOutOfBounds`] if the specified level index is invalid.
    pub fn read_tile(
        &self,
        level_idx: usize,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<Vec<u8>, WsiError> {
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let directory = self
            .tile_directories
            .get(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let (offset, byte_count) = directory.get_tile_location_by_coord(tile_x, tile_y, level)?;

        read_tile_bytes(&self.path, offset, byte_count)
    }
}
