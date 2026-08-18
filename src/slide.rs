use crate::backend::svs::parse_slide;
use crate::decoder::JpegDecoder;
use crate::error::WsiError;
use crate::extraction::{ExtractionLevel, ExtractionOptions, Parallelism};
use crate::filter::{TissueMask, grayscale_histogram, otsu_threshold_from_histogram};
use crate::logging::{DualObserver, ExtractionObserver, ExtractionStats, ReportCollector};
use crate::metadata::Metadata;
use crate::tfrecord::TfRecordWriter;
use crate::tile::{Tile, TileDirectory, read_tile_bytes, read_tile_bytes_at};
use crate::{report, tfrecord};
use image::{DynamicImage, RgbImage};
/// Representation of a single Whole Slide Image (WSI) along with its metadata.
use rayon::prelude::*;
use std::borrow::Cow;
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

    /// Computes a single Otsu threshold for tissue/background separation
    /// from the aggregated grayscale histogram of every tile at the
    /// lowest-resolution pyramid level.
    ///
    /// Tiles are classified against this shared reference point instead of
    /// each deriving its own threshold in isolation, which would let Otsu
    /// invent a spurious tissue/background split on a tile that is
    /// actually uniform background (or uniform tissue).
    fn global_tissue_threshold(&self) -> Result<u8, WsiError> {
        let level_idx = self.level_count() - 1;
        let mut histogram = [0u32; 256];

        for (x, y) in self.tile_coords(level_idx)? {
            let tile = self.decode_tile(level_idx, x, y)?;
            let cropped = self.cropped_image(level_idx, &tile)?;
            for (bin, count) in grayscale_histogram(&cropped.to_luma8()).iter().enumerate() {
                histogram[bin] += count;
            }
        }

        Ok(otsu_threshold_from_histogram(&histogram))
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
            None => return Err(WsiError::InvalidMetadata),
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

        // Computing the resize scale so the chosen level matches exactly the target resolution
        let resize_factor = match &options.extraction_level {
            Some(ExtractionLevel::TargetMpp(target)) => {
                let base_mpp = self
                    .metadata
                    .microns_per_pixel()
                    .ok_or(WsiError::InvalidMetadata)?;
                let level = self
                    .metadata
                    .level(level_idx)
                    .ok_or(WsiError::LevelIndexOutOfBounds)?;
                let level_mpp = base_mpp * level.downsample_factor();
                Some(level_mpp / target)
            }
            _ => None,
        };

        let coords: Vec<(u32, u32)> = self.tile_coords(level_idx)?.collect();
        let total_tiles = coords.len();
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
        let report_collector = match &outputs.report_path {
            Some(_) => Some(Arc::new(ReportCollector::new())),
            None => None,
        };
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

        // A single, shared threshold derived from the lowest-resolution
        // level. Deriving a threshold per-tile instead would let Otsu
        // invent a bogus tissue/background split on tiles that are
        // actually uniform background.
        let tissue_threshold = options
            .min_tissue_fraction
            .map(|_| self.global_tissue_threshold())
            .transpose()?;

        let process = |tile: Tile| -> Result<(), WsiError> {
            if let (Some(min_fraction), Some(threshold)) =
                (options.min_tissue_fraction, tissue_threshold)
            {
                let cropped = self.cropped_image(level_idx, &tile)?;
                let tissue_mask = TissueMask::compute_with_threshold(&cropped, threshold);
                if !tissue_mask.has_more_than_min_tissue(min_fraction) {
                    dropped_tiles.fetch_add(1, Ordering::Relaxed);
                    if let Some(observer) = &effective_observer {
                        observer.on_tile_dropped(
                            level_idx,
                            tile.tile_x(),
                            tile.tile_y(),
                            tissue_mask.tissue_fraction(),
                            min_fraction,
                        );
                    }
                    return Ok(());
                }
            }
            let tile = if options.normalize_stain {
                tile.normalize_stain()
            } else {
                tile
            };

            // Resizing after tissue-filtering/normalization
            let tile = if let Some(scale) = resize_factor {
                if (scale - 1.0).abs() > RESIZE_SCALE_EPSILON {
                    let (width, height) = (tile.image().width(), tile.image().height());
                    let new_width = ((width as f64) * scale).round().max(1.0) as u32;
                    let new_height = ((height as f64) * scale).round().max(1.0) as u32;
                    tile.resize(new_width, new_height)
                } else {
                    tile
                }
            } else {
                tile
            };

            if let Some(dir) = &outputs.tile_dir {
                tile.save(dir.join(format!("{}_{}.jpg", tile.tile_x(), tile.tile_y())))?;
            }
            if let Some(writer) = &tfrecord_writer {
                let bytes = tile.encode_jpeg()?;
                let loc_x = tile.tile_x() as i64;
                let loc_y = tile.tile_y() as i64;
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

        let result = match options.parallelism {
            Parallelism::Sequential => coords
                .into_iter()
                .try_for_each(|(x, y)| process(self.decode_tile(level_idx, x, y)?)),
            Parallelism::Parallel(threads) => {
                let run = || {
                    coords.into_par_iter().try_for_each(|(x, y)| {
                        process(self.decode_tile_concurrent(level_idx, x, y)?)
                    })
                };

                match threads {
                    Some(n) => rayon::ThreadPoolBuilder::new()
                        .num_threads(n)
                        .build()?
                        .install(run),
                    None => run(),
                }
            }
        };

        if options.min_tissue_fraction.is_some() {
            if let Some(observer) = &effective_observer {
                observer.on_extraction_complete(
                    level_idx,
                    ExtractionStats {
                        total: total_tiles,
                        dropped: dropped_tiles.load(Ordering::Relaxed),
                    },
                );
            }
        }

        // Only on success => produce a `.pdf` report
        if result.is_ok() {
            if let (Some(path), Some(collector)) = (&outputs.report_path, &report_collector) {
                collector.report(options).write_pdf(self, path)?;
            }
        }

        result
    }

    /// Stitches lowest-resolution pyramid level into a single image for use as a thumbnail.
    pub(crate) fn build_overview_image(&self) -> Result<DynamicImage, WsiError> {
        let level_idx = self.level_count() - 1;
        let level = self
            .metadata
            .level(level_idx)
            .ok_or(WsiError::LevelIndexOutOfBounds)?;

        let dimensions = level.dimensions();
        let mut canvas = RgbImage::new(dimensions.width, dimensions.height);

        for (x, y) in self.tile_coords(level_idx)? {
            let tile = self.decode_tile(level_idx, x, y)?;
            let cropped = self.cropped_image(level_idx, &tile)?;
            let cropped_rgb = cropped.to_rgb8();

            let origin_x = x * level.tile_size().width;
            let origin_y = y * level.tile_size().height;

            for (px, py, pixel) in cropped_rgb.enumerate_pixels() {
                canvas.put_pixel(origin_x + px, origin_y + py, *pixel);
            }
        }

        let overview = DynamicImage::ImageRgb8(canvas);
        let max_dim = overview.width().max(overview.height());

        Ok(if max_dim > report::OVERVIEW_MAX_PX {
            overview.thumbnail(report::OVERVIEW_MAX_PX, report::OVERVIEW_MAX_PX)
        } else {
            overview
        })
    }
}
