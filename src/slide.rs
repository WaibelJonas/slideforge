use crate::backend::svs::parse_slide;
use crate::decoder::JpegDecoder;
use crate::error::WsiError;
use crate::metadata::Metadata;
use crate::tile::{Tile, TileDirectory, read_tile_bytes};
/// Representation of a single Whole Slide Image (WSI) along with its metadata.
use std::path::{Path, PathBuf};

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

    /// Reads and returns the raw tile bytes at the specified level and coordinates.
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
    pub fn read_tile_raw(
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

    /// Reads the raw tile bytes at the specified level and coordinates and decodes them into a [`Tile`].
    ///
    /// Given a level index and tile coordinates (tile_x, tile_y), this function
    /// retrieves the corresponding tile bytes from the WSI, decodes them and returns them in the form of
    /// a loaded [`Tile`]. This function should preferably be used over `read_tile`
    ///
    /// # Arguments
    /// * `level_idx` - The index of the pyramid level.
    /// * `tile_x` - The x-coordinate of the tile.
    /// * `tile_y` - The y-coordinate of the tile.
    ///
    /// # Returns
    /// A [`Result`] containing a [`Tile`] representing the tile on success, or a [`WsiError`] on failure.
    ///
    /// # Errors
    /// Returns [`WsiError::LevelIndexOutOfBounds`] if the specified level index is invalid.
    pub fn decode_tile(
        &self,
        level_idx: usize,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<Tile, WsiError> {
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let directory = &self.tile_directories[level.level() as usize];
        let color_transform = directory.photometric();

        let (offset, byte_count) = directory.get_tile_location_by_coord(tile_x, tile_y, level)?;

        let tile = read_tile_bytes(&self.path, offset, byte_count)?;
        let decoded_tile = JpegDecoder::decode(directory.jpeg_reader(&tile), color_transform)?;

        Ok(Tile::new(decoded_tile, level_idx, tile_x, tile_y))
    }

    pub fn tile_coords(
        &self,
        level_idx: usize,
    ) -> Result<impl Iterator<Item = (u32, u32)> + '_, WsiError> {
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let tiles_x = level.tiles_x();
        let tiles_y = level.tiles_y();

        Ok((0..tiles_y).flat_map(move |y| (0..tiles_x).map(move |x| (x, y))))
    }

    pub fn tiles(
        &self,
        level_idx: usize,
    ) -> Result<impl Iterator<Item = Result<Tile, WsiError>> + '_, WsiError> {
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let tiles_x = level.tiles_x();
        let tiles_y = level.tiles_y();

        Ok((0..tiles_y)
            .flat_map(move |y| (0..tiles_x).map(move |x| self.decode_tile(level_idx, x, y))))
    }

    pub fn extract<F>(&self, level_idx: usize, mut f: F) -> Result<(), WsiError>
    where
        F: FnMut(Tile) -> Result<(), WsiError>,
    {
        self.tiles(level_idx)?.try_for_each(|tile| {
            let tile = tile?;
            f(tile)
        })
    }

    pub fn extract_to_dir(
        &self,
        level_idx: usize,
        output: impl AsRef<Path>,
    ) -> Result<(), WsiError> {
        let output = output.as_ref();

        std::fs::create_dir_all(output)?;

        self.extract(level_idx, |tile| {
            tile.save(output.join(format!("{}_{}.jpg", tile.tile_x(), tile.tile_y())))
        })?;
        Ok(())
    }
}
