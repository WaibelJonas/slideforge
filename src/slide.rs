use crate::backend::svs::parse_slide;
use crate::decoder::JpegDecoder;
use crate::error::WsiError;
use crate::extraction::{ExtractionOptions, Parallelism};
use crate::metadata::Metadata;
use crate::tile::{Tile, TileDirectory, read_tile_bytes, read_tile_bytes_at};
/// Representation of a single Whole Slide Image (WSI) along with its metadata.
use rayon::prelude::*;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Capacity of the buffer used for tile reads.
///
/// Chosen to span several compressed tiles so that reading tiles in storage
/// order is served from the buffer instead of one syscall per tile.
const TILE_BUFFER_CAPACITY: usize = 1024 * 1024;

#[derive(Debug)]
/// Represents a single WSI, along with its metadata.
pub struct Slide {
    path: PathBuf,
    metadata: Metadata,
    tile_directories: Vec<TileDirectory>,
    // Buffered reader for sequential tile reads during extraction.
    // Wrapped in Mutex so that multiple threads can decode tiles concurrently.
    reader: Mutex<BufReader<File>>,
    // File handle for random access reads during concurrent tile decoding.
    random_access_file: File,
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

        let reader = BufReader::with_capacity(TILE_BUFFER_CAPACITY, File::open(path)?);
        let random_access_file = File::open(path)?;

        Ok(Self {
            path: path.to_path_buf(),
            metadata: parsed.metadata,
            tile_directories: parsed.tile_directories,
            reader: Mutex::new(reader),
            random_access_file,
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

    /// Locates the tile directory entry for a given level and tile coordinate.
    /// 
    /// # Arguments
    /// * `level_idx` - The index of the pyramid level.
    /// * `tile_x` - The x-coordinate of the tile.
    /// * `tile_y` - The y-coordinate of the tile.
    /// 
    /// # Returns
    /// A [`Result`] containing a tuple of the tile directory, offset, and byte count on success, or a [`WsiError`] on failure.
    fn locate_tile(
        &self,
        level_idx: usize,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<(&TileDirectory, u64, u64), WsiError> {
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let directory = self
            .tile_directories
            .get(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let (offset, byte_count) = directory.get_tile_location_by_coord(tile_x, tile_y, level)?;

        Ok((directory, offset, byte_count))
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
        let (_, offset, byte_count) = self.locate_tile(level_idx, tile_x, tile_y)?;

        let mut reader = self.reader.lock().expect("Mutex lock failed!");
        read_tile_bytes(&mut reader, offset, byte_count)
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

        let (directory, offset, byte_count) = self.locate_tile(level_idx, tile_x, tile_y)?;
        let mut reader = self.reader.lock().expect("Mutex lock failed!");

        let decoded_tile = {
            let tile = read_tile_bytes(&mut reader, offset, byte_count);
            let color_transform = directory.photometric();
            JpegDecoder::decode(directory.jpeg_reader(&tile?), color_transform)?
        };

        Ok(Tile::new(decoded_tile, level_idx, tile_x, tile_y))
    }

    /// Reads the raw tile bytes at the specified level and coordinates and decodes them into a [`Tile`].
    ///
    /// Given a level index and tile coordinates (tile_x, tile_y), this function
    /// retrieves the corresponding tile bytes from the WSI, decodes them and returns them in the form of
    /// a loaded [`Tile`]. This function should preferably be used over `read_tile`
    /// In contrast to `decode_tile`, this function is safe to call concurrently from multiple threads.
    /// 
    /// # Arguments
    /// * `level_idx` - The index of the pyramid level.
    /// * `tile_x` - The x-coordinate of the tile.
    /// * `tile_y` - The y-coordinate of the tile.
    /// 
    /// # Returns
    /// A [`Result`] containing a [`Tile`] representing the tile on success, or a
    /// [`WsiError`] on failure.
    fn decode_tile_concurrent(
        &self,
        level_idx: usize,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<Tile, WsiError> {
        let (directory, offset, byte_count) = self.locate_tile(level_idx, tile_x, tile_y)?;
        let color_transform = directory.photometric();

        let tile = read_tile_bytes_at(&self.random_access_file, offset, byte_count)?;
        let decoded_tile = JpegDecoder::decode(directory.jpeg_reader(&tile), color_transform)?;

        Ok(Tile::new(decoded_tile, level_idx, tile_x, tile_y))
    }

    /// Returns an iterator over the tile coordinates (tile_x, tile_y) for the specified level.
    /// The iterator yields tuples of (tile_x, tile_y) for each tile in the specified level, in row-major order.
    ///
    /// # Arguments
    /// * `level_idx` - The index of the pyramid level.
    /// 
    /// # Returns
    /// A [`Result`] containing an iterator over tile coordinates on success, or a [`WsiError`] on failure.
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

    /// Decodes every tile at `level_idx` and passes each to `f`, according to
    /// `options`.
    ///
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

    /// Decodes every tile at `level_idx` and passes each to `f`, according to
    /// `options`.
    ///
    /// `f` must be safe to call concurrently: [`Parallelism::Parallel`]
    /// invokes it from multiple worker threads at once.
    ///
    /// # Errors
    /// Returns [`WsiError::LevelIndexOutOfBounds`] if the specified level
    /// index is invalid, or [`WsiError::ThreadPool`] if a dedicated thread
    /// pool could not be built for [`Parallelism::Parallel`] with an
    /// explicit thread count.
    pub fn extract<F>(
        &self, 
        level_idx: usize,
        options: &ExtractionOptions,
        f: F
    ) -> Result<(), WsiError>
    where
        F: Fn(Tile) -> Result<(), WsiError> + Sync,
    {
        let coords: Vec<(u32, u32)> = self.tile_coords(level_idx)?.collect();

        match options.parallelism {
            Parallelism::Sequential => coords
                .into_iter()
                .try_for_each(|(x, y)| f(self.decode_tile(level_idx, x, y)?)),
            Parallelism::Parallel(threads) => {
                let run = || {
                    coords
                        .into_par_iter()
                        .try_for_each(|(x, y)| f(self.decode_tile_concurrent(level_idx, x, y)?))
                };

                match threads {
                    Some(n) => rayon::ThreadPoolBuilder::new()
                        .num_threads(n)
                        .build()?
                        .install(run),
                    None => run(),
                }
            }
        }
    }

    /// Extracts every tile at `level_idx` into `output` as `{tile_x}_{tile_y}.jpg`,
    /// according to `options`.
    pub fn extract_to_dir(
        &self,
        level_idx: usize,
        output: impl AsRef<Path>,
        options: &ExtractionOptions,
    ) -> Result<(), WsiError> {
        let output = output.as_ref();

        std::fs::create_dir_all(output)?;

        self.extract(level_idx, options, |tile| {
            tile.save(output.join(format!("{}_{}.jpg", tile.tile_x(), tile.tile_y())))
        })
    }
}
