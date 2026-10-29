//! Whole slide image (WSI) reading and tile extraction.
//!
//! [`Slide`] represents a single opened WSI: its pyramid metadata and an
//! open file handle for reading tiles. [`Slide::extract`] is the main
//! entry point for pulling tiles out of it, with tissue filtering, stain
//! normalization, and resolution targeting layered in via
//! [`ExtractionOptions`].

use crate::Level;
use crate::backend::svs::parse_slide;
use crate::decoder::JpegDecoder;
use crate::error::WsiError;
use crate::extraction::{ExtractionLevel, ExtractionOptions, Parallelism, TileGeometry};
use crate::filter::{TissueMask, grayscale_histogram, otsu_threshold_from_histogram};
use crate::logging::{DualObserver, ExtractionObserver, ExtractionStats, ReportCollector};
use crate::metadata::Metadata;
use crate::tfrecord::TfRecordWriter;
use crate::tile::{Tile, TileDirectory, read_tile_bytes, read_tile_bytes_at};
use crate::{report, tfrecord};
use image::{DynamicImage, GrayImage, RgbImage};
use rayon::prelude::*;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Capacity of the buffer used for tile reads.
///
/// Chosen to span several compressed tiles so that reading tiles in storage
/// order is served from the buffer instead of one syscall per tile.
const TILE_BUFFER_CAPACITY: usize = 1024 * 1024;

/// Below this, resize is skipped entirely: the resolved
/// level's native resolution is already close enough to the target.
const RESIZE_SCALE_EPSILON: f64 = 1e-3;

/// Number of fixed-size windows (size `window_px`, stride `stride_px`)
/// that fit entirely within `level_dim`. Integer division floors, so a
/// trailing partial window is dropped rather than padded -- matches
/// slideflow's convention, unlike `Native`'s boundary-tile padding.
fn window_count(level_dim: u32, window_px: u32, stride_px: u32) -> u32 {
    if window_px > level_dim {
        return 0;
    }
    (level_dim - window_px) / stride_px + 1
}

/// Truncates a size in (fractional) pixels to whole pixels, matching
/// slideflow's `int(tile_um / mpp)`. The epsilon keeps a size that is
/// meant to be exact (e.g. `n * mpp / mpp`) from flooring one pixel short
/// due to float error. Never returns 0.
fn whole_pixels(px: f64) -> u32 {
    ((px + 1e-6).floor() as u32).max(1)
}

/// Runs `job` on `pool`, or on the current (default) rayon pool if `None`.
fn install<T: Send>(pool: Option<&rayon::ThreadPool>, job: impl FnOnce() -> T + Send) -> T {
    match pool {
        Some(pool) => pool.install(job),
        None => job(),
    }
}

/// Copies the part of `tile` that overlaps `canvas` into it. Both origins
/// are the images' top-left corners in the same (level) pixel space.
fn blit(
    canvas: &mut RgbImage,
    canvas_origin: (u32, u32),
    tile: &RgbImage,
    tile_origin: (u32, u32),
) {
    let x0 = canvas_origin.0.max(tile_origin.0);
    let y0 = canvas_origin.1.max(tile_origin.1);
    let x1 = (canvas_origin.0 + canvas.width()).min(tile_origin.0 + tile.width());
    let y1 = (canvas_origin.1 + canvas.height()).min(tile_origin.1 + tile.height());
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let row_bytes = (x1 - x0) as usize * 3;
    let (canvas_stride, tile_stride) = (canvas.width() as usize * 3, tile.width() as usize * 3);
    let canvas_x = (x0 - canvas_origin.0) as usize * 3;
    let tile_x = (x0 - tile_origin.0) as usize * 3;
    let tile_pixels: &[u8] = tile;
    let canvas_pixels: &mut [u8] = canvas;
    for y in y0..y1 {
        let dst = (y - canvas_origin.1) as usize * canvas_stride + canvas_x;
        let src = (y - tile_origin.1) as usize * tile_stride + tile_x;
        canvas_pixels[dst..dst + row_bytes].copy_from_slice(&tile_pixels[src..src + row_bytes]);
    }
}

/// The lowest-resolution pyramid level in grayscale, used as a
/// low-resolution tissue mask for the extraction level.
struct TissueOverview {
    gray: GrayImage,
    /// Otsu threshold derived from the whole overview
    threshold: u8,
    /// Overview pixels per extraction-level pixel, per axis
    scale_x: f64,
    scale_y: f64,
}

impl TissueOverview {
    /// Fraction of tissue (at or below `threshold`) in the overview pixels
    /// covering the given extraction-level rectangle. Partially covered
    /// overview pixels count fully, and a rectangle always covers at least
    /// one pixel.
    fn tissue_fraction(&self, x: u32, y: u32, width: u32, height: u32) -> f32 {
        let (gray_w, gray_h) = self.gray.dimensions();
        if gray_w == 0 || gray_h == 0 {
            return 1.0;
        }
        let span = |start: u32, len: u32, scale: f64, max: u32| {
            let lo = ((start as f64 * scale).floor() as u32).min(max - 1);
            let hi = (((start + len) as f64 * scale).ceil() as u32).clamp(lo + 1, max);
            lo..hi
        };
        let xs = span(x, width, self.scale_x, gray_w);
        let ys = span(y, height, self.scale_y, gray_h);

        let total = xs.len() * ys.len();
        let tissue = ys
            .flat_map(|py| xs.clone().map(move |px| (px, py)))
            .filter(|&(px, py)| self.gray.get_pixel(px, py).0[0] <= self.threshold)
            .count();
        tissue as f32 / total as f32
    }
}

/// Small tile grid paramater wrapper for later use in slide::extract
/// TODO! | Should probably be placed somewhere else
struct GridParams {
    tile_px: u32,
    window_px: u32,
    stride_px: u32,
}

/// Destination paths for the outputs a single [`Slide::extract`] run can
/// produce. Each field independently enables its output when `Some`;
/// `None` (the default) disables it.
#[derive(Debug, Clone, Default)]
pub struct SlideOutputs {
    /// Directory to save loose `.jpg` tiles to.
    pub tile_dir: Option<PathBuf>,
    /// File to write a `.tfrecord` to.
    pub tfrecord_file: Option<PathBuf>,
    /// File to write a single-slide PDF extraction report to.
    pub report_path: Option<PathBuf>,
}

impl SlideOutputs {
    pub fn with_tile_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.tile_dir = Some(dir.into());
        self
    }

    pub fn with_tfrecord_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.tfrecord_file = Some(path.into());
        self
    }

    pub fn with_report_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.report_path = Some(path.into());
        self
    }
}

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
    ) -> Result<(&TileDirectory, &Level, u64, u64), WsiError> {
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let directory = self
            .tile_directories
            .get(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let (offset, byte_count) = directory.get_tile_location_by_coord(tile_x, tile_y, level)?;

        Ok((directory, level, offset, byte_count))
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
        let (_, _, offset, byte_count) = self.locate_tile(level_idx, tile_x, tile_y)?;

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
        let (directory, level, offset, byte_count) = self.locate_tile(level_idx, tile_x, tile_y)?;
        let origin_x =
            ((tile_x * level.tile_size().width) as f64 * level.downsample_factor()).round() as u32;
        let origin_y =
            ((tile_y * level.tile_size().height) as f64 * level.downsample_factor()).round() as u32;

        let mut reader = self.reader.lock().expect("Mutex lock failed!");

        let decoded_tile = {
            let tile = read_tile_bytes(&mut reader, offset, byte_count);
            let color_transform = directory.photometric();
            JpegDecoder::decode(directory.jpeg_reader(&tile?), color_transform)?
        };

        Ok(Tile::new(
            decoded_tile,
            level_idx,
            tile_x,
            tile_y,
            origin_x,
            origin_y,
        ))
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
        let (directory, level, offset, byte_count) = self.locate_tile(level_idx, tile_x, tile_y)?;
        let origin_x =
            ((tile_x * level.tile_size().width) as f64 * level.downsample_factor()).round() as u32;
        let origin_y =
            ((tile_y * level.tile_size().height) as f64 * level.downsample_factor()).round() as u32;

        let color_transform = directory.photometric();

        let tile = read_tile_bytes_at(&self.random_access_file, offset, byte_count)?;
        let decoded_tile = JpegDecoder::decode(directory.jpeg_reader(&tile), color_transform)?;

        Ok(Tile::new(
            decoded_tile,
            level_idx,
            tile_x,
            tile_y,
            origin_x,
            origin_y,
        ))
    }

    /// Reads an arbitrary image region and stitches together native tiles that overlap with it.
    ///
    /// A region that extends partially or fully past the level's real
    /// dimensions is not an error -- the uncovered canvas area is left
    /// black, mirroring how native boundary tiles are already padded.
    ///
    /// # Arguments
    /// * `level_idx` - The index of the pyramid level.
    /// * `x` - The x-coordinate of the region's top-left corner, in `level_idx`'s own pixel space.
    /// * `y` - The y-coordinate of the region's top-left corner, in `level_idx`'s own pixel space.
    /// * `width` - The width of the region, in pixels.
    /// * `height` - The height of the region, in pixels.
    ///
    /// # Returns
    /// A [`Result`] containing a [`DynamicImage`] representing the image region on success, or a
    /// [`WsiError`] on failure.
    ///
    /// # Errors
    /// Returns [`WsiError::LevelIndexOutOfBounds`] if the specified level index is invalid.
    pub fn read_region(
        &self,
        level_idx: usize,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<DynamicImage, WsiError> {
        if width == 0 || height == 0 {
            return Ok(DynamicImage::ImageRgb8(RgbImage::new(width, height)));
        };

        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let tile_size = level.tile_size();
        let mut canvas = RgbImage::new(width, height);

        // Tile index at x y
        let tile_x0 = x / tile_size.width;
        let tile_y0 = y / tile_size.height;
        // Tile index at x + width y + height
        let tile_x1 = (x + width - 1) / tile_size.width;
        let tile_y1 = (y + height - 1) / tile_size.height;

        for ty in tile_y0..=tile_y1.min(level.tiles_y().saturating_sub(1)) {
            for tx in tile_x0..=tile_x1.min(level.tiles_x().saturating_sub(1)) {
                let tile = self.decode_tile_rgb(level_idx, tx, ty, true)?;
                blit(
                    &mut canvas,
                    (x, y),
                    &tile,
                    (tx * tile_size.width, ty * tile_size.height),
                );
            }
        }

        Ok(DynamicImage::ImageRgb8(canvas))
    }

    /// Decodes a native tile as RGB, cropped to its valid (non-padding)
    /// area. `concurrent` picks the positional-read path, which is safe to
    /// call from several threads at once.
    fn decode_tile_rgb(
        &self,
        level_idx: usize,
        tile_x: u32,
        tile_y: u32,
        concurrent: bool,
    ) -> Result<RgbImage, WsiError> {
        let tile = if concurrent {
            self.decode_tile_concurrent(level_idx, tile_x, tile_y)?
        } else {
            self.decode_tile(level_idx, tile_x, tile_y)?
        };
        Ok(self.cropped_image(level_idx, &tile)?.to_rgb8())
    }

    /// Cuts every [`TileGeometry::Grid`] window kept by `keep` out of the
    /// level and passes it to `process`, at window size (not yet resized to
    /// `params.tile_px`).
    ///
    /// Windows are handled one row at a time: every native tile the row's
    /// kept windows overlap is decoded exactly once and cached until no
    /// later row needs it, instead of each window decoding its own
    /// overlapping tiles (with 256px native tiles and ~300px windows,
    /// that's ~4.7 decodes per window, each native tile decoded ~3 times).
    /// The cache holds at most the native rows one window row spans.
    #[allow(clippy::too_many_arguments)]
    fn extract_grid<K, P>(
        &self,
        level_idx: usize,
        params: &GridParams,
        cols: u32,
        rows: u32,
        parallel: bool,
        keep: K,
        process: P,
    ) -> Result<(), WsiError>
    where
        K: Fn(u32, u32) -> bool + Sync,
        P: Fn(Tile) -> Result<(), WsiError> + Sync,
    {
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;
        let (tile_w, tile_h) = (level.tile_size().width, level.tile_size().height);
        let downsample_factor = level.downsample_factor();
        let window_px = params.window_px;

        let mut cache: HashMap<(u32, u32), RgbImage> = HashMap::new();

        for row in 0..rows {
            let y = row * params.stride_px;
            let (ty0, ty1) = (y / tile_h, (y + window_px - 1) / tile_h);
            let tile_cols = |col: u32| {
                let x = col * params.stride_px;
                x / tile_w..=(x + window_px - 1) / tile_w
            };

            // Window rows only move down, so native rows above this one are done
            cache.retain(|&(_, ty), _| ty >= ty0);

            let kept: Vec<u32> = (0..cols).filter(|&col| keep(col, row)).collect();
            if kept.is_empty() {
                continue;
            }

            // In storage (row-major) order, for the sequential buffered reader
            let mut missing: Vec<(u32, u32)> = (ty0..=ty1)
                .flat_map(|ty| {
                    kept.iter()
                        .flat_map(|&col| tile_cols(col))
                        .map(move |tx| (tx, ty))
                })
                .filter(|key| !cache.contains_key(key))
                .collect();
            missing.sort_unstable_by_key(|&(tx, ty)| (ty, tx));
            missing.dedup();

            let decode = |&(tx, ty): &(u32, u32)| {
                self.decode_tile_rgb(level_idx, tx, ty, parallel)
                    .map(|tile| ((tx, ty), tile))
            };
            let decoded: Vec<_> = if parallel {
                missing.par_iter().map(decode).collect::<Result<_, _>>()?
            } else {
                missing.iter().map(decode).collect::<Result<_, _>>()?
            };
            cache.extend(decoded);

            let cut = |&col: &u32| -> Result<(), WsiError> {
                let x = col * params.stride_px;
                let mut window = RgbImage::new(window_px, window_px);
                for ty in ty0..=ty1 {
                    for tx in tile_cols(col) {
                        if let Some(tile) = cache.get(&(tx, ty)) {
                            blit(&mut window, (x, y), tile, (tx * tile_w, ty * tile_h));
                        }
                    }
                }
                process(Tile::new(
                    DynamicImage::ImageRgb8(window),
                    level_idx,
                    col,
                    row,
                    (x as f64 * downsample_factor).round() as u32,
                    (y as f64 * downsample_factor).round() as u32,
                ))
            };
            if parallel {
                kept.par_iter().try_for_each(cut)?;
            } else {
                kept.iter().try_for_each(cut)?;
            }
        }

        Ok(())
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

    fn cropped_image<'a>(
        &self,
        level_idx: usize,
        tile: &'a Tile,
    ) -> Result<Cow<'a, DynamicImage>, WsiError> {
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;
        let tile_size = level.tile_size();
        let (valid_width, valid_height) = level.valid_tile_dimensions(tile.tile_x(), tile.tile_y());

        // Check whether tile exceeds the valid dimensions of whether it got padded
        if valid_width == tile_size.width && valid_height == tile_size.height {
            Ok(Cow::Borrowed(tile.image()))
        } else {
            Ok(Cow::Owned(tile.image().crop_imm(
                0,
                0,
                valid_width,
                valid_height,
            )))
        }
    }

    /// Stitches the lowest-resolution pyramid level into a single image,
    /// decoding its tiles on the current rayon pool when `parallel`.
    fn lowest_level_image(&self, parallel: bool) -> Result<RgbImage, WsiError> {
        let level_idx = self.level_count() - 1;
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;
        let tile_size = level.tile_size();

        let coords: Vec<(u32, u32)> = self.tile_coords(level_idx)?.collect();
        let decode = |&(x, y): &(u32, u32)| {
            self.decode_tile_rgb(level_idx, x, y, parallel)
                .map(|tile| ((x, y), tile))
        };
        let tiles: Vec<_> = if parallel {
            coords.par_iter().map(decode).collect::<Result<_, _>>()?
        } else {
            coords.iter().map(decode).collect::<Result<_, _>>()?
        };

        let dimensions = level.dimensions();
        let mut canvas = RgbImage::new(dimensions.width, dimensions.height);
        for ((x, y), tile) in &tiles {
            blit(
                &mut canvas,
                (0, 0),
                tile,
                (x * tile_size.width, y * tile_size.height),
            );
        }
        Ok(canvas)
    }

    /// Builds a [`TissueOverview`] of the lowest-resolution pyramid level
    /// for classifying regions of `level_idx`.
    ///
    /// Its single Otsu threshold comes from the aggregated grayscale
    /// histogram of the whole level. Tiles are classified against this
    /// shared reference point instead of each deriving its own threshold in
    /// isolation, which would let Otsu invent a spurious tissue/background
    /// split on a tile that is actually uniform background (or uniform
    /// tissue).
    fn tissue_overview(
        &self,
        level_idx: usize,
        parallel: bool,
    ) -> Result<TissueOverview, WsiError> {
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;
        let gray = DynamicImage::ImageRgb8(self.lowest_level_image(parallel)?).into_luma8();
        let threshold = otsu_threshold_from_histogram(&grayscale_histogram(&gray));

        Ok(TissueOverview {
            scale_x: gray.width() as f64 / level.dimensions().width as f64,
            scale_y: gray.height() as f64 / level.dimensions().height as f64,
            gray,
            threshold,
        })
    }

    /// Resolves the index of the pyramid level
    /// best suited to resolve to the target resolution
    /// specified by [`ExtractionLevel`]
    pub(crate) fn resolve_resolution_level(
        &self,
        extraction_level: &Option<ExtractionLevel>,
    ) -> Result<usize, WsiError> {
        match extraction_level {
            Some(ExtractionLevel::Index(idx)) => Ok(*idx),
            Some(ExtractionLevel::TargetMpp(target)) => self
                .metadata
                .best_level_for_target_mpp(*target)
                .ok_or(WsiError::InvalidMetadata),
            None => Err(WsiError::InvalidMetadata),
        }
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
        options: &ExtractionOptions,
        outputs: &SlideOutputs,
        f: F,
    ) -> Result<(), WsiError>
    where
        F: Fn(Tile) -> Result<(), WsiError> + Sync,
    {
        let level_idx = self.resolve_resolution_level(&options.extraction_level)?;
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let grid_params = match &options.geometry {
            TileGeometry::Grid {
                tile_px,
                tile_um,
                stride_um,
            } => {
                let base_mpp = self
                    .metadata
                    .microns_per_pixel()
                    .ok_or(WsiError::InvalidMetadata)?;
                let level_mpp = base_mpp * level.downsample_factor();
                Some(GridParams {
                    tile_px: *tile_px,
                    window_px: whole_pixels(tile_um / level_mpp),
                    stride_px: whole_pixels(stride_um.unwrap_or(*tile_um) / level_mpp),
                })
            }
            TileGeometry::Native => None,
        };

        // A tile's physical footprint in base-level pixels. Depends on the tile geometry
        let (footprint_w, footprint_h) = match &grid_params {
            Some(params) => {
                let footprint =
                    (params.window_px as f64 * level.downsample_factor()).round() as u32;
                (footprint, footprint)
            }
            None => (
                (level.tile_size().width as f64 * level.downsample_factor()).round() as u32,
                (level.tile_size().height as f64 * level.downsample_factor()).round() as u32,
            ),
        };

        // Computing the resize scale
        let resize_factor = match (&options.extraction_level, &grid_params) {
            (Some(ExtractionLevel::TargetMpp(target)), None) => {
                let base_mpp = self
                    .metadata
                    .microns_per_pixel()
                    .ok_or(WsiError::InvalidMetadata)?;
                let level_mpp = base_mpp * level.downsample_factor();
                Some(level_mpp / target)
            }
            _ => None,
        };

        // Grid windows, or native tiles, per row and column
        let (cols, rows) = match &grid_params {
            Some(params) => (
                window_count(level.dimensions().width, params.window_px, params.stride_px),
                window_count(
                    level.dimensions().height,
                    params.window_px,
                    params.stride_px,
                ),
            ),
            None => (level.tiles_x(), level.tiles_y()),
        };
        let total_tiles = cols as usize * rows as usize;
        let dropped_tiles = AtomicUsize::new(0);

        let slide_name = self
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("slide")
            .to_string();

        if let Some(dir) = &outputs.tile_dir {
            std::fs::create_dir_all(dir)?;
        }
        if let Some(dir) = outputs.tfrecord_file.as_deref().and_then(Path::parent) {
            std::fs::create_dir_all(dir)?;
        }
        if let Some(dir) = outputs.report_path.as_deref().and_then(Path::parent) {
            std::fs::create_dir_all(dir)?;
        }

        let mut tfrecord_writer: Option<Mutex<TfRecordWriter<File>>> = None;
        if let Some(file) = &outputs.tfrecord_file {
            tfrecord_writer = Some(Mutex::new(TfRecordWriter::new(File::create(file)?)));
        }

        // When `outputs.report_path` is set, build a `ReportCollector` and
        // add it to the observer chain (needs a DualObserver setup if
        // there's already a different observer)
        let report_collector = outputs
            .report_path
            .as_ref()
            .map(|_| Arc::new(ReportCollector::new()));
        let effective_observer: Option<Arc<dyn ExtractionObserver>> =
            match (&options.observer, &report_collector) {
                (Some(existing), Some(collector)) => {
                    Some(Arc::new(DualObserver(existing.clone(), collector.clone())))
                }
                (Some(existing), None) => Some(existing.clone()),
                (None, Some(collector)) => Some(collector.clone() as Arc<dyn ExtractionObserver>),
                (None, None) => None,
            };

        if let Some(observer) = &effective_observer {
            observer.on_extraction_start(level_idx, total_tiles);
        }

        let parallel = matches!(options.parallelism, Parallelism::Parallel(_));
        let pool = match options.parallelism {
            Parallelism::Parallel(Some(n)) => {
                Some(rayon::ThreadPoolBuilder::new().num_threads(n).build()?)
            }
            _ => None,
        };

        // A single, shared threshold derived from the lowest-resolution
        // level. Deriving a threshold per-tile instead would let Otsu
        // invent a bogus tissue/background split on tiles that are
        // actually uniform background.
        let tissue_overview = options
            .min_tissue_fraction
            .map(|_| install(pool.as_ref(), || self.tissue_overview(level_idx, parallel)))
            .transpose()?;
        let tissue_threshold = tissue_overview.as_ref().map(|overview| overview.threshold);

        let record_drop = |col: u32, row: u32, tissue_fraction: f32, min_fraction: f32| {
            dropped_tiles.fetch_add(1, Ordering::Relaxed);
            if let Some(observer) = &effective_observer {
                observer.on_tile_dropped(level_idx, col, row, tissue_fraction, min_fraction);
            }
        };

        // Drops background on the low-resolution overview, before anything
        // at `level_idx` is read. Kept tiles are still checked at full
        // resolution in `process`.
        let keep = |col: u32, row: u32| -> bool {
            let (Some(min_fraction), Some(overview)) =
                (options.min_tissue_fraction, &tissue_overview)
            else {
                return true;
            };
            let (x, y, width, height) = match &grid_params {
                Some(params) => (
                    col * params.stride_px,
                    row * params.stride_px,
                    params.window_px,
                    params.window_px,
                ),
                None => {
                    let tile_size = level.tile_size();
                    let (width, height) = level.valid_tile_dimensions(col, row);
                    (col * tile_size.width, row * tile_size.height, width, height)
                }
            };
            let tissue_fraction = overview.tissue_fraction(x, y, width, height);
            if tissue_fraction < min_fraction {
                record_drop(col, row, tissue_fraction, min_fraction);
                return false;
            }
            true
        };

        let process = |tile: Tile| -> Result<(), WsiError> {
            if let (Some(min_fraction), Some(threshold)) =
                (options.min_tissue_fraction, tissue_threshold)
            {
                let cropped: Cow<DynamicImage> = match options.geometry {
                    TileGeometry::Native => self.cropped_image(level_idx, &tile)?,
                    TileGeometry::Grid { .. } => Cow::Borrowed(tile.image()),
                };
                let tissue_mask = TissueMask::compute_with_threshold(&cropped, threshold);
                if !tissue_mask.has_more_than_min_tissue(min_fraction) {
                    record_drop(
                        tile.tile_x(),
                        tile.tile_y(),
                        tissue_mask.tissue_fraction(),
                        min_fraction,
                    );
                    return Ok(());
                }
            }
            let tile = if options.normalize_stain {
                tile.normalize_stain()
            } else {
                tile
            };

            // Resizing after tissue-filtering/normalization
            let tile = match (&grid_params, resize_factor) {
                (Some(params), _) if params.window_px != params.tile_px => {
                    tile.resize(params.tile_px, params.tile_px)
                }
                (None, Some(scale)) if (scale - 1.0).abs() > RESIZE_SCALE_EPSILON => {
                    let (width, height) = (tile.image().width(), tile.image().height());
                    let new_width = ((width as f64) * scale).round().max(1.0) as u32;
                    let new_height = ((height as f64) * scale).round().max(1.0) as u32;
                    tile.resize(new_width, new_height)
                }
                _ => tile,
            };

            // Encoded once, shared by both file outputs
            let jpeg = if outputs.tile_dir.is_some() || tfrecord_writer.is_some() {
                Some(tile.encode_jpeg_with_quality(options.jpeg_quality)?)
            } else {
                None
            };
            if let (Some(dir), Some(bytes)) = (&outputs.tile_dir, &jpeg) {
                std::fs::write(
                    dir.join(format!("{}_{}.jpg", tile.tile_x(), tile.tile_y())),
                    bytes,
                )?;
            }
            if let (Some(writer), Some(bytes)) = (&tfrecord_writer, jpeg) {
                let loc_x = (tile.origin_x() + footprint_w / 2) as i64;
                let loc_y = (tile.origin_y() + footprint_h / 2) as i64;

                let record = tfrecord::tile_record(&slide_name, bytes, loc_x, loc_y);
                writer
                    .lock()
                    .expect("Mutex lock failed!")
                    .write_record(&record)?;
            }
            if let Some(observer) = &effective_observer {
                observer.on_tile_extraction(level_idx, &tile);
            }
            f(tile)
        };

        let result = install(pool.as_ref(), || match &grid_params {
            Some(params) => {
                self.extract_grid(level_idx, params, cols, rows, parallel, keep, process)
            }
            None => {
                let coords = (0..rows).flat_map(|row| (0..cols).map(move |col| (col, row)));
                if parallel {
                    let coords: Vec<_> = coords.collect();
                    coords
                        .into_par_iter()
                        .filter(|&(x, y)| keep(x, y))
                        .try_for_each(|(x, y)| {
                            process(self.decode_tile_concurrent(level_idx, x, y)?)
                        })
                } else {
                    coords
                        .filter(|&(x, y)| keep(x, y))
                        .try_for_each(|(x, y)| process(self.decode_tile(level_idx, x, y)?))
                }
            }
        });

        if options.min_tissue_fraction.is_some()
            && let Some(observer) = &effective_observer
        {
            observer.on_extraction_complete(
                level_idx,
                ExtractionStats {
                    total: total_tiles,
                    dropped: dropped_tiles.load(Ordering::Relaxed),
                },
            );
        }

        // Only on success => produce a `.pdf` report
        if result.is_ok()
            && let (Some(path), Some(collector)) = (&outputs.report_path, &report_collector)
        {
            collector.report(options).write_pdf(self, path)?;
        }

        result
    }

    /// Stitches lowest-resolution pyramid level into a single image for use as a thumbnail.
    pub fn build_overview_image(&self) -> Result<DynamicImage, WsiError> {
        let overview = DynamicImage::ImageRgb8(self.lowest_level_image(false)?);
        let max_dim = overview.width().max(overview.height());

        Ok(if max_dim > report::OVERVIEW_MAX_PX {
            overview.thumbnail(report::OVERVIEW_MAX_PX, report::OVERVIEW_MAX_PX)
        } else {
            overview
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// See `tests/fixtures/README.md` for provenance.
    const FIXTURE: &str = "tests/fixtures/CMU-1-Small-Region.svs";

    fn open_fixture() -> Slide {
        Slide::open(FIXTURE).expect("fixture slide should open")
    }

    #[test]
    fn open_reads_real_slide_metadata() {
        let slide = open_fixture();
        assert_eq!(slide.level_count(), 1);

        let level = slide.metadata().level(0).unwrap();
        assert_eq!(level.dimensions().width, 2220);
        assert_eq!(level.dimensions().height, 2967);
    }

    #[test]
    fn open_fails_on_missing_file() {
        let result = Slide::open("tests/fixtures/does_not_exist.svs");
        assert!(result.is_err());
    }

    #[test]
    fn decode_tile_returns_expected_dimensions() {
        let slide = open_fixture();
        let tile = slide.decode_tile(0, 0, 0).unwrap();
        assert_eq!(tile.image().width(), 240);
        assert_eq!(tile.image().height(), 240);
    }

    #[test]
    fn decode_tile_computes_base_pixel_origin_from_grid_coords() {
        let slide = open_fixture();

        // The fixture has a single pyramid level (downsample_factor == 1.0),
        // so a tile's base-pixel origin is just its grid coordinate times
        // the native 240px tile size.
        let tile = slide.decode_tile(0, 0, 0).unwrap();
        assert_eq!((tile.origin_x(), tile.origin_y()), (0, 0));

        let tile = slide.decode_tile(0, 1, 0).unwrap();
        assert_eq!((tile.origin_x(), tile.origin_y()), (240, 0));

        let tile = slide.decode_tile(0, 2, 3).unwrap();
        assert_eq!((tile.origin_x(), tile.origin_y()), (480, 720));
    }

    #[test]
    fn window_count_handles_exact_multiples_remainders_and_oversized_windows() {
        assert_eq!(window_count(900, 300, 300), 3); // exact multiple
        assert_eq!(window_count(1000, 300, 300), 3); // remainder dropped, not padded
        assert_eq!(window_count(100, 300, 300), 0); // window larger than the level
        assert_eq!(window_count(1000, 300, 150), 5); // overlapping stride
    }

    #[test]
    fn whole_pixels_truncates_but_tolerates_float_error() {
        assert_eq!(whole_pixels(298.9), 298); // truncated like slideflow, not rounded
        assert_eq!(whole_pixels(298.0 * 0.499 / 0.499), 298);
        assert_eq!(whole_pixels(297.999_999_9), 298); // float error, not a real fraction
        assert_eq!(whole_pixels(0.4), 1); // never a zero-sized window
    }

    #[test]
    fn decode_tile_rejects_out_of_bounds_level() {
        let slide = open_fixture();
        let result = slide.decode_tile(5, 0, 0);
        assert!(matches!(result, Err(WsiError::LevelIndexOutOfBounds)));
    }

    #[test]
    fn decode_tile_rejects_out_of_bounds_tile_coord() {
        let slide = open_fixture();
        let level = slide.metadata().level(0).unwrap();
        let result = slide.decode_tile(0, level.tiles_x(), 0);
        assert!(matches!(result, Err(WsiError::TileIndexOutOfBounds)));
    }

    #[test]
    fn extract_sequential_visits_every_tile_exactly_once() {
        let slide = open_fixture();
        let expected: HashSet<_> = slide.tile_coords(0).unwrap().collect();

        let seen = Mutex::new(Vec::new());
        let options = ExtractionOptions::sequential().with_level(0);
        slide
            .extract(&options, &SlideOutputs::default(), |tile| {
                seen.lock().unwrap().push((tile.tile_x(), tile.tile_y()));
                Ok(())
            })
            .unwrap();

        let seen = seen.into_inner().unwrap();
        let seen_set: HashSet<_> = seen.iter().copied().collect();
        assert_eq!(seen.len(), expected.len(), "no duplicate or skipped tiles");
        assert_eq!(seen_set, expected);
    }

    #[test]
    fn extract_parallel_visits_every_tile_exactly_once() {
        let slide = open_fixture();
        let expected: HashSet<_> = slide.tile_coords(0).unwrap().collect();

        let seen = Mutex::new(Vec::new());
        let options = ExtractionOptions::parallel().with_level(0);
        slide
            .extract(&options, &SlideOutputs::default(), |tile| {
                seen.lock().unwrap().push((tile.tile_x(), tile.tile_y()));
                Ok(())
            })
            .unwrap();

        let seen = seen.into_inner().unwrap();
        let seen_set: HashSet<_> = seen.iter().copied().collect();
        assert_eq!(
            seen.len(),
            expected.len(),
            "no duplicate or skipped tiles under parallelism"
        );
        assert_eq!(seen_set, expected);
    }

    #[test]
    fn extract_with_tissue_filter_only_emits_tiles_above_the_threshold() {
        let slide = open_fixture();
        let total = slide.tile_coords(0).unwrap().count();

        let kept = Mutex::new(0usize);
        let options = ExtractionOptions::sequential()
            .with_level(0)
            .with_min_tissue_fraction(0.5);
        slide
            .extract(&options, &SlideOutputs::default(), |_tile| {
                *kept.lock().unwrap() += 1;
                Ok(())
            })
            .unwrap();

        let kept = kept.into_inner().unwrap();
        assert!(kept > 0, "expected at least some tissue-containing tiles");
        assert!(
            kept < total,
            "expected the filter to drop at least one tile"
        );
    }

    #[test]
    fn extract_with_stain_normalization_does_not_error() {
        let slide = open_fixture();
        let options = ExtractionOptions::sequential()
            .with_level(0)
            .with_stain_normalization();

        let result = slide.extract(&options, &SlideOutputs::default(), |_tile| Ok(()));
        assert!(result.is_ok());
    }

    #[test]
    fn extract_with_target_mpp_resizes_tiles_to_match() {
        let slide = open_fixture();
        let native_mpp = slide.metadata().microns_per_pixel().unwrap();
        let target_mpp = native_mpp * 2.0; // half resolution

        let sizes = Mutex::new(Vec::new());
        let options = ExtractionOptions::sequential().with_target_mpp(target_mpp);
        slide
            .extract(&options, &SlideOutputs::default(), |tile| {
                sizes.lock().unwrap().push(tile.image().width());
                Ok(())
            })
            .unwrap();

        let sizes = sizes.into_inner().unwrap();
        assert!(!sizes.is_empty());
        // Native tile width is 240px; doubling the target MPP halves it.
        assert!(sizes.iter().all(|&w| w == 120));
    }

    #[test]
    fn extract_writes_tile_dir_tfrecord_and_report_outputs() {
        let slide = open_fixture();
        let dir = tempfile::tempdir().unwrap();
        let tile_dir = dir.path().join("tiles");
        let tfrecord_path = dir.path().join("out.tfrecord");
        let report_path = dir.path().join("report.pdf");

        let options = ExtractionOptions::sequential().with_level(0);
        let outputs = SlideOutputs::default()
            .with_tile_dir(&tile_dir)
            .with_tfrecord_file(&tfrecord_path)
            .with_report_path(&report_path);

        slide.extract(&options, &outputs, |_tile| Ok(())).unwrap();

        let expected_tiles = slide.tile_coords(0).unwrap().count();
        let written_tiles = std::fs::read_dir(&tile_dir).unwrap().count();
        assert_eq!(written_tiles, expected_tiles);

        let tfrecord_meta = std::fs::metadata(&tfrecord_path).unwrap();
        assert!(tfrecord_meta.len() > 0);

        let report_bytes = std::fs::read(&report_path).unwrap();
        assert!(report_bytes.starts_with(b"%PDF"));
    }

    #[test]
    fn build_overview_image_matches_lowest_level_aspect_ratio() {
        let slide = open_fixture();
        let overview = slide.build_overview_image().unwrap();
        let level = slide.metadata().level(slide.level_count() - 1).unwrap();

        assert!(overview.width() > 0 && overview.height() > 0);

        let level_ratio = level.dimensions().width as f64 / level.dimensions().height as f64;
        let overview_ratio = overview.width() as f64 / overview.height() as f64;
        assert!((level_ratio - overview_ratio).abs() < 0.01);
    }

    #[test]
    fn read_region_handles_rectangle_entirely_within_one_tile() {
        let slide = open_fixture();
        let region = slide.read_region(0, 10, 10, 50, 50).unwrap().to_rgb8();
        let tile = slide.decode_tile(0, 0, 0).unwrap().image().to_rgb8();

        for py in 0..50u32 {
            for px in 0..50u32 {
                assert_eq!(
                    region.get_pixel(px, py),
                    tile.get_pixel(10 + px, 10 + py),
                    "mismatch at ({px}, {py})"
                );
            }
        }
    }

    #[test]
    fn read_region_stitches_a_2x2_block_of_native_tiles() {
        let slide = open_fixture();
        let region = slide.read_region(0, 200, 200, 100, 100).unwrap().to_rgb8();

        // Spans tiles (0,0), (1,0), (0,1), (1,1); spot-check one pixel
        // sourced from each quadrant of the stitched region.
        let expect = |region_x: u32, region_y: u32, tile_x: u32, tile_y: u32| {
            let tile = slide
                .decode_tile(0, tile_x, tile_y)
                .unwrap()
                .image()
                .to_rgb8();
            let local_x = (200 + region_x) - tile_x * 240;
            let local_y = (200 + region_y) - tile_y * 240;
            assert_eq!(
                region.get_pixel(region_x, region_y),
                tile.get_pixel(local_x, local_y)
            );
        };

        expect(0, 0, 0, 0); // top-left quadrant
        expect(99, 0, 1, 0); // top-right quadrant
        expect(0, 99, 0, 1); // bottom-left quadrant
        expect(99, 99, 1, 1); // bottom-right quadrant
    }

    #[test]
    fn read_region_pads_with_black_past_the_slide_edge() {
        let slide = open_fixture();
        // Level is 2220px wide; requesting x=2200..2300 runs 80px past
        // that edge (tile column 9's valid width is only 60px, ending at
        // x=2220, well within row 9's full height -- no Y clipping here).
        let region = slide
            .read_region(0, 2200, 2200, 100, 100)
            .unwrap()
            .to_rgb8();
        let tile = slide.decode_tile(0, 9, 9).unwrap().image().to_rgb8();

        // Columns 0..20 (level x 2200..2220) are real, in-bounds data.
        for py in 0..100u32 {
            for px in 0..20u32 {
                assert_eq!(region.get_pixel(px, py), tile.get_pixel(40 + px, 40 + py));
            }
        }

        // Columns 20..100 fall past the slide's real edge and stay black.
        for py in 0..100u32 {
            for px in 20..100u32 {
                assert_eq!(*region.get_pixel(px, py), image::Rgb([0, 0, 0]));
            }
        }
    }

    #[test]
    fn grid_geometry_matches_native_over_their_overlapping_tiles() {
        use std::collections::HashMap;

        let slide = open_fixture();
        let native_mpp = slide.metadata().microns_per_pixel().unwrap(); // 0.499
        let tile_um = 240.0 * native_mpp;

        let collect = |options: ExtractionOptions| -> HashMap<(u32, u32), Vec<u8>> {
            let map = Mutex::new(HashMap::new());
            slide
                .extract(&options, &SlideOutputs::default(), |tile| {
                    map.lock().unwrap().insert(
                        (tile.tile_x(), tile.tile_y()),
                        tile.image().to_rgb8().into_raw(),
                    );
                    Ok(())
                })
                .unwrap();
            map.into_inner().unwrap()
        };

        let native = collect(ExtractionOptions::sequential().with_level(0));
        let grid = collect(
            ExtractionOptions::sequential()
                .with_level(0)
                .with_grid_geometry(240, tile_um, None),
        );

        // Grid drops the padded boundary row/column Native keeps: a 9x12
        // window grid here, not Native's 10x13 -- see `window_count`.
        assert_eq!(grid.len(), 9 * 12);

        for (coord, grid_bytes) in &grid {
            let native_bytes = native
                .get(coord)
                .unwrap_or_else(|| panic!("native tile missing at {coord:?}"));
            assert_eq!(grid_bytes, native_bytes, "pixel mismatch at {coord:?}");
        }
    }

    #[test]
    fn grid_windows_match_read_region_with_overlapping_misaligned_stride() {
        use std::collections::HashMap;

        let slide = open_fixture();
        let mpp = slide.metadata().microns_per_pixel().unwrap();
        // 300px windows every 260px: neither lines up with the 240px native
        // tiles, and consecutive window rows share native rows
        let (window, stride) = (300u32, 260u32);

        for options in [
            ExtractionOptions::sequential(),
            ExtractionOptions::parallel(),
        ] {
            let options = options.with_level(0).with_grid_geometry(
                window,
                window as f64 * mpp,
                Some(stride as f64 * mpp),
            );
            let tiles = Mutex::new(HashMap::new());
            slide
                .extract(&options, &SlideOutputs::default(), |tile| {
                    tiles.lock().unwrap().insert(
                        (tile.tile_x(), tile.tile_y()),
                        tile.image().to_rgb8().into_raw(),
                    );
                    Ok(())
                })
                .unwrap();
            let tiles = tiles.into_inner().unwrap();

            assert_eq!(
                tiles.len(),
                (window_count(2220, window, stride) * window_count(2967, window, stride)) as usize
            );
            for (&(col, row), pixels) in &tiles {
                let expected = slide
                    .read_region(0, col * stride, row * stride, window, window)
                    .unwrap()
                    .to_rgb8()
                    .into_raw();
                assert!(*pixels == expected, "pixel mismatch at {:?}", (col, row));
            }
        }
    }

    #[test]
    fn grid_tiles_are_resized_to_tile_px() {
        let slide = open_fixture();
        let mpp = slide.metadata().microns_per_pixel().unwrap();

        let sizes = Mutex::new(Vec::new());
        let options = ExtractionOptions::parallel()
            .with_level(0)
            .with_grid_geometry(128, 298.0 * mpp, None);
        slide
            .extract(&options, &SlideOutputs::default(), |tile| {
                sizes
                    .lock()
                    .unwrap()
                    .push((tile.image().width(), tile.image().height()));
                Ok(())
            })
            .unwrap();

        let sizes = sizes.into_inner().unwrap();
        assert_eq!(sizes.len(), (7 * 9) as usize); // 2220 / 298, 2967 / 298
        assert!(sizes.iter().all(|&size| size == (128, 128)));
    }

    #[test]
    fn blit_copies_only_the_overlap() {
        let tile = RgbImage::from_fn(4, 4, |x, y| image::Rgb([x as u8, y as u8, 7]));
        let mut canvas = RgbImage::new(4, 4);
        // Canvas covers (10..14, 10..14), tile covers (12..16, 9..13)
        blit(&mut canvas, (10, 10), &tile, (12, 9));

        for (x, y, pixel) in canvas.enumerate_pixels() {
            let expected = if x >= 2 && y <= 2 {
                image::Rgb([(x - 2) as u8, (y + 1) as u8, 7])
            } else {
                image::Rgb([0, 0, 0])
            };
            assert_eq!(*pixel, expected, "at ({x}, {y})");
        }
    }

    #[test]
    fn tissue_overview_measures_the_covered_overview_pixels() {
        // Left half tissue (dark), right half background, at 1/10th of the
        // extraction level's resolution
        let gray = GrayImage::from_fn(10, 10, |x, _| image::Luma([if x < 5 { 50 } else { 230 }]));
        let overview = TissueOverview {
            gray,
            threshold: 128,
            scale_x: 0.1,
            scale_y: 0.1,
        };

        assert_eq!(overview.tissue_fraction(0, 0, 50, 100), 1.0);
        assert_eq!(overview.tissue_fraction(50, 0, 50, 100), 0.0);
        assert_eq!(overview.tissue_fraction(0, 0, 100, 100), 0.5);
        // Partially covered overview pixels count fully: 45..55 spans x=4..6
        assert_eq!(overview.tissue_fraction(45, 0, 10, 10), 0.5);
        // A sub-pixel region still samples one pixel; past the edge clamps
        assert_eq!(overview.tissue_fraction(1, 1, 1, 1), 1.0);
        assert_eq!(overview.tissue_fraction(500, 500, 10, 10), 0.0);
    }

    #[test]
    fn tissue_overview_matches_the_lowest_level() {
        let slide = open_fixture();
        let overview = slide.tissue_overview(0, true).unwrap();
        let sequential = slide.tissue_overview(0, false).unwrap();

        // Single-level fixture: the overview *is* level 0
        assert_eq!(overview.gray.dimensions(), (2220, 2967));
        assert_eq!((overview.scale_x, overview.scale_y), (1.0, 1.0));
        assert_eq!(overview.gray, sequential.gray);
        assert_eq!(overview.threshold, sequential.threshold);
    }
}
