//! Python bindings for the `slideforge` whole-slide image tile extraction
//! library.
//!
//! The extension module is intentionally named `slideforge` -- the same
//! name as the Rust dependency it wraps -- so `import slideforge` reads
//! naturally from Python. To avoid any ambiguity between "this crate" and
//! the `slideforge` dependency, every reference to the wrapped library
//! below is written as `::slideforge::...` (crate-root-qualified), and the
//! `#[pymodule]` function itself is named `py_slideforge` with an explicit
//! `name = "slideforge"` override.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use numpy::{IntoPyArray, PyArray3, ndarray::Array3};
use pyo3::exceptions::{
    PyFileNotFoundError, PyIOError, PyIndexError, PyRuntimeError, PyValueError,
};
use pyo3::prelude::*;

use ::slideforge::error::WsiError;
use ::slideforge::slide::{Slide, SlideOutputs};
use ::slideforge::{ExtractionObserver, ExtractionOptions};

/// `(width, height, downsample_factor)` per pyramid level.
type LevelInfo = (u32, u32, f64);
/// `(microns_per_pixel, objective_power, levels)`.
type SlideMetadata = (Option<f64>, Option<f32>, Vec<LevelInfo>);
/// `(tile_x, tile_y, width, height, rgb_bytes)`, before conversion to a
/// `PyArray` (which requires the GIL, unlike collecting the raw bytes).
type RawTile = (u32, u32, u32, u32, Vec<u8>);
/// `(tile_x, tile_y, ndarray)`.
type TileArray<'py> = (u32, u32, Bound<'py, PyArray3<u8>>);

/// Maps a [`WsiError`] onto the closest matching built-in Python exception,
/// so callers can `except FileNotFoundError` / `except ValueError` etc.
/// rather than catching one opaque exception type for everything.
fn to_pyerr(err: WsiError) -> PyErr {
    match err {
        WsiError::Io(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                PyFileNotFoundError::new_err(e.to_string())
            } else {
                PyIOError::new_err(e.to_string())
            }
        }
        WsiError::Tiff(e) => PyValueError::new_err(format!("invalid TIFF/SVS file: {e}")),
        WsiError::UnsupportedFormat => PyValueError::new_err("unsupported file format"),
        WsiError::InvalidMetadata => PyValueError::new_err("invalid or missing slide metadata"),
        WsiError::TileIndexOutOfBounds => PyIndexError::new_err("tile index out of bounds"),
        WsiError::LevelIndexOutOfBounds => PyIndexError::new_err("level index out of bounds"),
        WsiError::ThreadPool(e) => PyRuntimeError::new_err(e.to_string()),
        WsiError::Report => PyRuntimeError::new_err("failed to generate extraction report"),
    }
}

/// Tracks extraction totals via the public [`ExtractionObserver`] hook,
/// since [`slideforge::logging::ReportCollector`]'s own counters are
/// crate-private.
#[derive(Default)]
struct CountingObserver {
    total: AtomicUsize,
    dropped: AtomicUsize,
}

impl ExtractionObserver for CountingObserver {
    fn on_extraction_start(&self, _level_idx: usize, total_tiles: usize) {
        self.total.store(total_tiles, Ordering::Relaxed);
    }

    fn on_tile_dropped(
        &self,
        _level_idx: usize,
        _tile_x: u32,
        _tile_y: u32,
        _tissue_fraction: f32,
        _min_fraction: f32,
    ) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

impl CountingObserver {
    fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    fn kept(&self) -> usize {
        self.total() - self.dropped.load(Ordering::Relaxed)
    }
}

/// Builds [`ExtractionOptions`] from the keyword arguments shared by
/// `extract` and `extract_arrays`. Exactly one of `level`/`target_mpp` must
/// be given.
fn build_options(
    level: Option<usize>,
    target_mpp: Option<f64>,
    min_tissue_fraction: Option<f32>,
    stain_normalize: bool,
    threads: Option<usize>,
) -> PyResult<ExtractionOptions> {
    let mut options = match (level, target_mpp) {
        (Some(_), Some(_)) => {
            return Err(PyValueError::new_err(
                "specify either level or target_mpp, not both",
            ));
        }
        (None, None) => {
            return Err(PyValueError::new_err("specify one of level or target_mpp"));
        }
        (Some(level), None) => match threads {
            Some(n) => ExtractionOptions::parallel_with_threads(n).with_level(level),
            None => ExtractionOptions::parallel().with_level(level),
        },
        (None, Some(target_mpp)) => match threads {
            Some(n) => ExtractionOptions::parallel_with_threads(n).with_target_mpp(target_mpp),
            None => ExtractionOptions::parallel().with_target_mpp(target_mpp),
        },
    };

    if let Some(min_fraction) = min_tissue_fraction {
        options = options.with_min_tissue_fraction(min_fraction);
    }
    if stain_normalize {
        options = options.with_stain_normalization();
    }

    Ok(options)
}

/// Extracts tiles from a slide, writing them as loose JPEGs and/or a
/// `.tfrecord` file. Returns `(kept, total)` tile counts.
#[pyfunction]
#[pyo3(signature = (
    path,
    tfrecord=None,
    tile_dir=None,
    *,
    level=None,
    target_mpp=None,
    min_tissue_fraction=None,
    stain_normalize=false,
    threads=None,
))]
#[allow(clippy::too_many_arguments)]
fn extract(
    py: Python<'_>,
    path: PathBuf,
    tfrecord: Option<PathBuf>,
    tile_dir: Option<PathBuf>,
    level: Option<usize>,
    target_mpp: Option<f64>,
    min_tissue_fraction: Option<f32>,
    stain_normalize: bool,
    threads: Option<usize>,
) -> PyResult<(usize, usize)> {
    py.detach(move || {
        let slide = Slide::open(&path).map_err(to_pyerr)?;
        let mut options = build_options(
            level,
            target_mpp,
            min_tissue_fraction,
            stain_normalize,
            threads,
        )?;

        let counter = Arc::new(CountingObserver::default());
        options = options.with_shared_observer(counter.clone() as Arc<dyn ExtractionObserver>);

        let mut outputs = SlideOutputs::default();
        if let Some(dir) = tile_dir {
            outputs = outputs.with_tile_dir(dir);
        }
        if let Some(path) = tfrecord {
            outputs = outputs.with_tfrecord_file(path);
        }

        slide
            .extract(&options, &outputs, |_tile| Ok(()))
            .map_err(to_pyerr)?;

        Ok((counter.kept(), counter.total()))
    })
}

/// Extracts tiles from a slide directly into memory as `(tile_x, tile_y,
/// ndarray)` triples, with each array shaped `(height, width, 3)` `uint8`.
///
/// Intended for small-scale/interactive use -- for anything at
/// gigapixel-slide scale, prefer [`extract`]'s `.tfrecord` path, which
/// never collects more than one tile's worth of pixels in memory at once.
#[pyfunction]
#[pyo3(signature = (
    path,
    *,
    level=None,
    target_mpp=None,
    min_tissue_fraction=None,
    stain_normalize=false,
))]
fn extract_arrays<'py>(
    py: Python<'py>,
    path: PathBuf,
    level: Option<usize>,
    target_mpp: Option<f64>,
    min_tissue_fraction: Option<f32>,
    stain_normalize: bool,
) -> PyResult<Vec<TileArray<'py>>> {
    // Collect raw pixel buffers with the GIL released -- constructing
    // `PyArray`s requires the GIL, so that happens afterwards instead.
    let raw = py.detach(move || -> PyResult<Vec<RawTile>> {
        let slide = Slide::open(&path).map_err(to_pyerr)?;
        let options = build_options(
            level,
            target_mpp,
            min_tissue_fraction,
            stain_normalize,
            None,
        )?;

        let collected = Mutex::new(Vec::new());
        slide
            .extract(&options, &SlideOutputs::default(), |tile| {
                let rgb = tile.image().to_rgb8();
                let (width, height) = rgb.dimensions();
                collected.lock().expect("mutex poisoned").push((
                    tile.tile_x(),
                    tile.tile_y(),
                    width,
                    height,
                    rgb.into_raw(),
                ));
                Ok(())
            })
            .map_err(to_pyerr)?;

        Ok(collected.into_inner().expect("mutex poisoned"))
    })?;

    raw.into_iter()
        .map(|(x, y, width, height, buf)| {
            let array = Array3::from_shape_vec((height as usize, width as usize, 3), buf)
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok((x, y, array.into_pyarray(py)))
        })
        .collect()
}

/// Reads a slide's metadata without extracting any tiles. Returns
/// `(microns_per_pixel, objective_power, levels)`, where `levels` is a list
/// of `(width, height, downsample_factor)` tuples ordered from
/// highest to lowest resolution.
#[pyfunction]
fn metadata(path: PathBuf) -> PyResult<SlideMetadata> {
    let slide = Slide::open(&path).map_err(to_pyerr)?;
    let meta = slide.metadata();

    let levels = meta
        .levels()
        .iter()
        .map(|level| {
            let dims = level.dimensions();
            (dims.width, dims.height, level.downsample_factor())
        })
        .collect();

    Ok((meta.microns_per_pixel(), meta.objective_power(), levels))
}

#[pymodule(name = "slideforge")]
fn py_slideforge(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(extract, m)?)?;
    m.add_function(wrap_pyfunction!(extract_arrays, m)?)?;
    m.add_function(wrap_pyfunction!(metadata, m)?)?;
    Ok(())
}
